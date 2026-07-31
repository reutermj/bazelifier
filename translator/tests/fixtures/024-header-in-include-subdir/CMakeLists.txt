cmake_minimum_required(VERSION 3.20)
project(header_in_include_subdir LANGUAGES C)

# The capability under test is bzl-htv: a header reached through an include
# directory but living in a SUBDIRECTORY of it. `-Iinclude` with
# `#include "proj/util.h"` is ordinary C — the header is at
# include/proj/util.h — and a flat listing of include/ misses it entirely.
#
# This is fixture 023's shape one level deeper, and it is a separate fixture
# because the failure is indistinguishable from 023's: the same
#
#   src/app.c:6:10: fatal error: 'proj/util.h' file not found
#
# that 023 exists to prevent, with 023's fix already applied. Without a
# fixture at this depth, the next person to hit it reads the lore entry, sees
# the fix is present, and has nowhere to go.
#
# DO NOT flatten include/proj/util.h to include/util.h to make this pass: the
# nesting is the case under test, and this file is immutable test input.

add_executable(app src/app.c)
target_include_directories(app PRIVATE include)
