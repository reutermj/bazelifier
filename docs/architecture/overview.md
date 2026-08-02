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

The agent is **inside** the loop, not a fallback beside it. What this
pipeline validates is that a deterministic translator plus an agent can
convert a project — so an unresolved gap is an unfinished run, and green
is the only passing state. Judgement calls are expected at several points;
the equivalence checks, not reproducibility of the process, are the
contract. Note this makes the pipeline deliberately non-hermetic, which is
an accepted modelling choice rather than a defect to design out.

Source build files are never edited to make a conversion succeed — see
[build-verification.md](build-verification.md#the-input-build-files-are-immutable).

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
  know otherwise — the items, the recipes in `resolutions/`, and the
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
