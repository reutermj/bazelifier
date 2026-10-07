#!/usr/bin/env bash
# Runs a test the project registered — CMake's add_test() or automake's
# TESTS; this wrapper cannot tell and does not need to, since by here a test
# is a binary, a working directory and an optional pass regex. Named for what
# it does rather than for one frontend: it shipped as `run_cmake_test.sh`
# into five Autotools modules that have no CMake in them.
# (See docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md for where
# the CMake half of that model comes from.) Not `set -e`:
# the binary's own nonzero exit is data this script evaluates, not a reason
# to abort before checking the pass regex.
set -uo pipefail

binary_name="$1"
working_dir="$2"
# Defaulted, not "$3": Bazel DROPS a trailing empty string when it tokenizes
# `args`, so a test with no PASS_REGULAR_EXPRESSION arrives with argc=2 and a
# bare "$3" aborts under `set -u` before the binary ever runs. The
# working-directory slot dodges this by never being empty (codegen sends "."),
# but the regex is genuinely empty whenever CMake declared none — which is the
# common case, and every prior sh_test producer happened to declare one.
pass_regex="${3:-}"

# This wrapper, the test binary, and the module's runtime data are all in
# the same package, so in runfiles they are siblings in this script's own
# directory. Derive the module's runfiles root from $0 rather than from
# TEST_WORKSPACE: when this module is a *dependency* of the workspace under
# test, its files live under runfiles/<canonical_repo>/ (e.g. tinyxml2+/),
# not runfiles/<TEST_WORKSPACE>/ — and the canonical repo name is something
# the module itself cannot know. $0's directory is correct whether the module
# is the root or a dependency.
module_runfiles="$(cd "$(dirname "$0")" && pwd)"

# Runfiles are read-only and the test writes into its working directory
# (tinyxml2's xmltest writes resources/out/), so stage a writable copy of the
# module's DATA and run with that as the working directory.
#
# The binary itself is deliberately NOT run from the staged copy. A binary
# linked against a cc_shared_library finds its .so through an $ORIGIN-relative
# RUNPATH into Bazel's _solib_* tree; moving the executable out from under
# runfiles severs every one of those paths and it dies with "cannot open
# shared object file". Left in place it resolves with no LD_LIBRARY_PATH at
# all. Verified both ways (bzl-i4i.8).
#
# Safe because the test binaries resolve their data relative to the WORKING
# DIRECTORY, not to their own location — checked across the corpus; the only
# argv[0] uses are usage strings and minigzip's basename-based mode switch,
# neither of which cares where the executable sits.
work_root="${TEST_TMPDIR:-$(mktemp -d)}/work"
mkdir -p "${work_root}"
cp -RL "${module_runfiles}/." "${work_root}/" 2>/dev/null || true
# working_dir is "." for the module root, or a module-relative subdir — never
# empty (an empty positional arg would be dropped when Bazel tokenizes args).
run_dir="${work_root}/${working_dir}"
mkdir -p "${run_dir}"

# In runfiles, not in the staged copy — see above.
binary="${module_runfiles}/${binary_name}"
chmod +x "${binary}" 2>/dev/null || true

output="$(cd "${run_dir}" && "${binary}" 2>&1)"
exit_code=$?

echo "${output}"

if [[ -n "${pass_regex}" ]]; then
  if ! grep -qE "${pass_regex}" <<<"${output}"; then
    echo "FAIL: output did not match PASS_REGULAR_EXPRESSION ${pass_regex}" >&2
    exit 1
  fi
  # With a pass regex, CTest treats a match as success regardless of exit
  # code; mirror that (many test harnesses signal via output, not status).
  exit 0
fi

# The project harness's SKIP: automake's test driver reports exit 77 as a
# skipped test rather than a failure (Open MPI's mpool_memkind without
# memkind). Bazel has no skip, so it passes — loudly, so a log reader sees
# that nothing was tested.
if [[ -n "${SKIP_EXIT_CODE:-}" && "${exit_code}" == "${SKIP_EXIT_CODE}" ]]; then
  echo "SKIP: exited ${exit_code}, which the project's own test harness reports as skipped"
  exit 0
fi

exit "${exit_code}"
