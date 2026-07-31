cmake_minimum_required(VERSION 3.20)
project(textually_included_source LANGUAGES C)

# The capability under test is bzl-2wi.5: a source that #includes another
# SOURCE file textually, rather than linking against it.
#
# `test/impl-test.c` does `#include "../src/impl.c"` so it can see and test
# static functions the translation unit does not export. CMake deliberately
# does NOT list src/impl.c among impl-test's sources — doing so would compile
# it a second time and produce duplicate symbols.
#
# Two things make this hard, and both are the point:
#   - The file is a .c, so no header-based staging rule reaches it.
#   - The path is RELATIVE and escapes the including file's own directory
#     (../src/), so treating test/ as an implicit include dir does not help.
# It is recovered from Ninja's dependency database — the compiler's own record
# of what it opened.
#
# In Bazel it cannot go in `srcs`: a .c there is a translation unit, so Bazel
# would compile it separately AND still not stage it for the compile that
# includes it. It belongs in `textual_hdrs` — "header files that cannot be
# compiled on their own" — which is a cc_library-only attribute, hence the
# generated companion library.
#
# DO NOT "fix" this by adding src/impl.c to impl-test's sources: that changes
# what the fixture tests and would break the real build with duplicate
# symbols. This file is immutable test input.

add_library(impl src/impl.c)
target_include_directories(impl PUBLIC src)

add_executable(impl-test test/impl-test.c)
