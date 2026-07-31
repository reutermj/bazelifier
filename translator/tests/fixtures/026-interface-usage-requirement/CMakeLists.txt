cmake_minimum_required(VERSION 3.20)
project(interface_usage_requirement LANGUAGES C)

# The capability under test is bzl-2wi.3: an include directory that reaches a
# target through a target_link_libraries USAGE REQUIREMENT, with no dependency
# edge behind it.
#
# `iface` is an INTERFACE library — no sources, no artifact — so CMake
# resolves its requirements into the consumer at configure time and reports
# NOTHING for it: it appears in no codemodel target and in no target's
# dependencies[]. What the File API does report is `pub/` on app's include
# path, tagged with a target_link_libraries backtrace.
#
# The translator drops includes with that backtrace, on the reasoning that
# Bazel re-supplies them transitively through the deps edge. Here there is no
# such edge, so before the fix `pub/` was supplied by nothing and app failed
# with `fatal error: 'pub.h' file not found`.
#
# fmt's HEADER_ONLY tests are the live shape:
# target_link_libraries(unicode-test gtest fmt-header-only) reports
# dependencies: [gtest] alone.
#
# DO NOT replace iface with a static library to make this pass: an INTERFACE
# library carrying usage requirements is the case under test, and this file is
# immutable test input.

add_library(iface INTERFACE)
target_include_directories(iface INTERFACE pub)

add_executable(app app/main.c)
target_link_libraries(app iface)
