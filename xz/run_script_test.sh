#!/usr/bin/env bash
# Runs one of xz's automake TESTS scripts in the build-tree layout it
# hardcodes. The scripts find xz as ../src/xz/xz, read ../config.h to decide
# what to skip, write temporaries into the working directory, and locate
# their data through $srcdir — automake's test driver supplies all four, so
# this does too. Not `set -e`: the script's exit status is the result.
set -uo pipefail

script="$1"

# The module's runfiles root, from $0 rather than TEST_WORKSPACE: as a
# dependency the module lives under runfiles/<canonical repo>/, a name it
# cannot know (same reasoning as run_registered_test.sh).
module="$(cd "$(dirname "$0")" && pwd)"

build="${TEST_TMPDIR:-$(mktemp -d)}/build"
mkdir -p "${build}/tests" "${build}/src/xz" "${build}/src/xzdec" "${build}/src/scripts"
# Symlinked, not copied: a binary linked against a cc_shared_library finds
# it through an $ORIGIN RUNPATH, which the loader resolves from the real
# path, so a link keeps it working where a copy would not.
ln -sf "${module}/xz" "${build}/src/xz/xz"
ln -sf "${module}/xzdec" "${build}/src/xzdec/xzdec"
ln -sf "${module}/create_compress_files" "${build}/tests/create_compress_files"
cp "${module}/src/scripts/xzdiff" "${module}/src/scripts/xzgrep" "${build}/src/scripts/"
chmod +x "${build}/src/scripts/"*
cp "${module}/config.h" "${build}/config.h"

srcdir="${module}/tests"
export srcdir
cd "${build}/tests"
sh "${srcdir}/${script}"
exit_code=$?

if [[ -n "${SKIP_EXIT_CODE:-}" && "${exit_code}" == "${SKIP_EXIT_CODE}" ]]; then
  echo "SKIP: exited ${exit_code}, which the project's own test harness reports as skipped"
  exit 0
fi
exit "${exit_code}"
