cmake_minimum_required(VERSION 3.20)
project(subdir_target_fixture LANGUAGES C)

# The capability under test is bzl-fxa.13: a target CMake builds under a
# SUBDIRECTORY. add_subdirectory(app) + add_executable(app ...) gives the
# target the Bazel name `app` but a build-relative artifact path of `app/app`.
# The ground-truth artifact is exported under its path while the comparison
# test references it by target name, so without the fix the two disagree and
# the comparison fails at analysis (target //ground_truth:app not found). This
# is json-c's shape (apps/json_parse, tests/test1, ...).
#
# DO NOT flatten this to a single top-level add_executable to dodge the subdir:
# targets in subdirectories are the case under test, and this file is immutable
# test input.

add_subdirectory(app)
