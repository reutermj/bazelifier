cmake_minimum_required(VERSION 3.20)
project(configure_file_fixture VERSION 3.7.0 LANGUAGES C)

# The capability under test: a configure_file-generated config header, the
# autoconf-style shape a huge class of C projects use. The translator must
# reproduce config.h Bazel-natively via a cc_config config_header rule
# (probes computed against the CONSUMER's toolchain), NOT bake this host's
# feature-detection into the module. See
# docs/architecture/configure-file-and-toolchain-probes.md.
#
# Every macro here is one the shared cc_config catalog covers, deliberately,
# so this fixture converts fully GREEN (no needs_attention). A project macro
# the catalog lacks (or a project-prefixed alias) escalates instead — that
# path is exercised elsewhere; this fixture proves the clean end-to-end wire.
#
# DO NOT resolve a red conversion by checking in a hand-written config.h:
# this file is immutable test input, and a configure_file config header is
# the normal case the translator must handle.

include(CheckIncludeFile)
include(CheckSymbolExists)
include(CheckTypeSize)

check_include_file("stdlib.h" HAVE_STDLIB_H)
check_include_file("string.h" HAVE_STRING_H)
check_symbol_exists(strdup "string.h" HAVE_STRDUP)
check_type_size("long" SIZEOF_LONG)

configure_file(config.h.in config.h)

add_executable(app src/main.c)
# The generated config.h lands in the build dir; the sources include it, so
# the build dir must be on the include path (as every configure_file project
# does).
target_include_directories(app PRIVATE ${PROJECT_BINARY_DIR})
