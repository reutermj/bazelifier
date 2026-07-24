# Build verification

Covers how we confirm a conversion actually worked, and the plan for making
that verification hermetic over time.

## Success bar

A conversion is verified when:

1. The generated Bazel targets **build**.
2. The project's existing **tests pass** when run under Bazel.

## Current approach (starting point)

Use the host's local `cmake` and `ninja` installations where needed —
e.g. to establish ground truth for what the original build produces
(binaries, generated sources, compile flags), or to cross-check the
Bazel-built output against the CMake-built output. This is the fastest path
to getting the pipeline working end-to-end and is acceptable for now.

## Direction: push verification into Bazel

Longer term, verification should happen as much as possible *inside* Bazel
itself rather than by shelling out to a local toolchain:

- Bring `cmake` and `ninja` into Bazel as hermetic toolchains/rules (e.g.
  rules that invoke a Bazel-provided `cmake`/`ninja` rather than whatever's
  on the host `PATH`), so builds don't depend on host state.
- This unlocks remote execution and distributed testing for both the
  verification step and, eventually, for driving/comparing against the
  original CMake build itself.
- Move incrementally: local-toolchain correctness first, hermeticity second.
  Don't block early translator progress on having a fully hermetic
  cmake/ninja-in-Bazel setup.

**Open question:** which existing Bazel rules for cmake/ninja interop (if
any suitable ones exist) can we adopt vs. needing to write our own. Worth a
survey once the translator has enough real fixtures to test against.

## Test discovery

Figuring out what "the project's existing tests" means varies by project
(CTest, GoogleTest registered via CTest, hand-rolled test binaries run via
custom targets, etc.). No fixed approach yet — expect this to be refined
once we have real CMake fixtures with tests to convert.
