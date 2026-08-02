# Build verification

Covers how we confirm a conversion actually produces a genuinely
independent, functionally equivalent Bazel module — and the plan for
making that verification hermetic over time.

This document is about proving ONE conversion correct. Noticing that a
change moved some *other* project is a different question, answered by
[pipeline-metrics.md](pipeline-metrics.md).

## Success bar

A conversion is verified when:

1. The generated output is a **standalone Bazel module** (its own
   `MODULE.bazel` + `BUILD.bazel`, own `bazel_dep`s) that builds with **no
   reference back to bazelifier's own workspace**. This is the whole point
   — a converted project must be usable by dropping it into someone else's
   repo, checked in on its own, with bazelifier nowhere in the picture. A
   `BUILD.bazel` that only happens to build because it inherited
   bazelifier's `MODULE.bazel` does not count.
2. The generated module is **functionally equivalent** to the original
   project — not necessarily binary-identical, but behaviorally the same.
   See "Equivalence checks" below for what that means concretely.

We are not targeting binary compatibility with the original build.
Toolchain differences (hermetic LLVM vs. whatever the host build used),
flag normalization, etc. mean the built artifacts will differ at the byte
level. What must match is behavior.

## Pipeline (implemented)

1. **`convert_cmake_project`** (a Bazel rule,
   `translator/build_defs/convert_cmake_project.bzl`) runs the `bazelifier`
   binary as an action against a fixture's sources. Its `frontend` attribute
   selects which build system to read, and picks both the file that marks the
   project root and the `--frontend` the translator is told to use;
   `convert_autotools_project` is a thin wrapper that passes
   `frontend = "autotools"`. Passing it explicitly rather than relying on the
   translator's own detection matters for a project shipping BOTH — xz has
   `CMakeLists.txt` and `configure.ac`, and which to convert is the BUILD
   author's choice. The translator:
   - Configures the project and interrogates its build system to discover
     targets — the CMake File API (`codemodel-v2`, `cache-v2`), see
     [cmake-frontend.md](cmake-frontend.md); or the build's own command
     output plus `make -p`, see
     [autotools-frontend.md](autotools-frontend.md).
   - Actually builds the project to produce **ground-truth artifacts** (the
     real binaries the project's own build system produced).
   - Emits a standalone Bazel module: `MODULE.bazel`, `BUILD.bazel` (the
     user-facing converted output — see
     [bazel-codegen.md](bazel-codegen.md)), copied sources, and a
     `ground_truth/` subdirectory containing the real built binaries plus
     a small `exports_files`-only `BUILD.bazel` (kept separate from the
     top-level `BUILD.bazel` specifically so validation-only targets never
     leak into the user-facing converted output).
   - The whole thing is declared as a single Bazel **tree artifact**.
2. **`validation_workspace`** (`translator/build_defs/validation_workspace.bzl`)
   takes a list of `convert_cmake_project` targets (fixtures) and packages
   them into one tarball:
   - The tarball's exact layout is staged on disk first — every fixture's
     tree artifact copied to `fixtures/<fixture dir name>/`, the root files
     written alongside — and that one directory is then archived.
   - A root `MODULE.bazel` is generated declaring a real `bazel_dep` +
     `local_path_override` for every fixture module. Because these are
     genuine Bzlmod dependencies, fixture modules can also depend on **each
     other** once bazelifier converts projects with real inter-project
     dependencies — this validates cross-project composition, not just
     single-project isolation.
   - A root `BUILD.bazel` is also generated, containing one `sh_test` per
     expected target per fixture (see "Equivalence checks"), aggregated
     into a `test_suite`.
3. **Unpack, fully outside this repo.** The tarball is extracted into a
   plain directory with no connection to bazelifier's own `MODULE.bazel`.
   From that directory's root, `bazel build //...` /
   `bazel test //:all_ground_truth_comparisons` resolves every fixture as
   a real Bzlmod dependency and builds/tests it. This is the step that
   actually proves independence — see "Why unpack it" below.

   A fixture that reproduces a `configure_file` config header depends on
   `cc_config` (the toolchain-probing module — see
   [configure-file-and-toolchain-probes.md](configure-file-and-toolchain-probes.md)).
   `cc_config` is not published yet, so the validation run supplies it with a
   flag rather than baking a path into the portable tarball:

   ```sh
   bazel test //:all_ground_truth_comparisons --keep_going \
       --override_module=cc_config=<bazelifier-checkout>/cc_config
   ```

   `--keep_going` is not optional in practice. Without it a single fixture
   that fails to *build* aborts the whole invocation — every other fixture
   reports `NO STATUS` and the run yields no signal at all. That is exactly
   the situation the suite is most needed in: one corpus project mid-
   conversion should not blind you to the other twenty-odd.

   This overrides a genuine third-party dependency to a local checkout, the
   standard dev-mode mechanism; it does **not** reference bazelifier's own
   module. It stays on the validation invocation (our harness), not in the
   tarball, so the deliverable remains path-free and portable — a real
   consumer would resolve `cc_config` from a registry. A fixture with no
   `configure_file` needs no flag.

   Without the flag, module resolution fails with `module cc_config@0.0.0
   not found in registries`. **That is the tarball's expected state, not a
   packaging defect** — the two "fixes" it invites (writing the override
   into the generated root `MODULE.bazel`, or staging a copy of
   `cc_config/` inside the tarball) each destroy the portability this step
   exists to prove, and both have been attempted here. See
   [the lore entry](../lore/cc-config-is-supplied-by-flag-not-shipped-in-the-tarball.md);
   `//translator/tests:root_module_cc_config_note_test` enforces it.
4. **Agent triage.** Any fixture that emitted `needs_attention/` items is
   an unfinished conversion. An agent reads each item and resolves it by
   editing the *generated* `BUILD.bazel` in the unpacked workspace, then
   the build and tests are re-run. This repeats until the suite is green.
   The agent stage is part of the pipeline under test, not a manual
   escape hatch: what these fixtures validate is that **the translator
   plus an agent** can convert a project, not that the translator can do
   it alone.

The unit under test is the whole pipeline. **Green is the only passing
state** — a red fixture means an open `needs_attention` item the agent stage
has not yet resolved, which is a result to act on. That an escalation-firing
fixture *starts* red is expected and documented (it is how the agent-in-the-
loop cycle is tested); what is never acceptable is treating a red fixture as
a *finished* outcome — leaving the item open and calling it done, or making
it green by editing the immutable input instead of the generated output.

### The input build files are immutable

A fixture's own build files — `CMakeLists.txt`, `Makefile.am`,
`configure.ac` — are test inputs and are **never edited to make a
conversion succeed**. A pattern the translator can't handle (like
`003-library-no-file-set`'s plain, non-`FILE_SET` headers) is a real
shape found in real projects — the goal is a translator and escalations
robust enough to handle it, not a corpus curated down to the subset that
happens to convert cleanly. "Fix the project's build files" is never the
resolution to a `needs_attention` item.

## Equivalence checks

Implemented today:

- **`needs_attention/` gate.** Before comparing anything, the generated
  `sh_test` checks the fixture's `needs_attention/` directory (see
  [cmake-frontend.md](cmake-frontend.md)) for any unresolved items. If any
  exist, the test fails immediately, printing their full content, and the
  comparison below never runs. This is deliberate, not a shortcut: a
  conversion with an open `needs_attention` item can still happen to build
  and behave correctly today (see `003-library-no-file-set`, where the
  build works despite the header visibility gap because Bazel doesn't
  enforce `hdrs`/`srcs` — see "Header visibility is not enforced by
  default" below) — if the comparison ran anyway and passed, it would read
  as "this is fine," masking a real, unresolved translation gap. The gate
  forces triage first: the agent stage (step 4 above) resolves the flagged
  issue **in the unpacked workspace's generated `BUILD.bazel`** and the
  build is re-run, so `needs_attention/` comes back empty before the
  equivalence check means anything. An unresolved item is an unfinished
  conversion, not an accepted outcome.

  The gate can't check for zero items by testing whether
  `needs_attention/` exists in the test's runfiles: Bazel drops an empty
  `data` filegroup from runfiles entirely rather than leaving an empty
  directory behind, so a fixture with genuinely zero items and one whose
  runfiles wiring is simply broken (e.g. a change to Bazel's canonical
  repo naming) would look identical — both silently skip the gate and fall
  through to the comparison. To close that gap, the
  translator always writes a `needs_attention/MANIFEST` file (see
  `main::write_needs_attention`) alongside the `.md` items, and
  `render_needs_attention_build_bazel` adds it to the `filegroup`'s `srcs`
  explicitly rather than only via the `*.md` glob — so, unlike the glob's
  output, it's guaranteed to survive into runfiles regardless of item
  count. `compare_runtime_output.sh` checks for `MANIFEST`'s presence, not
  the directory's, and fails loud if it's missing: that can only mean the
  wiring is broken, never a clean conversion.
- **Runtime output comparison** (`translator/build_defs/compare_runtime_output.sh`,
  wired up as a generated `sh_test` per target): run the ground-truth
  binary and the Bazel-built binary, diff stdout, stderr, and exit code.
  Directly answers "does it behave the same," and works even for a
  fixture with no CMake-registered tests of its own.

  A ground-truth binary that **dynamically links a project shared library**
  (json-c's `json_parse` against `libjson-c.so.5`, exercised by fixture
  `016-shared-library`) can't just be copied and run: the absolute RUNPATH
  CMake baked in points at the throwaway build directory, gone by test time,
  and the `.so` isn't otherwise in the test's runfiles. So
  `copy_ground_truth_artifacts` stages the shared library's whole versioned
  symlink chain (`libfoo.so` → `libfoo.so.5` → `libfoo.so.5.2.0`, flattened
  to real files so they survive the tarball) into `ground_truth/` next to the
  binary, groups them into a `shared_libs` filegroup the comparison test
  depends on, and the script points `LD_LIBRARY_PATH` at that directory for
  the ground-truth run. The Bazel-built binary needs none of this: a
  `cc_library` links into a `cc_binary` statically by default, so it is
  self-contained. This is functional-equivalence bookkeeping, not a change to
  what is compared — the two binaries' stdout/stderr/exit are still the whole
  contract.

  Some binaries emit **nondeterministic** output — json-c's `json_parse`
  prints `maxrss: <ru_maxrss> KB` to stderr, and that value differs between
  any two runs and between the two builds — so a byte-exact diff can never
  pass even when behavior is identical. Rather than a per-fixture filter (which
  would have to be hand-declared and could silently hide a real difference) or
  a global relaxation (which would weaken every fixture's check), the script is
  **self-calibrating**: it runs each binary *twice* and treats any output line
  that differs between a binary's own two runs as nondeterministic, excluding
  exactly those lines from the ground-truth-vs-Bazel comparison. A deterministic
  binary has zero such lines, so its comparison stays byte-exact — the check is
  not weakened for it. The masks from both binaries are unioned (a line that
  varies in either build is excluded from both), and the excluded set is
  line-indexed, so a *structural* nondeterminism (a binary's two runs differing
  in line count, not just a value) is failed loudly rather than masked, since
  the line-position model can't represent it. Fixture
  `019-nondeterministic-stderr` exercises this with a program that prints its
  PID. A binary whose two runs disagree on *exit code* is likewise surfaced,
  not reconciled.
- **CMake's own registered tests** (CTest/`add_test()`): the strongest
  signal, since it reuses the project's own correctness assertions rather
  than ours. The File API has no test model, so these are read from `ctest
  --show-only=json-v1` (see
  [../lore/cmake-test-model-lives-in-ctest-not-file-api.md](../lore/cmake-test-model-lives-in-ctest-not-file-api.md)),
  and the translator emits, per test, a `sh_test` that runs the binary at
  its declared `WORKING_DIRECTORY` — with the runtime data staged writable —
  and asserts the declared `PASS_REGULAR_EXPRESSION`. tinyxml2's `xmltest`
  is the live example. `validation_workspace` runs that test **instead of**
  the naive runtime-output comparison for the same binary: a data-driven
  test run without its data would fail identically on both sides and
  false-pass the diff. Currently tinyxml2-shaped (command +
  `WORKING_DIRECTORY` + `PASS_REGULAR_EXPRESSION`); the long tail
  (`FAIL_REGULAR_EXPRESSION`, `WILL_FAIL`, fixtures, multi-config) is
  future work.

Deferred until a fixture actually exercises them meaningfully (not worth
building against a fixture with nothing to differentiate):

- **Compile-command comparison**: CMake's `compile_commands.json` vs.
  Bazel's actual compile actions (`bazel aquery`), diffing a normalized
  subset (defines, include paths, `-std=`) — not exact flag equality, since
  Bazel and CMake will never fully agree on internal plumbing flags.
- **Symbol table comparison** (`nm`/`readelf -s`): confirms the same
  symbols are defined/exported, catching a silently-dropped source file
  without needing the binary to run. More useful once fixtures include
  libraries.
- **Target inventory**: every CMake codemodel target has a corresponding
  Bazel target that built. Cheap, but only catches gross omissions.

## Fixtures

- `001-hello-world` — single `cc_binary`, no dependencies. Its `project()`
  is also the only one to declare a `VERSION`, so it's what exercises
  `read_project_version` and the `MODULE.bazel` `version = "..."` line
  against real CMake output rather than only against hand-constructed
  Rust test data.
- `002-with-library` — a library with a properly declared public `FILE_SET`
  header; exercises `cc_library` codegen (`hdrs`, `includes`, `deps`) end
  to end. Passes both the gate and the comparison.
- `003-library-no-file-set` — exercises the `needs_attention/` gate and the
  agent stage: a library with plain (non-`FILE_SET`) headers and a
  consumer. The translator can't tell which headers are public, so it
  escalates rather than guessing; the agent is expected to resolve the
  item by populating `hdrs` in the generated `BUILD.bazel`. Passes once
  that resolution lands. Its `CMakeLists.txt` is deliberately left
  without a `FILE_SET` **permanently** — that's the input shape under
  test.

  Note this fixture still *builds* with an empty `hdrs`, so a green build
  alone does not prove the agent resolved it correctly — see "Header
  visibility is not enforced by default" below.
- `004-binary-private-include` — an executable with its own
  `target_include_directories()` and no dependencies. Covers the one path
  where nothing else can supply an include dir: Bazel propagates a
  dependency's `includes` transitively (which is what `002` exercises), but
  nothing supplies a target's *own*, so dropping it produces a module that
  fails to compile. Emits no `needs_attention/` item — that escalation is
  library-only — so this is a pure build + equivalence check.
- `005-unsupported-target-type` — exercises the `needs_attention/` gate for
  a target the translator could not emit **at all** (a `UTILITY` target
  from `add_custom_target`), plus the agent stage that resolves it. Where
  `003` escalates an attribute it couldn't populate, this escalates a whole
  missing target, and asserts the rest of the project still converts.

  The distinguishing property: `app` builds and its runtime output matches
  ground truth *even with the item open* — the dropped dependency edge is
  order-only, so it contributes nothing to the binary. Until the agent
  stage resolves the item, the gate is therefore the only thing failing
  this fixture, which is exactly the point. Escalation must not depend on
  the conversion also happening to break; otherwise gaps that produce
  working-but-wrong output go unnoticed.
- `006-sibling-sources` — a CMake project in `proj/` that compiles sources
  from `shared/`, a sibling directory shipping in the same deliverable.
  Exercises a module root **wider than the CMake project directory**: with
  `deliverable_root` set to the fixture directory, the module roots there
  and holds `proj/src/main.cpp` alongside `shared/helper.cpp`, all paths
  module-relative. Emits no `needs_attention/` item — the files are
  reproducible from what the project ships, so there is no gap to report,
  which is the property under test.

  The fixture is still one self-contained directory: the deliverable is the
  fixture, and the CMake project is a subdirectory of it. That keeps the
  one-directory-per-fixture convention while still placing a source outside
  the CMake project's own root.
- `007-generated-source` — an executable whose sources include one CMake
  produces via `add_custom_command()` at build time, exercising the
  `generated_sources_needs_attention` escalation no other fixture
  triggers. The generated function is deliberately never called from
  `main.cpp`: like `005`, the `needs_attention/` gate must be the only
  thing failing this fixture. If the generated source's contribution were
  load-bearing, the Bazel-generated `cc_binary` (missing it from `srcs`)
  would fail to *link*, which would fail the comparison test's own `data`
  dependency on the binary before the gate got a chance to run at all —
  conflating "the escalation fired" with an unrelated break.
- `008-sources-outside-deliverable-root` — the mirror image of `006`: a
  CMake project in `proj/` references a source from a sibling directory,
  but here `deliverable_root` is scoped to `proj/` alone rather than
  widened to the whole fixture directory, so the sibling is genuinely
  outside the declared deliverable. Exercises
  `sources_outside_deliverable_needs_attention`, which `006`'s
  non-escalating case doesn't reach. Same non-load-bearing-symbol
  discipline as `007`, for the same reason.

### Autotools fixtures

These live under `fixtures/autotools/` rather than in the numbered sequence,
which is the one thing to know about the layout: `validation_workspace`
stages every fixture by its package BASENAME, so the nesting is erased in
the tarball and an `autotools/003-foo` would collide with a CMake `003-foo`.
Numbers are not shared across the two sets, so pick names that stay distinct.

- `autotools/001-programs-and-libraries` — all three target shapes at once:
  `bin_PROGRAMS`, `noinst_LIBRARIES` and a libtool `lib_LTLIBRARIES`. Its
  frozen build-output and `make -p -n` captures are the evidence the
  `autotools.rs` unit tests deserialize.

  Enrolled, and it took two fixes to get there: libtool wrapper scripts
  broke its ground-truth capture (bzl-yjn.4), and header staging then
  *replaced* `greet.h` in `hdrs` with the staged copy, so `greet.c`'s quoted
  `#include "greet.h"` resolved to nothing (bzl-daj). Staging is additive
  now — a project can use both include styles for one header.
- `autotools/002-sibling-sources-recursive-make` — enrolled, and the fixture
  that pins recursive make: `app/` compiles `../common/util.c`, so every path
  in the command stream is relative to the subdirectory the command ran in
  rather than to the build root.
- `autotools/004-config-header` — the Autotools config-header path, which
  was unit-only until it existed: `AC_CONFIG_HEADERS` recovered from
  `config.status`, expanded through the shared `cc_config` catalog against
  the consumer's toolchain. Deliberately FULLY MAPPED, so it converts with
  no escalation and is green — the escalating direction is xz's, and it
  cannot be a fixture without being permanently red. Its binary prints what
  the header resolved to, so a probe answering differently than it did for
  the original build fails the comparison rather than passing silently.
- `autotools/003-sibling-outside-project-root` — enrolled, and the Autotools
  mirror of `006-sibling-sources`: the project is `proj/` but compiles
  `../shared/helper.c`, so the module root widens to the fixture directory and
  ships both. Deliberately the same shape as its CMake counterpart, because
  the two frontends have to agree — the Autotools frontend used to ignore
  `deliverable_root` entirely and silently drop the sibling. Its capped
  direction (a narrow `deliverable_root`, so the same source escalates
  instead) is covered by unit tests rather than a second fixture, since only
  the rule's attribute differs.

## Header visibility is not enforced by default

Bazel does **not** enforce the `hdrs`/`srcs` split for C++ headers by
default. A header listed only in a dependency's `srcs` is still propagated
as an input to a dependent's compile action, so the dependent can
`#include` it and the build succeeds. Propagation comes from the header
being declared in *some* target's `srcs`/`hdrs`; `includes` only supplies a
`-I` search path, which is useless on its own.

This was established experimentally, not assumed. The three-case matrix
that establishes it (Bazel 9.2.0, sandboxed), the `aquery` output showing
the header among a consumer's compile-action inputs, and why the intuitive
mental model gets the causality backwards are recorded once in
[docs/lore/bazel-does-not-enforce-hdrs-vs-srcs.md](../lore/bazel-does-not-enforce-hdrs-vs-srcs.md)
— not repeated here.

Consequences for this project:

- `hdrs` vs `srcs` in generated output is **documentation, not
  enforcement**. Getting it wrong degrades the public/private boundary
  without breaking the build.
- Therefore a green build does not prove a `needs_attention` header-
  visibility item was resolved *correctly* — only that it was resolved
  somehow. An agent that deleted the item without touching `hdrs` would
  also go green. Whether to add a structural assertion on the generated
  `BUILD.bazel` is still open.
- `layering_check` *would* enforce the split and is not active under any
  toolchain in use here — including the hermetic `llvm` one, so this is not
  a host-vs-hermetic distinction. The lore doc has the reason and the
  toolchain-source citation; it is the likeliest thing on this page to
  change, so it is stated once there rather than twice (bzl-ayl).

## Why unpack it (rather than validate in-tree)

An earlier version of this pipeline generated a fixture's `BUILD.bazel`
directly inside `translator/tests/fixtures/.../`, sharing bazelifier's own
`MODULE.bazel`. That built successfully, but for the wrong reason — it was
inheriting `rules_cc`/`llvm` from bazelifier's own workspace, so it never
actually proved the generated output was independent. Unpacking the
tarball into a directory with no relationship to this repo, then building
from *that* root, is what actually exercises "can a user drop this into
their own repo" rather than "does this happen to resolve when a sibling
`MODULE.bazel` already declared the right deps."

## Hermeticity status

- **Bazel side:** hermetic today. Every generated module depends on `llvm`
  (see [overview.md](overview.md)) for its C/C++ toolchain — verified by
  the fact that `libc++`/`libunwind` get compiled from source as part of
  every fixture build, not linked against the host's `libstdc++`.
- **CMake side:** not yet hermetic. `convert_cmake_project`'s action calls
  the host's `cmake` (via `use_default_shell_env = True`). This is the
  accepted current limitation — see the direction below.

## Direction: push CMake-side verification into Bazel

Longer term, the CMake configure+build step itself should also run through
a hermetic, Bazel-provided `cmake`/`ninja` rather than the host's:

- Unlocks remote execution and distributed testing for the ground-truth
  build, not just the converted output.
- Removes the last piece of host-state dependency from the whole pipeline.
- Move incrementally: today's `use_default_shell_env` approach is fine
  while fixture count is low — don't block translator progress on this.

**Open question:** which existing Bazel rules for cmake/ninja interop (if
any suitable ones exist) can we adopt vs. needing to write our own. Worth a
survey once the translator has enough real fixtures to test against.

## Test discovery

Figuring out what "the project's existing tests" means varies by project
(CTest, GoogleTest registered via CTest, hand-rolled test binaries run via
custom targets, etc.). No fixed approach yet beyond the runtime-output
comparison above — expect this to be refined as fixtures gain real
CMake-registered tests.
