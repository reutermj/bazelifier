cmake_minimum_required(VERSION 3.20)
project(generated_source CXX)

# app consumes a source add_custom_command() produces at build time, not
# one checked into the project tree. The translator has no way to
# translate the recipe that produces it (a shell command here, but in
# general anything), so it escalates via needs_attention/ instead of
# guessing — see docs/architecture/cmake-frontend.md and
# docs/lore/cmake-file-api-generated-source-shape.md (which also explains
# the "gen.cpp.rule" phantom sibling CMake reports alongside the real
# output, and why that name must not also show up in the escalation).
#
# generated_answer() is deliberately NOT called from main.cpp: like
# 005-unsupported-target-type's dropped dependency edge, the point under
# test is that the needs_attention/ gate is the ONLY thing failing this
# fixture. If main.cpp referenced generated_answer(), the Bazel-generated
# cc_binary (missing gen.cpp from srcs) would fail to LINK, which would
# also fail the comparison test's own `data` dependency on the binary
# before the gate even got a chance to run — conflating "the escalation
# fired" with "the conversion happened to break for an unrelated reason".
#
# DO NOT resolve this by checking gen.cpp into the project or inlining its
# contents into main.cpp. This file is immutable test input: a project
# with a generated source is the normal case the translator has to cope
# with, not a defect in this fixture. Resolution belongs in the GENERATED
# BUILD.bazel (a genrule producing gen.cpp, wired into app's srcs).
add_custom_command(
    OUTPUT gen.cpp
    COMMAND ${CMAKE_COMMAND} -E echo "int generated_answer() { return 42; }" > gen.cpp
    VERBATIM
)

add_executable(app src/main.cpp gen.cpp)
