cmake_minimum_required(VERSION 3.20)
project(header_via_include_path LANGUAGES C)

# The capability under test is bzl-fxa.18: a target #includes a header it does
# NOT list as a source, reachable only through one of its own include
# directories. CMake needs no declaration — it compiles app.c and the
# preprocessor finds helpers/util.h on disk via -I. Bazel stages only DECLARED
# inputs into the sandbox, so without the fix the same header is absent at
# compile time and the build fails with
#
#   src/app.c:5:10: fatal error: 'util.h' file not found
#
# even though the -I flag is present and correct and the header was copied
# into the module.
#
# This is json-c's `test1` shape: add_executable(${TESTNAME} ${TESTNAME}.c)
# lists a single .c, and parse_flags.h is reachable only via
# target_include_directories(${TESTNAME} PUBLIC ${CMAKE_CURRENT_LIST_DIR}).
#
# helpers/ also holds unused.h, which nothing includes: it must NOT be swept
# into the module. "Copy every header under every include dir" would make the
# module a mirror of the source tree, which is what the referenced-files-only
# contract exists to prevent — so the fixture pins that direction too.
#
# DO NOT resolve a failure here by adding helpers/util.h to add_executable's
# source list: a header found via the include path is the case under test, and
# this file is immutable test input.

add_executable(app src/app.c)
target_include_directories(app PRIVATE helpers)
