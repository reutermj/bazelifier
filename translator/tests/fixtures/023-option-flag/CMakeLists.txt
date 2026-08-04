cmake_minimum_required(VERSION 3.20)
project(option_flag_fixture LANGUAGES C)

# The CMake counterpart to autotools/009-enable-flag, and the same capability
# gap (bzl-1p6): an option() is a BUILD CHOICE, not a fact about the host, so
# no cc_config probe can answer it -- there is no program whose success tells
# you whether someone passed -DENABLE_GREETING=OFF.
#
# The CMake side fails DIFFERENTLY from the Autotools side, and worse. An
# autoconf --enable- macro escalates, so an agent at least sees it. An
# option() resolves silently from the cache, so today it is frozen into the
# generated module as `values = {"ENABLE_GREETING": "ON"}` with nothing
# recording that it was ever a choice. The conversion decides for every
# consumer and says nothing.
#
# Deliberately ON, so the default build prints "greeting on" and the ground
# truth captures that. -DENABLE_GREETING=OFF is reachable natively, which is
# what makes a wrong conversion FAIL the comparison rather than pass quietly.
#
# DO NOT resolve a red conversion by hardcoding the macro or flipping this
# option: it is immutable test input, and the point is that the option must
# survive conversion as an option.
option(ENABLE_GREETING "Include the greeting" ON)

configure_file(config.h.in config.h)

add_executable(app src/main.c)
target_include_directories(app PRIVATE ${PROJECT_BINARY_DIR})
