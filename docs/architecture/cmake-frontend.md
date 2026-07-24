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

**Decision:** the frontend uses the CMake File API (`codemodel-v2`) as its
primary source of truth. The translator configures the project (`cmake -B
<dir> -G Ninja -DCMAKE_EXPORT_COMPILE_COMMANDS=ON`, requesting the
`codemodel-v2` File API query) and reads the resulting reply JSON for
targets, their type, sources, include paths, compile definitions, and link
dependencies — all already resolved by CMake itself (generator expressions
evaluated, `if()`/variables/`find_package()` already accounted for). This
avoids re-implementing CMake-the-language, at the cost of requiring a real
`cmake` invocation in the pipeline (not yet hermetic on the CMake side —
see [build-verification.md](build-verification.md)) and tying translation
to a given CMake version's File API schema.

`compile_commands.json` (emitted alongside the File API reply) is *not* a
second parsing path — it's reserved for build-verification: cross-checking
that the flags Bazel actually compiles with match what CMake/Ninja would
have used, catching drift the File API's more abstract "compile groups"
could mask.

Direct `CMakeLists.txt` parsing is not used for correctness, but may be
revisited later purely to recover source-level intent the File API
discards (comments, variable names, original target grouping/ordering) for
more idiomatic codegen. Not needed for the first fixture.

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
