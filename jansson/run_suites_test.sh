#!/usr/bin/env bash
# Runs jansson's test/run-suites the way `make check` does: from the test/
# build directory, with top_srcdir and top_builddir exported (test/Makefile.am's
# TESTS_ENVIRONMENT), and the built binaries where the suites look for them.
# Not `set -e`: run-suites' exit status is the result.
set -uo pipefail

# From $0, not TEST_WORKSPACE: as a dependency the module lives under
# runfiles/<canonical repo>/ (see run_registered_test.sh).
module="$(cd "$(dirname "$0")" && pwd)"

build="${TEST_TMPDIR:-$(mktemp -d)}/build"
mkdir -p "${build}/test/bin" "${build}/test/suites/api" "${build}/src/.libs"
# Symlinks keep each binary's $ORIGIN RUNPATH to libjansson resolving.
ln -sf "${module}/json_process" "${build}/test/bin/json_process"
for t in "${module}"/test_*; do
  ln -sf "${t}" "${build}/test/suites/api/$(basename "${t}")"
done
ln -sf "${module}/libjansson.so.4" "${build}/src/.libs/libjansson.so"

# api/check-exports compares `nm -D` names against src/jansson.def, but
# binutils >= 2.35 prints the version too (json_array@@libjansson.so.4), so
# upstream's own `make check` fails it against the ground-truth library on
# this host — a bug in the test, not the build. The shim strips the version
# so the test checks what it was written to: the exported SET. The versions
# themselves are what the version script reproduces.
real_nm="$(command -v nm || true)"
if [[ -n "${real_nm}" ]]; then
  mkdir -p "${build}/shim"
  printf '#!/bin/sh\n"%s" "$@" | sed -e "s/@.*$//"\n' "${real_nm}" > "${build}/shim/nm"
  chmod +x "${build}/shim/nm"
  PATH="${build}/shim:${PATH}"
  export PATH
fi

top_srcdir="${module}"
top_builddir="${build}"
export top_srcdir top_builddir
cd "${build}/test"
VERBOSE=1 sh "${top_srcdir}/test/run-suites"
exit_code=$?

if [[ -n "${SKIP_EXIT_CODE:-}" && "${exit_code}" == "${SKIP_EXIT_CODE}" ]]; then
  echo "SKIP: exited ${exit_code}, which the project's own test harness reports as skipped"
  exit 0
fi
exit "${exit_code}"
