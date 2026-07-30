cmake_minimum_required(VERSION 3.23)
project(compile_definitions CXX)

# The capability under test: target_compile_definitions at each of the
# three visibilities. The translator's CompileGroup used to deserialize
# only includes, so every one of these was dropped SILENTLY — no
# escalation, just a cc_library compiled without them. This fixture makes
# each define OBSERVABLE at runtime so the ground-truth comparison (stdout +
# exit code) fails if any define is missing from the generated build, not
# just if the compile happens to break.
#
# Layer A (the capability this fixture drives) emits every effective define
# as Bazel local_defines, which do NOT propagate — so the executable does
# not inherit PUB_DEF/IFACE_DEF through the deps edge. It gets its own copy
# because CMake already propagated them onto the executable's compile line,
# and the File API reports that flattened effective set per target. See
# docs/lore/cmake-file-api-compile-definitions-shape.md.
#
# DO NOT "fix" a red conversion by editing this file — it is immutable test
# input. Resolution belongs in the generated BUILD.bazel.

add_library(defs src/defs.cpp)
target_sources(defs
  PUBLIC
    FILE_SET public_headers TYPE HEADERS
    BASE_DIRS include
    FILES include/defs.hpp
)
target_include_directories(defs PUBLIC include)

target_compile_definitions(defs
  # Seen by both defs.cpp and any consumer that links defs.
  PUBLIC PUBLIC_VALUE=42
  # Seen only by consumers, never compiled into defs itself.
  INTERFACE INTERFACE_FLAG
  # Seen only by defs.cpp; a value with a name so codegen's NAME=VALUE
  # handling is exercised on the private side too.
  PRIVATE PRIVATE_VALUE=7
)

add_executable(app src/main.cpp)
target_link_libraries(app PRIVATE defs)
