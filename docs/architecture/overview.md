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
 │ Deterministic        │  Rust. Parses the source build description into
 │ translator            │  an internal model, mechanically emits Bazel
 │                       │  BUILD files for recognized patterns.
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
 │ Generated Bazel       │  BUILD files + any needed toolchain/rule glue.
 │ BUILD files           │
 └──────────┬────────────┘
            │
            ▼
 ┌─────────────────────┐
 │ Build + test          │  `bazel build` / `bazel test` against the
 │ verification          │  generated targets. Success = builds AND the
 │                       │  project's existing tests pass under Bazel.
 └───────────────────────┘
```

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

1. The generated Bazel targets build successfully.
2. The project's existing test suite passes when run under Bazel (`bazel
   test`), giving behavioral parity confidence — not just "it compiles."

**Open question:** how do we define "existing test suite" precisely across
different CMake projects (CTest, GoogleTest, ad hoc scripts, etc.)? Likely
needs its own doc once we have real fixtures to reason about.
