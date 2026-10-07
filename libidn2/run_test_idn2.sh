#!/usr/bin/env bash
# Runs tests/test-idn2.sh as automake's driver does: from a writable tests/
# directory, with srcdir set, and IDN2 naming the built tool (its default,
# ../src/idn2, is a build-tree path). Not `set -e`: the script's status is
# the result.
set -uo pipefail
module="$(cd "$(dirname "$0")" && pwd)"
work="${TEST_TMPDIR:-$(mktemp -d)}/tests"
mkdir -p "${work}"
srcdir="${module}/tests"
IDN2="${module}/idn2"
export srcdir IDN2
cd "${work}"
sh "${srcdir}/test-idn2.sh"
exit_code=$?
if [[ -n "${SKIP_EXIT_CODE:-}" && "${exit_code}" == "${SKIP_EXIT_CODE}" ]]; then
  echo "SKIP: exited ${exit_code}, which the project's own test harness reports as skipped"
  exit 0
fi
exit "${exit_code}"
