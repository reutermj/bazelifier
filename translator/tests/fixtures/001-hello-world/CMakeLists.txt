cmake_minimum_required(VERSION 3.20)
# VERSION forces the explicit LANGUAGES form — `project(p VERSION 1.2.3
# CXX)` is a CMake error. Also the only fixture whose project() declares a
# VERSION, so this is what exercises read_project_version and the
# MODULE.bazel version line against real CMake output — see
# docs/lore/cmake-file-api-generated-source-shape.md's "How to check this
# sort of thing".
project(hello_world VERSION 1.0.0 LANGUAGES CXX)

add_executable(hello src/main.cpp)
