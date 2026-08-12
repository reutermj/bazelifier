# System overview

## Goal

Convert a project's existing build scripts into Bazel `BUILD` files. Two
frontends exist: **CMake** (the first, and the more developed — see
[cmake-frontend.md](cmake-frontend.md)) and **Autotools** (see
[autotools-frontend.md](autotools-frontend.md)). Others (Make, Meson, ...)
remain possible later.

Nothing in the pipeline design should be build-system-specific where it can
reasonably be generalized, but we are not paying an abstraction tax up front
for build systems we have not implemented. That the boundary held is now
demonstrated rather than assumed: the Autotools frontend renders through
codegen with no change to it and no new `model::Target` field.

## Pipeline

```
source build system files (e.g. CMakeLists.txt)
        │
        ▼
 ┌─────────────────────┐
 │ Deterministic        │  Rust. Discovers targets via the CMake File API,
 │ translator            │  also runs the real build to capture ground-truth
 │                       │  artifacts. Mechanically emits a STANDALONE Bazel
 │                       │  module for recognized patterns — its own
 │                       │  MODULE.bazel + BUILD.bazel, not a package inside
 │                       │  bazelifier's own workspace. See
 │                       │  docs/architecture/build-verification.md for why
 │                       │  that distinction is the whole point.
 └──────────┬────────────┘
            │ construct not understood
            ▼
 ┌─────────────────────┐
 │ needs_attention/      │  Structured markdown (see
 │ item (gap description)│  docs/architecture/needs-attention-interface.md)
 │                       │  describing what wasn't understood and what's
 │                       │  needed, written into the module's own output.
 └──────────┬────────────┘
            │ agent (e.g. Claude Code) resolves the gap
            ▼
 ┌─────────────────────┐
 │ Standalone Bazel      │  MODULE.bazel + BUILD.bazel + copied sources +
 │ module (per project)  │  ground_truth/ (real cmake+ninja-built
 │                       │  artifacts, for validation only — never part of
 │                       │  the user-facing output).
 └──────────┬────────────┘
            │ (for our own fixtures) packaged into a validation tarball
            │ alongside every other converted fixture, with a root
            │ MODULE.bazel depending on all of them
            ▼
 ┌─────────────────────┐
 │ Build + equivalence   │  Unpacked completely outside this repo. `bazel
 │ verification          │  build`/`bazel test` from the unpacked root
 │                       │  proves the module is independent (no reference
 │                       │  back to bazelifier) AND functionally equivalent
 │                       │  to the CMake build (runtime output comparison
 │                       │  today; more checks planned).
 └──────────┬────────────┘
            │ unresolved needs_attention/ items → agent resolves them in
            │ the generated BUILD.bazel and the build is re-run
            └──────────────► (loop back to verification, until green)
```

### The two stages are different in kind, on purpose

This is the project's central design decision, so it is worth stating as a
claim rather than a caveat.

A build translation splits into two unlike problems. One is **mechanical**:
which sources a library compiles, what its include paths are, which targets
link which. A program can get that exactly right from what the build system
itself reports. The other is **judgement about a particular project**: what a
`UTILITY` target is for, whether `JSON_C_HAVE_STDINT_H` means what
`HAVE_STDINT_H` means, whether a test's working directory is load-bearing.
That has no answer derivable from the inputs, and a translator that guesses
at it is confidently wrong rather than usefully approximate.

**Stage one is deterministic and refuses to guess.** Same commit, same
project, byte-identical output. Where confidence runs out it escalates
rather than falling back to a heuristic — an escalation is the translator
being honest about the edge of what it can know, and every one it emits is a
place a heuristic *could* have been added and deliberately was not.

**Stage two is an agent and is not reproducible.** Two runs may resolve one
item differently and both be right. That is the shape of the problem, not a
defect to engineer out; a pipeline restricted to the deterministic half
would convert almost nothing real.

**Validation, not reproducibility, is the contract.** A conversion must
build with no reference back to this repo, behave identically to the
original, and pass its own tests. Those checks are objective and are what
the project iterates against. The process may vary; the result may not.

Two things follow, both of which look like defects until you have read the
above:

- The pipeline is **deliberately non-hermetic**. An accepted modelling
  choice, not something to design away.
- Resolutions are **ephemeral**. They are not cached and replayed, because
  replaying one would make a re-conversion look green without the agent
  stage having engaged with what changed — which is exactly what is being
  tested. See bzl-b9b.

The agent is therefore **inside** the loop, not a fallback beside it: what
is validated is "translator + agent", so an unresolved gap is an unfinished
run and green is the only passing state.

Source build files are never edited to make a conversion succeed — see
[build-verification.md](build-verification.md#the-input-build-files-are-immutable).

## Replicate the build's behaviour, not this host's outcome

A conversion reproduces **what the project's build system does**, not what
happens to matter on the machine that ran the conversion. Those differ more
often than they look like they do, and the difference is always invisible
here and visible somewhere else.

The config header states the general case in miniature — see
[configure-file-and-toolchain-probes.md](configure-file-and-toolchain-probes.md#decision-bazel-native-not-host-capture)
on why the resolved `config.h` sitting in the build directory is *not*
copied into the module. But the rule is not about config headers. It applies
to any construct whose effect this host makes look unnecessary:

- A **gnulib replacement header** is inert on glibc/Linux — measured on
  libidn2, every `gl/*.h` can be deleted and the library still builds with
  byte-identical output. It is not inert on musl, macOS or Windows, which is
  the entire reason the project vendors it.
- A **compile-time conditional** whose branch this platform never takes.
- A **feature probe** that happens to succeed here.

In each case "we could skip it" means "we could bake in this machine's
answer", which is the thing the whole pipeline is built to avoid. Skipping
is not a smaller conversion, it is a conversion that is silently wrong for
the next platform — and the failure lands on whoever tries that platform,
far from anyone who could connect it to the decision.

That does not make skipping always wrong. It makes it a **decision to record
in the conversion**, with what was dropped and why, rather than an
optimisation to apply quietly. The corollary is that "this is inert here" is
never sufficient evidence on its own: the question is whether the construct
is inert *by design* or merely on this platform.

## Components

- **Translator (Rust):** owns parsing the source build system and codegen
  of Bazel files. See [cmake-frontend.md](cmake-frontend.md) and
  [bazel-codegen.md](bazel-codegen.md).
- **`needs_attention/` interface:** the contract for escalating unhandled
  constructs to an agent. See
  [needs-attention-interface.md](needs-attention-interface.md).
- **Agent stage:** resolving those items in the GENERATED output until the
  module builds and its comparisons pass. Driven by
  `.claude/skills/resolve-escalations/`; `tools/sweep/sweep.py --post-agent`
  sets the run up and measures it, and deliberately does not resolve
  anything itself. The module carries what an agent needs that it could not
  know otherwise — the items, any `project_notes/` for that project, and the
  constraints a resolution must not break — so a module lifted out of the
  corpus is still resolvable.
- **Verification:** how we confirm a conversion is actually correct, and the
  plan to make that hermetic. See [build-verification.md](build-verification.md).

## Success criteria for a conversion

A conversion is successful when:

1. The output is a **standalone Bazel module** that builds with no
   reference back to bazelifier's own workspace — see
   [build-verification.md](build-verification.md) for why this is checked
   explicitly (by unpacking a packaged tarball outside this repo) rather
   than assumed.
2. The generated module is **functionally equivalent** to the original
   CMake project. We are not targeting binary compatibility — see
   [build-verification.md](build-verification.md) for the specific
   equivalence checks: runtime output comparison, and the project's own
   CTest-registered tests run against the Bazel-built binary (its declared
   `PASS_REGULAR_EXPRESSION` at its `WORKING_DIRECTORY`) where it has them.
   Compile-command and symbol-table comparison are still planned, deferred
   until a fixture exercises them meaningfully.

**Partially settled:** "the project's existing tests" now means, concretely,
its CTest-registered `add_test` tests, read from `ctest
--show-only=json-v1` and translated per test (see
[cmake-frontend.md](cmake-frontend.md) and
[build-verification.md](build-verification.md)). tinyxml2's `xmltest` is the
first real case. What remains open is the long tail — GoogleTest sharding,
CTest test fixtures, `WILL_FAIL`, multi-config, and projects whose tests run
via ad hoc custom targets rather than `add_test` — to be worked out as
fixtures exercise them.
