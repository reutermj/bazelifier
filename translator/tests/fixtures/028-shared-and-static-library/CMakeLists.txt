cmake_minimum_required(VERSION 3.20)
project(shared_and_static_library LANGUAGES C)

# The capability under test is bzl-i4i.1: a SHARED library, and specifically
# a project that builds the SAME SOURCES both ways with consumers linking
# each. That is zlib's shape (libz.so as target `zlib`, libz.a as
# `zlibstatic`), reduced to two consumers.
#
# CMake's File API reports SHARED_LIBRARY and STATIC_LIBRARY as distinct
# types. Both become a cc_library in Bazel — linkage is not a different rule
# — so a conversion that ignores the distinction still BUILDS and still
# produces identical runtime output.
#
# So be precise about what this fixture proves. It is the only REAL-CMake
# evidence that the File API reports the two types distinctly and that both
# forms convert, build and run. It does NOT check linkage: the ground-truth
# comparison sees stdout, and both consumers print the same thing, so a
# silent downgrade to static would pass here. The linkage assertion lives in
# codegen's unit tests; a fixture-tier check would need readelf (bzl-rqi).
#
# DO NOT "simplify" this to a single library: two libraries over the same
# sources, with one consumer each, is the case under test, and this file is
# immutable test input.

add_library(greet_shared SHARED lib/greet.c)
target_include_directories(greet_shared PUBLIC lib)

add_library(greet_static STATIC lib/greet.c)
target_include_directories(greet_static PUBLIC lib)

add_executable(app_shared app/main.c)
target_link_libraries(app_shared greet_shared)

add_executable(app_static app/main.c)
target_link_libraries(app_static greet_static)
