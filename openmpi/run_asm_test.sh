#!/usr/bin/env bash
# Runs one of Open MPI's test/asm programs the way its make check does:
# TESTS_ENVIRONMENT = $(SHELL) $(srcdir)/run_tests, which runs the program
# once per thread count (1 2 4 5 8) and folds the results — any failure
# fails, otherwise a pass beats a skip. Reproduced rather than copied: the
# original parses automake's test-driver argv, which a Bazel test has none of.
set -uo pipefail
binary="$(cd "$(dirname "$0")" && pwd)/$1"
echo "--> Testing $1"
retval=-1
for threads in 1 2 4 5 8; do
    "${binary}" "${threads}"
    result=$?
    if [ "${result}" = "0" ]; then
        echo "    - ${threads} threads: Passed"
        [ "${retval}" -eq -1 ] && retval=0
    elif [ "${result}" = "77" ]; then
        echo "    - ${threads} threads: Skipped"
        [ "${retval}" -eq -1 ] && retval=77
    else
        echo "    - ${threads} threads: Failed"
        retval=${result}
    fi
done
exit "${retval}"
