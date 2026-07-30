cmake_minimum_required(VERSION 3.20)
project(configure_file_if_define_fixture VERSION 1.0.0 LANGUAGES C)

# The capability under test is the specific configure_file shape json-c uses
# and fixture 012 does not: a boolean check written as `#cmakedefine FOO @FOO@`
# and then evaluated with `#if FOO` (not `#ifdef`). That requires the boolean
# probe's RESULT to substitute its @FOO@ ("1" when the check passes, empty when
# it fails); if @FOO@ is left literal the `#if @FOO@` is a compile error, not a
# silently-wrong value. This regresses cc_config bzl-fxa.6. See
# docs/architecture/configure-file-and-toolchain-probes.md.
#
# Both macros are cc_config catalog facts (one true on the build toolchain, one
# false), so this converts fully GREEN with no needs_attention.
#
# DO NOT resolve a red conversion by checking in a hand-written config.h or by
# rewriting these `#if`s as `#ifdef`: this file is immutable test input, and the
# `#cmakedefine FOO @FOO@` / `#if FOO` pattern is the normal case json-c and
# many autoconf-derived projects use.

include(CheckIncludeFile)

check_include_file("stdlib.h" HAVE_STDLIB_H)
check_include_file("xlocale.h" HAVE_XLOCALE_H)

configure_file(config.h.in config.h)

add_executable(app src/main.c)
target_include_directories(app PRIVATE ${PROJECT_BINARY_DIR})
