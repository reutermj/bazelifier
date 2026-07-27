cmake_minimum_required(VERSION 3.20)
project(unsupported_target_type CXX)

add_executable(app src/main.cpp)

# add_custom_target() produces a CMake UTILITY target. The translator has no
# Bazel rule for that type, so it escalates via needs_attention/ and carries
# on converting the rest of the project — one unrecognized target must not
# cost the project every other target it defines.
#
# add_dependencies() then makes `app` depend on it. That edge exists only to
# order the build; it contributes nothing to the binary. It is reported in
# the File API's `dependencies` list indistinguishably from a real link
# dependency, so the translator drops the edge rather than emitting a label
# pointing at a target it never generated.
#
# DO NOT resolve this by deleting the custom target. This file is immutable
# test input: a project full of developer-convenience utility targets is the
# normal case, and the translator has to cope with it. Resolution belongs in
# the GENERATED BUILD.bazel.
add_custom_target(gen_docs COMMAND ${CMAKE_COMMAND} -E echo "generating docs")
add_dependencies(app gen_docs)
