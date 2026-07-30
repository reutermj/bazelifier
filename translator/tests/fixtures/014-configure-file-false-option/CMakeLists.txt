cmake_minimum_required(VERSION 3.20)
project(configure_file_false_option_fixture LANGUAGES C)

# The capability under test is bzl-fxa.8: a CMake option left at a false token
# (OFF/FALSE/NO/0) reaches a config header as a #cmakedefine and must resolve to
# /* #undef */, exactly as CMake does -- NOT to a #define of the literal token.
# json-c hit this: ENABLE_RDRAND=OFF became `#define ENABLE_RDRAND OFF`, and
# code testing the macro then compiled the bare token `OFF`. The option's value
# comes from the CMake cache (an option() is a BOOL cache entry), so this
# exercises the translator's cache path plus the expander's CMake-truthiness.
#
# ENABLE_RDRAND is deliberately OFF and ENABLE_FEATURE deliberately ON, so the
# generated header undefs the first and defines the second; both are resolvable
# from the cache, so this converts fully GREEN with no needs_attention.
#
# DO NOT resolve a red conversion by flipping these options ON or hand-writing
# config.h: this file is immutable test input, and a config header gating on a
# disabled option is the normal case.

option(ENABLE_RDRAND "Use the RDRAND instruction" OFF)
option(ENABLE_FEATURE "Enable the feature" ON)

configure_file(config.h.in config.h)

add_executable(app src/main.c)
target_include_directories(app PRIVATE ${PROJECT_BINARY_DIR})
