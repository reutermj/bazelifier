cmake_minimum_required(VERSION 3.20)
project(sibling_header_no_include_dir LANGUAGES C)

# The capability under test is bzl-2wi.4: a source #includes a header sitting
# BESIDE it, and no include directory covers that location. The compiler finds
# it by the C/C++ quoted-include rule — `#include "x.h"` searches the
# INCLUDING FILE's own directory before any -I — which is a language rule, not
# a build setting, so it appears nowhere in the File API.
#
# Note there is deliberately NO target_include_directories here. Adding one
# would make fixture 023 cover this case; the whole point is that nothing
# declares lib/ at all.
#
# fmt's test/util.cc is the live shape: it does `#include "util.h"` with
# test/util.h beside it, listed on no target and covered by no include dir.
#
# DO NOT add target_include_directories(app PRIVATE lib) to make this pass:
# the absence of any declared include dir is the case under test, and this
# file is immutable test input.

add_executable(app lib/app.c lib/helper.c)
