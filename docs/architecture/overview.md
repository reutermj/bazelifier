# System overview

## Goal

Convert a project's existing build scripts into Bazel `BUILD` files. Initial
scope is **CMake** projects (typically CMake configured to generate Ninja
build files); the overall project intends to support additional build
systems (Make, Autotools, Meson, ...) later. Nothing in the pipeline design
should be CMake-specific where it can reasonably be generalized, but we are
not paying an abstraction tax up front for build systems we haven't
implemented yet.

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
 │ Runbook               │  Structured markdown (see docs/runbooks/)
 │ (gap description)     │  describing what wasn't understood and what's
 │                       │  needed.
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
[build-verification.md](build-verification.md#the-input-cmake-is-immutable).

## Components

- **Translator (Rust):** owns parsing the source build system and codegen
  of Bazel files. See [cmake-frontend.md](cmake-frontend.md) and
  [bazel-codegen.md](bazel-codegen.md).
- **Runbook interface:** the contract for escalating unhandled constructs to
  an agent. See [runbook-interface.md](runbook-interface.md).
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
   equivalence checks (runtime output comparison today; compile-command
   and symbol comparison, and running the project's own CTest/GoogleTest
   suite, planned as fixtures grow to exercise them).

**Open question:** how do we define "existing test suite" precisely across
different CMake projects (CTest, GoogleTest, ad hoc scripts, etc.)? Likely
needs its own doc once we have real fixtures to reason about.
