cmake_minimum_required(VERSION 3.20)
project(nondeterministic_stderr_fixture LANGUAGES C)

# The capability under test is bzl-fxa.12: the equivalence comparison must
# tolerate a program that emits NONDETERMINISTIC output (a diagnostic that
# differs run-to-run), the way json-c's json_parse prints "maxrss: <N> KB" to
# stderr. app here prints its PID to stderr, which differs on every exec.
#
# compare_runtime_output.sh runs each binary twice and excludes exactly the
# lines that vary between a binary's own two runs, so the PID line drops out
# while the deterministic stdout ("parsed 3 objects") and the stable stderr
# line ("done") must still match between the ground-truth and Bazel builds.
# The check is NOT globally weakened: a fully deterministic fixture has nothing
# excluded (see docs/architecture/build-verification.md).
#
# DO NOT resolve a comparison failure by making app deterministic (dropping the
# PID line): nondeterministic diagnostics are the case under test, and this
# file — with its source — is immutable test input.

add_executable(app src/main.c)
