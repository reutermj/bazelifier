#!/usr/bin/env bash
# Runs one of json-c's tests/<name>.test wrappers as ctest does, in the build
# layout test-defs.sh hardcodes: it appends /tests to $top_builddir and runs
# binaries from there, and test_json_parse_cli-style wrappers reach
# ../apps/json_parse. Not `set -e`: the wrapper's status is the result.
set -uo pipefail
name="$1"

# From $0, not TEST_WORKSPACE: as a dependency the module lives under
# runfiles/<canonical repo>/ (see run_registered_test.sh).
module="$(cd "$(dirname "$0")" && pwd)"

build="${TEST_TMPDIR:-$(mktemp -d)}/build"
mkdir -p "${build}/tests" "${build}/apps"
for f in "${module}"/*; do
  # Every built test binary sits at the module root; link the executables,
  # not the sources and scripts that share it.
  [[ -f "${f}" && -x "${f}" && "${f}" != *.sh ]] || continue
  ln -sf "${f}" "${build}/tests/$(basename "${f}")"
done
ln -sf "${module}/json_parse" "${build}/apps/json_parse"

srcdir="${module}/tests"
top_builddir="${build}"
VERBOSE=1
export srcdir top_builddir VERBOSE
cd "${build}/tests"
sh "${srcdir}/${name}.test"
