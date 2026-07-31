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

**Decision:** the frontend uses CMake's own resolved output as its primary
source of truth, not `CMakeLists.txt` parsing. Three sources feed it, all
in `translator/src/cmake_api.rs`:

- **`codemodel-v2`** (File API) — configures the project (`cmake -B <dir> -G
  Ninja`) and reads the reply for each target's name, type, sources, build
  artifacts (the built binary's path), compile-group include directories
  **and compile definitions**, file sets, inter-target dependencies, the
  per-target backtrace (which command defined it — used to tell a
  project-authored target from one a CMake module injected), and each
  directory's install rules (`installers[]`). Plus the top-level project's
  name. All already resolved by CMake itself (generator expressions
  evaluated, `if()`/variables/`find_package()` already accounted for) —
  this avoids re-implementing CMake-the-language, at the cost of requiring
  a real `cmake` invocation in the pipeline (not yet hermetic on the CMake
  side — see [build-verification.md](build-verification.md)) and tying
  translation to a given CMake version's File API schema.
- **`cache-v2`** (File API) — read for two cache entries:
  `CMAKE_PROJECT_VERSION` (when the top-level `project()` specified a
  `VERSION`; becomes the generated `MODULE.bazel`'s own `version`, omitted
  when absent since Bazel's `module()` doesn't require one), and
  `CMAKE_ROOT` (the CMake installation path, used to recognize targets a
  CMake-provided module injected — see the UTILITY-target filtering below).
- **`ctest --show-only=json-v1`** — the File API has *no* test model, so
  registered tests (`add_test`) come from CTest instead: each test's
  command, `WORKING_DIRECTORY`, and `PASS_REGULAR_EXPRESSION`. Run after the
  build (so the test binaries' paths resolve). See
  [../lore/cmake-test-model-lives-in-ctest-not-file-api.md](../lore/cmake-test-model-lives-in-ctest-not-file-api.md)
  and the test-model section below.

After configuring, the frontend also runs the actual build (`cmake
--build`) to produce ground-truth artifacts for validation — see
[build-verification.md](build-verification.md). This is not a File API
query; it's a real build, reusing the same configured `build_dir`, and it
must precede the `ctest` query above.

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

The internal model (`translator/src/model.rs`) has two top-level types on
its `BuildGraph`: `Target`s and `Test`s.

A `Target` currently carries: `kind` (`Executable` | `Library`), private
`sources`, public `public_headers`, `dependencies` (resolved target names),
`includes` (this target's own include directories), `local_defines` (this
target's own compile definitions — see the compile-definitions section
below), and `artifacts` (build output paths, for ground-truth comparison).
External deps, compile *options*, and linker options aren't modeled yet.

A `Test` (from CTest, not the File API) carries the test name, the target
it runs, its `working_directory` (module-relative), and an optional
`pass_regex`. See the test-model section below.

### Library targets: public vs. private headers

CMake's File API reports a target's headers in its `sources` list without
marking them public or private. **Decision:** a header becomes a target's
`cc_library` `hdrs` when *either* of two authoritative signals says it is
public, and stays in `srcs` otherwise:

- a `target_sources(... FILE_SET ... TYPE HEADERS)` with `visibility:
  "PUBLIC"`/`"INTERFACE"` (CMake 3.23+) — the source carries a
  `fileSetIndex` into a `fileSets[]` entry; or
- an `install(FILES ... TYPE INCLUDE)` (or a target's `INCLUDES
  DESTINATION`) rule — the pre-FILE_SET way to declare a public header,
  which the many projects that never adopted `FILE_SET` still use. The File
  API reports these in the directory reply's `installers[]`; a file
  installed to an `include` destination is being declared public (see
  `installed_public_headers`). The destination is matched by
  `is_include_destination`, which accepts both the relative `include[/sub]`
  and the absolute `<prefix>/include[/sub]` that
  `CMAKE_INSTALL_FULL_INCLUDEDIR` expands to (json-c installs there); an
  `include` nested under `lib`/`lib64`/`share` is a build-private tree and
  does not count.

The translator does **not** guess beyond those declarations. A header in
`srcs` with neither signal, on a library something depends on, is the gap
the `needs_attention/` mechanism below flags — it is genuinely ambiguous,
not merely undeclared.

**Headers no target enumerated.** A large class of C projects lists only
`.c` files on a target and leaves headers ambient on the include path —
CMake compiles each source and finds its headers via `-I`, so a header that
is never a build input is never reported as a target source, and
`copy_referenced_sources` would never copy it. The converted library then
fails to compile the moment one of its own sources `#include`s that header.
When such a header is `install()`-declared public (the authoritative signal
above), `inject_unenumerated_installed_headers` adds it to the `hdrs` of
every library whose own include directories contain it — so it is copied and
reachable. This is deliberately scoped to headers the project *declared*
public: it does not copy every header sitting under an include dir, so a
header with no public evidence still defaults to private/absent rather than
being guessed public. json-c is the live case — a `CMakeLists` ordering bug
drops `json_pointer.h`/`json_patch.h` from the library's header list, yet
`json_pointer.c` includes `json_pointer.h`; both are `install()`d public, so
both are injected.

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
own. Only the target's own include dirs are captured — Bazel's `includes`
attribute is transitive, so a consumer gets a dependency's include dirs
automatically through its `deps` edge, without the frontend needing to
duplicate that inheritance itself. They are captured as reported
(absolute), then made module-relative by the rebasing step below.

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

### Compile definitions

`target_compile_definitions()` surfaces in the File API as
`compileGroups[].defines[]`, each `{define, backtrace}` where `define` is
the full `NAME` or `NAME=VALUE`. **Decision:** these are emitted as Bazel
`local_defines` (non-propagating), not `defines`. The reason is that the
File API reports the *effective* set on each target's own compile line with
the PUBLIC/PRIVATE/INTERFACE origin already flattened away — so we cannot
tell which defines are meant to propagate. Making them all non-propagating
and letting each converted consumer re-derive its own from its own compile
group is self-consistent: every target gets exactly the set CMake computed
for it. It is wrong only for a consumer *outside* the converted set, which
never inherits a PUBLIC define it should have; recovering that split
(`defines` vs `local_defines`) via the backtrace graph is future work. The
full shape, including why generator-expression-conditional defines only
appear for the configured config, is in
[../lore/cmake-file-api-compile-definitions-shape.md](../lore/cmake-file-api-compile-definitions-shape.md).

### `add_custom_target` / UTILITY targets

CMake has no Bazel rule the translator maps `UTILITY` targets to (the
product of `add_custom_target` — a named build step, not a compiled
artifact). Rather than escalate every one, the frontend distinguishes two
cases by **provenance** (the target's backtrace file) and **inertness** (no
artifacts, no dependents):

- A UTILITY target a CMake-provided module injected — its defining command
  lives under `CMAKE_ROOT` — is **dropped silently**. This is what keeps
  `include(CTest)`'s ~28 dashboard targets (and a Doxygen module's `doc`
  target) out of the escalation stream entirely. See
  [../lore/cmake-include-ctest-injects-utility-targets.md](../lore/cmake-include-ctest-injects-utility-targets.md).
- A UTILITY target the *project itself* authored, but still inert, is
  **aggregated** into a single `needs_attention/` item rather than one
  apiece, so the drop is a decision rather than an oversight.
- A UTILITY target that is load-bearing (has artifacts, or something
  depends on it) is escalated **individually** — dropping it would leave
  real dependents incomplete.

### Registered tests (CTest)

The File API has no test model, so `add_test`-registered tests come from
`ctest --show-only=json-v1` (above). Each becomes a `model::Test` carrying
the target it runs, its `WORKING_DIRECTORY` (rebased module-relative), and
its `PASS_REGULAR_EXPRESSION`. Codegen turns each into a Bazel `sh_test`
that runs the binary at that working directory — with the runtime data
staged writable — and asserts the pass regex, i.e. the project's own
correctness criterion translated rather than invented. This is currently
tinyxml2-shaped (that subset of properties); the long tail
(`FAIL_REGULAR_EXPRESSION`, `WILL_FAIL`, test fixtures, multi-config) is
future work. See
[../lore/cmake-test-model-lives-in-ctest-not-file-api.md](../lore/cmake-test-model-lives-in-ctest-not-file-api.md)
and [build-verification.md](build-verification.md#equivalence-checks).

## The `needs_attention/` mechanism

When the translator can't confidently resolve something for a *specific*
conversion, it writes a `needs_attention/<NNN>-<slug>.md` file into the
output tree (`translator/src/needs_attention.rs`) — actionable follow-up
for whoever picks up that converted project. See
[needs-attention-interface.md](needs-attention-interface.md) for the
format and the design intent behind it.

Currently implemented triggers:

- **Header visibility** — a library target has header-like files with no
  public `FILE_SET` declaration, **and** at least one other target depends
  on it (a library nothing depends on has no consumer that could need an
  exposed header, so it's not worth flagging). See
  `needs_attention.rs::header_visibility_needs_attention`.
- **Unsupported target type** — the target's CMake type has no Bazel rule
  yet (anything besides `EXECUTABLE`/`STATIC_LIBRARY`/`SHARED_LIBRARY`).
  The target is skipped and *the rest of the project still converts*; one
  unrecognized target must not cost the project every other target it
  defines. The escalation carries type-specific guidance rather than a
  generic "unsupported" message. See
  `needs_attention.rs::unsupported_target_needs_attention`.
- **Generated sources** — a target consumes a source CMake produces during
  the build (`isGenerated`), such as an `add_custom_command()` output or
  the objects an `OBJECT_LIBRARY` splices into its consumers. See
  `needs_attention.rs::generated_sources_needs_attention`.
- **Sources the module cannot reach** — a target compiles a file the
  translator could not place inside the generated module. See
  `needs_attention.rs::sources_outside_deliverable_needs_attention`.

### What actually makes an input a problem: reproducibility

The criterion is **not** where a file sits on the machine that ran the
conversion. It is whether the file can be reproduced from the source
deliverable — the tarball, checkout, or directory the project ships. That
framing is deliberately version-control agnostic: a tarball is a perfectly
valid way to deliver source code, so nothing here consults git, and
nothing should.

Three tiers fall out of it, and they want different responses:

1. **In the deliverable, outside the CMake source directory** — e.g. a
   sibling `../shared/util.cpp` that ships alongside the project. Nothing
   is wrong: the file is reproducible, and the translator handles it by
   widening the module root (see below) rather than escalating.
2. **Not in the deliverable, but derivable from it** — generated sources.
   Also legitimate: the recipe ships with the project. This is a
   translator **capability gap**, not a project defect — the escalation
   says so explicitly, because "your generated file is a problem" is the
   wrong message to send about a normal build construct.
3. **Neither in the deliverable nor derivable from it** — an absolute path
   into a system location, a machine-local checkout, a prebuilt artifact.
   This is the only tier that indicates something genuinely wrong: the
   build has an input that cannot be reproduced from what the project
   ships, and no conversion can be faithful while that holds.

### The module root is derived, and the deliverable root caps it

A converted module's root is **not** assumed to be the CMake project
directory. `cmake_api.rs::rebase_to_module_root` computes it as the deepest
directory containing both the project and every referenced file that ships
with it, then rewrites every path relative to that. When nothing reaches
outside the project — the common case — the root *is* the project
directory and nothing changes. When the build compiles `../shared/util.cpp`
from a sibling that ships alongside, the root widens to cover both and the
paths become `proj/src/main.cpp` and `shared/util.cpp`.

That widening is capped by an explicitly declared **deliverable root**
(`--deliverable-root`, or the `deliverable_root` attribute on
`convert_cmake_project`), defaulting to the CMake project directory. It
answers the question the tiers turn on — what does this project ship? —
without the translator inferring it. Inference was rejected deliberately:
it fails silently, and in the direction of packaging too much.

The cap is what separates tier 1 from tier 3, and the same project sorts
into either depending on what it declares. With the default root, a
sibling-directory source is unreachable and escalates; declare a root
containing both and it is simply part of the module. Tier 3 is then
precisely "referenced from outside what the project ships," which is the
thing actually worth reporting.

Include directories are handled the same way, with one difference: one that
ends up outside the module is a system include path (`/usr/include`), which
has no `includes` translation and is dropped rather than escalated.

### Only referenced files enter the module

`main.rs::copy_referenced_sources` copies exactly the files the build graph
names — every target's `sources` and `public_headers` — rather than
recursively copying the source directory. The output is a Bazel module, not
a mirror of the CMake project.

Besides keeping `.git/`, stale build outputs and editor scratch files out
of a converted module, this makes the module reproducible by construction:
every file in it traces to a build-graph reference. A file that exists in
the source tree but is not part of the build — a gitignored leftover, an
artifact of an earlier in-source build — cannot silently become part of the
deliverable. That property needs no knowledge of version control. The
project's own `CMakeLists.txt` is not copied either; nothing in the
generated module builds from it.

### Source paths are only conditionally relative

The File API reports `sources[].path` **relative to the top-level source
directory only when the file is actually inside it**; anything else comes
through as an absolute path. So a source path cannot be passed through to a
generated `srcs` unvalidated — an absolute path is not a usable Bazel
label, it bakes the build machine's filesystem layout into output meant to
be checked into someone else's repo, and the file isn't in the generated
module anyway unless it was copied there.

Two pieces handle this, and they are not redundant with each other:

- `rebase_to_module_root` (above) resolves every path against the derived
  module root, keeping what lands inside it and escalating what doesn't.
  This is where the decision is made.
- `model::is_module_relative` is the single definition of what "inside"
  means — relative, with no `..` components. Codegen re-checks it on every
  emitted path in `render_path_list` and panics on a violation, so a
  frontend field that forgets to rebase can't reach the output silently.
  See [bazel-codegen.md](bazel-codegen.md#every-emitted-path-must-be-module-relative).

Note that `isGenerated` is *not* a sufficient test on its own: an ordinary,
non-generated source in a sibling directory is reported absolute and
carries no flag distinguishing it.

### Skipping a target without breaking its dependents

When a target is skipped, any surviving target that named it as a
dependency has that edge **dropped** from its generated `deps`. Keeping it
would emit a label pointing at a target that was never generated, which
fails at Bazel *analysis* time with an error far removed from the real
cause — and leaves the agent no workspace to resolve the escalation in.
The dropped edges are listed in the escalation itself, so the information
isn't lost.

This is a genuine judgement call rather than an obviously-correct
translation: the File API's `dependencies` list conflates real link
dependencies with order-only edges from `add_dependencies()`, and nothing
distinguishes them. An order-only edge contributes nothing to the binary
and dropping it is harmless; a link edge means the dependent is incomplete
until the skipped target is translated. The escalation says so explicitly
rather than the translator guessing which case it's in.

Whatever an agent does about a dropped edge, it does in the generated
output — the source `CMakeLists.txt` is immutable input. See
[build-verification.md](build-verification.md#the-input-cmake-is-immutable).

## Known hard cases (expect to escalate via `needs_attention/`)

- Generator expressions (`$<...>`) with complex conditions.
- `execute_process()` / custom commands that shell out arbitrarily.
- `find_package()` resolving to system or vendored libraries with no
  obvious Bazel equivalent.
- Conditional logic driven by `CMakeCache.txt` values or platform detection
  that doesn't map cleanly to Bazel `select()`.
- `OBJECT_LIBRARY`/`MODULE_LIBRARY`/`UTILITY` target types — recognized as
  untranslatable and escalated via `needs_attention/` (see above), not
  mapped to a Bazel rule yet.
- `INTERFACE_LIBRARY` is a special case worth knowing about: it does **not
  appear in the codemodel reply at all** (verified against CMake 3.28 +
  Ninja), even when another target links it. So the unsupported-type
  escalation cannot catch it — the translator never sees it. Its usage
  requirements do reach a consumer's `compileGroups`, but with a backtrace
  pointing at `target_link_libraries`, so they're classified as inherited
  and dropped on the assumption a Bazel `deps` edge will supply them —
  which it won't, because no rule was generated for the interface library.
  A consumer of a header-only `INTERFACE` library can therefore lose its
  include dirs silently. Not yet handled.

This list will grow as real fixtures surface real cases — treat it as a
living list, not a spec.
