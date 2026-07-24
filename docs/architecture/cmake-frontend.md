# CMake frontend

Covers how bazelifier reads and understands CMake input.

## Scope

The initial target is CMake projects that generate Ninja build files
(`cmake -G Ninja`). Parsing/understanding can draw on:

- `CMakeLists.txt` / `.cmake` sources directly (textual/AST-level
  understanding of CMake's own language), and/or
- CMake's own generated output (e.g. the Ninja build graph, `compile_commands.json`,
  CMake File API query results) as a source of ground truth for what
  actually gets built.

**Decision:** the frontend uses the CMake File API as its primary source of
truth, currently two query kinds (`translator/src/cmake_api.rs`):

- **`codemodel-v2`** — configures the project (`cmake -B <dir> -G Ninja`)
  and reads the reply for each target's name, type, sources, and build
  artifacts (the built binary's path), plus the top-level project's name.
  All already resolved by CMake itself (generator expressions evaluated,
  `if()`/variables/`find_package()` already accounted for) — this avoids
  re-implementing CMake-the-language, at the cost of requiring a real
  `cmake` invocation in the pipeline (not yet hermetic on the CMake side —
  see [build-verification.md](build-verification.md)) and tying
  translation to a given CMake version's File API schema.
- **`cache-v2`** — read for `CMAKE_PROJECT_VERSION`, when the top-level
  `project()` call specified a `VERSION`. Used for the generated
  `MODULE.bazel`'s own `version` (omitted when absent — Bazel's `module()`
  doesn't require one).

After configuring, the frontend also runs the actual build (`cmake
--build`) to produce ground-truth artifacts for validation — see
[build-verification.md](build-verification.md). This is not a third File
API query; it's a real build, reusing the same configured `build_dir`.

`compile_commands.json` is planned but not yet requested/read — see the
compile-command comparison item in
[build-verification.md](build-verification.md#equivalence-checks). It is
not intended as a second parsing path, only as a build-verification
cross-check once a fixture has interesting flags to compare.

Direct `CMakeLists.txt` parsing is not used for correctness, but may be
revisited later purely to recover source-level intent the File API
discards (comments, variable names, original target grouping/ordering) for
more idiomatic codegen. Not needed for the fixtures so far.

## What the frontend needs to produce

An internal model of the build graph that the Bazel codegen stage can
consume, roughly: targets, their type (library/binary/etc.), sources,
include paths, dependencies (internal + external), compile
definitions/options, and linker options. Exact shape TBD — update this doc
once the internal model stabilizes.

## Known hard cases (expect to escalate via runbook)

- Generator expressions (`$<...>`) with complex conditions.
- `execute_process()` / custom commands that shell out arbitrarily.
- `find_package()` resolving to system or vendored libraries with no
  obvious Bazel equivalent.
- Conditional logic driven by `CMakeCache.txt` values or platform detection
  that doesn't map cleanly to Bazel `select()`.

This list will grow as real fixtures surface real cases — treat it as a
living list, not a spec.
