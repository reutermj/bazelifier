cmake_minimum_required(VERSION 3.20)
project(binary_private_include CXX)

# An executable with its OWN include directory and no dependencies —
# isolating the case where nothing else can supply the include path.
# 002-with-library only covers include dirs reaching a binary *transitively*
# through `deps`, which Bazel propagates for free; nothing propagates a
# target's own.
#
# `inc/cfg.hpp` is listed as an add_executable() source deliberately: that's
# what makes the CMake File API report it, so it lands in the generated
# cc_binary's `srcs` and becomes a real Bazel action input. Do NOT drop it
# from this list — a header no CMake target declares is invisible to the File
# API entirely, which is a different (and still unresolved) translator gap.
# Removing it would silently repoint this fixture at that gap and leave it
# permanently red.
add_executable(app src/main.cpp inc/cfg.hpp)
target_include_directories(app PRIVATE inc)
