cmake_minimum_required(VERSION 3.20)
project(gnu_source_symbol_fixture LANGUAGES C)

# The capability under test is bzl-fxa.7: a symbol glibc gates behind
# _GNU_SOURCE (here vasprintf) must be detected as PRESENT so the project does not
# define a colliding fallback. This is exactly json-c's failure: HAVE_VASPRINTF
# came out false because the shared catalog probe compiled a bare snippet
# without _GNU_SOURCE, so vasprintf_compat.h defined its own vasprintf and
# collided with glibc's. The catalog's symbol probes now compile under
# _GNU_SOURCE (probe.bzl / catalog.bzl), so HAVE_VASPRINTF resolves true on the
# hermetic llvm/libc++ toolchain the converted module builds with.
#
# check_symbol_exists is run with _GNU_SOURCE the way json-c's own CMake does,
# via CMAKE_REQUIRED_DEFINITIONS, so the CMake ground-truth build agrees.
#
# DO NOT resolve a failure by pinning HAVE_VASPRINTF or dropping the fallback:
# correct feature detection under the consumer's real feature-test macros is
# the point, and this file (with config.h.in and its source) is immutable input.

include(CheckSymbolExists)
set(CMAKE_REQUIRED_DEFINITIONS -D_GNU_SOURCE)
check_symbol_exists(vasprintf "stdio.h" HAVE_VASPRINTF)

configure_file(config.h.in config.h)

add_executable(app src/main.c)
target_include_directories(app PRIVATE ${PROJECT_BINARY_DIR})
