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
  and reads the reply for each target's name, type, sources, build
  artifacts (the built binary's path), compile-group include directories,
  file sets, and inter-target dependencies, plus the top-level project's
  name. All already resolved by CMake itself (generator expressions
  evaluated, `if()`/variables/`find_package()` already accounted for) —
  this avoids re-implementing CMake-the-language, at the cost of requiring
  a real `cmake` invocation in the pipeline (not yet hermetic on the CMake
  side — see [build-verification.md](build-verification.md)) and tying
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

## What the frontend produces

The internal model (`translator/src/model.rs`) a `Target` currently
carries: `kind` (`Executable` | `Library`), private `sources`, public
`public_headers`, `dependencies` (resolved target names), `includes`
(this target's own include directories), and `artifacts` (build output
paths, for ground-truth comparison). External deps, compile
definitions/options, and linker options aren't modeled yet.

### Library targets: public vs. private headers

CMake's File API reports a target's headers in its `sources` list, but
only distinguishes public from private ones via `target_sources(...
FILE_SET ... TYPE HEADERS)` — each such source gets a `fileSetIndex`
pointing at a `fileSets[]` entry with `visibility: "PUBLIC"` (or
`"INTERFACE"`). **Decision:** only file-set-declared public headers become
a target's `cc_library` `hdrs`; everything else (including `.hpp` files
added as plain sources, with no file set) stays in `srcs`. The translator
does **not** guess which plain-source headers are "meant to be public" —
see the `needs_attention/` mechanism below for what happens when that gap
actually matters (a dependent target exists).

### Include directories

`target_include_directories()` (or a `FILE_SET`'s `BASE_DIRS`) surfaces in
the File API as `compileGroups[].includes[].path` — as an absolute path,
identically for both the target that declared it *and* every dependent
that inherited it via `target_link_libraries`. The File API doesn't
separately flag "mine" vs. "inherited," so the frontend distinguishes them
by each include's `backtrace`: an include entry whose backtrace resolves
to a `target_link_libraries` command in `backtraceGraph` was inherited (a
dependency pulled it in); anything else (`target_include_directories`, a
`FILE_SET`'s `BASE_DIRS`, or whatever else produced it) is the target's
own. Only the target's own include dirs are captured (as project-relative
paths, via the codemodel index's top-level `paths.source`) — Bazel's
`cc_library` `includes` attribute is transitive, so a consumer gets a
dependency's include dirs automatically through its `deps` edge, without
the frontend needing to duplicate that inheritance itself.

Note this doesn't give the same encapsulation as a hand-written
`hdrs`-only `cc_library` — but not for the reason it might appear. Bazel
does not enforce the `hdrs`/`srcs` split by default at all: a header in a
dependency's `srcs` is still propagated as an input to dependents' compile
actions, so consumers can `#include` it whether or not it was declared in
`hdrs`. `includes` only supplies the `-I`-style search path that decides
how the `#include` is *spelled*; it is not what exposes the file. Either
way this matches CMake's own looser semantics (a consumer can `#include`
any header under an include directory, declared public or not), so it's a
faithful translation, just not a stricter one than the source project had.

See
[build-verification.md](build-verification.md#header-visibility-is-not-enforced-by-default)
for the experiment establishing this, and the open question about whether
the hermetic `llvm` toolchain's `layering_check` changes it.

## The `needs_attention/` mechanism

When the translator can't confidently resolve something for a *specific*
conversion, it writes a `needs_attention/<NNN>-<slug>.md` file into the
output tree (`translator/src/needs_attention.rs`) — actionable follow-up
for whoever picks up that converted project. This is deliberately **not**
called a "runbook": `docs/runbooks/` in this repo documents bazelifier's
own general escalation contract (for people building the translator);
`needs_attention/` is per-conversion, project-specific guidance, closer to
a to-do list than an interface spec. See
[runbook-interface.md](runbook-interface.md) for how the two relate.

Currently implemented trigger: a library target has header-like files with
no public `FILE_SET` declaration, **and** at least one other target
depends on it (a library nothing depends on has no consumer that could
need an exposed header, so it's not worth flagging). See
`cmake_api.rs::header_visibility_needs_attention`.

**Resolutions go in the generated output, never in the source project.**
An agent resolves a `needs_attention` item by editing the generated
`BUILD.bazel` (here, moving the right headers into `hdrs`) — not by adding
a `FILE_SET` to the project's `CMakeLists.txt`. The source build files are
the input being translated; changing them to make one project convert
cleanly leaves the translator no better at the next project with the same
shape, which is the actual goal. This holds for real projects as much as
for fixtures.

## Known hard cases (expect to escalate via `needs_attention/`)

- Generator expressions (`$<...>`) with complex conditions.
- `execute_process()` / custom commands that shell out arbitrarily.
- `find_package()` resolving to system or vendored libraries with no
  obvious Bazel equivalent.
- Conditional logic driven by `CMakeCache.txt` values or platform detection
  that doesn't map cleanly to Bazel `select()`.
- `OBJECT_LIBRARY`/`INTERFACE_LIBRARY` target types (not yet recognized —
  `to_target` returns `UnsupportedTargetType` for anything besides
  `EXECUTABLE`/`STATIC_LIBRARY`/`SHARED_LIBRARY`).

This list will grow as real fixtures surface real cases — treat it as a
living list, not a spec.
