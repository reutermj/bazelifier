cmake_minimum_required(VERSION 3.20)
project(subdir_binary_shared_library_fixture VERSION 5.2.0 LANGUAGES C)

# The capability under test is bzl-0x7 — the INTERSECTION of fixture 016
# (a SHARED library, whose .so chain must be staged for the ground-truth run)
# and fixture 018 (a binary built under a SUBDIRECTORY). Each of those passes
# on its own; only together do they reproduce the bug.
#
# copy_ground_truth_artifacts stages the .so chain at the ground_truth/ ROOT,
# but a subdirectory target's binary lands at ground_truth/app/app. The
# comparison script used to derive LD_LIBRARY_PATH from the BINARY's own
# directory, which is ground_truth/app/ here — where no .so lives. The loader
# then fails before main and the ground-truth run exits 127, so the comparison
# reports an exit-code mismatch rather than anything about the conversion.
#
# This is json-c's exact shape: libjson-c.so.5 at the root, apps/json_parse in
# a subdirectory.
#
# DO NOT flatten the subdirectory or link greet statically to make this pass:
# the combination is the case under test, and this file is immutable test
# input.

add_library(greet SHARED src/greet.c)
set_target_properties(greet PROPERTIES SOVERSION 5 VERSION 5.2.0)
target_include_directories(greet PUBLIC src)

add_subdirectory(app)
