# Build verification

Covers how we confirm a conversion actually produces a genuinely
independent, functionally equivalent Bazel module — and the plan for
making that verification hermetic over time.

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
   CMake project — not necessarily binary-identical, but behaviorally the
   same. See "Equivalence checks" below for what that means concretely.

We are not targeting binary compatibility with the CMake/Ninja build.
Toolchain differences (hermetic LLVM vs. whatever the host CMake used),
flag normalization, etc. mean the built artifacts will differ at the byte
level. What must match is behavior.

## Pipeline (implemented)

1. **`convert_cmake_project`** (a Bazel rule,
   `translator/build_defs/convert_cmake_project.bzl`) runs the `bazelifier`
   binary as an action against a fixture's CMake sources. The translator:
   - Configures the project and runs the CMake File API (`codemodel-v2`,
     `cache-v2`) to discover targets — see
     [cmake-frontend.md](cmake-frontend.md).
   - Actually builds the project (`cmake --build`) to produce **ground-truth
     artifacts** (the real cmake+ninja-built binaries).
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
   - Every fixture's tree artifact is renamed to `fixtures/<fixture dir
     name>/` inside the tarball (via `tar.bzl`'s `mtree_mutate`
     `strip_prefix`/`package_dir`).
   - A root `MODULE.bazel` is generated declaring a real `bazel_dep` +
     `local_path_override` for every fixture module. Because these are
     genuine Bzlmod dependencies, fixture modules can also depend on **each
     other** once bazelifier converts projects with real inter-project
     dependencies — this validates cross-project composition, not just
     single-project isolation.
   - A root `BUILD.bazel` is also generated, containing one `sh_test` per
     expected target per fixture (see "Equivalence checks"), aggregated
     into a `test_suite`.
3. **Unpack and validate, fully outside this repo.** The tarball is
   extracted into a plain directory with no connection to bazelifier's own
   `MODULE.bazel`. From that directory's root, `bazel build //...` /
   `bazel test //:all_ground_truth_comparisons` resolves every fixture as
   a real Bzlmod dependency and builds/tests it. This is the step that
   actually proves independence — see "Why unpack it" below.

## Equivalence checks

Implemented today:

- **`needs_attention/` gate.** Before comparing anything, the generated
  `sh_test` checks the fixture's `needs_attention/` directory (see
  [cmake-frontend.md](cmake-frontend.md)) for any unresolved items. If any
  exist, the test fails immediately, printing their full content, and the
  comparison below never runs. This is deliberate, not a shortcut: a
  conversion with an open `needs_attention` item can still happen to build
  and behave correctly today (see `003-library-no-file-set`, where
  `includes`' looser exposure means the build works despite the header
  visibility gap) — if the comparison ran anyway and passed, it would read
  as "this is fine," masking a real, unresolved translation gap. The gate
  forces triage first: an agent (or human) is expected to resolve the
  flagged issue and re-run the conversion so `needs_attention/` comes back
  empty before the equivalence check means anything.
- **Runtime output comparison** (`translator/build_defs/compare_runtime_output.sh`,
  wired up as a generated `sh_test` per target): run the ground-truth
  binary and the Bazel-built binary, diff stdout, stderr, and exit code.
  Directly answers "does it behave the same," and works even for a
  fixture with no CMake-registered tests of its own.

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
- **CMake's own registered tests** (CTest/`add_test()`), run against the
  Bazel-built test binary: the strongest signal once a fixture has one,
  since it reuses the project's own correctness assertions rather than
  ours.
- **Target inventory**: every CMake codemodel target has a corresponding
  Bazel target that built. Cheap, but only catches gross omissions.

## Fixtures

- `001-hello-world` — single `cc_binary`, no dependencies.
- `002-with-library` — a library with a properly declared public `FILE_SET`
  header; exercises `cc_library` codegen (`hdrs`, `includes`, `deps`) end
  to end. Passes both the gate and the comparison.
- `003-library-no-file-set` — deliberately exercises the `needs_attention/`
  gate: a library with plain (non-`FILE_SET`) headers and a consumer.
  Expected to **fail** `bazel test` until the underlying `CMakeLists.txt`
  is fixed to declare a public `FILE_SET` and the fixture is
  re-converted — this is the intended, correct behavior, not a bug in the
  fixture.

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
