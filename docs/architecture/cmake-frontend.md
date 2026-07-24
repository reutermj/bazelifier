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

**Open question:** how much weight to put on parsing `CMakeLists.txt`
directly (understanding CMake-the-language: variables, functions,
`if()`/generator expressions) versus leaning on CMake's own generated
artifacts (File API, Ninja graph) as the primary source of truth, with
`CMakeLists.txt` parsing used mainly to recover intent (target names,
visibility, structure) that the generated output doesn't preserve well.
This is a foundational decision and should be settled with real fixtures
before too much translator code depends on one direction.

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
