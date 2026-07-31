cmake_minimum_required(VERSION 3.20)
project(ctest_script_command LANGUAGES C)

# The capability under test is bzl-fxa.16: a CTest test whose command is NOT
# an executable the translator builds. This is json-c's shape — all 28 of its
# tests run a checked-in shell script from the SOURCE tree
# (`<src>/tests/test1.test`) rather than a built binary, and CTest reports the
# script's absolute path as the command.
#
# Both shapes are here on purpose, because the escalation has to distinguish
# them and a fixture with only the failing one can't show that:
#
#   `app_test`    runs the `app` executable       -> translated to an sh_test
#   `script_test` runs tests/check.sh (a script)  -> escalated, no sh_test
#
# Before the fix, the translator reduced every command to its BASENAME and
# emitted `sh_test(data = [":check.sh"])` — a label that silently doesn't
# resolve, because the file is at `tests/check.sh` and, more fundamentally,
# is not a target at all. That failed only at analysis time in the unpacked
# workspace, with `missing input file`, saying nothing about the construct
# that wasn't understood.
#
# Note this fixture is EXPECTED to emit a needs_attention item and therefore
# STARTS RED, like fixtures 003/005/015 — that is the escalation half of the
# pipeline working, and the agent stage closing the item is what turns it
# green. See CLAUDE.md on red fixtures.
#
# DO NOT rewrite check.sh as an add_executable to make this translate: a
# script-driven test is the case under test, and this file is immutable test
# input.

enable_testing()

add_executable(app src/main.c)

add_test(NAME app_test COMMAND app)

add_test(NAME script_test COMMAND ${CMAKE_CURRENT_SOURCE_DIR}/tests/check.sh)
