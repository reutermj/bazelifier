cmake_minimum_required(VERSION 3.20)
project(sibling_sources CXX)

# This project compiles sources from a SIBLING directory that ships
# alongside it. `../shared/` is not inside the CMake project directory, but
# it is part of the same source deliverable — the fixture directory is the
# thing you would tarball, and it contains both.
#
# That distinction is the point of this fixture. What decides whether an
# input is a problem is not where it sits relative to the CMakeLists.txt,
# but whether it can be reproduced from what the project ships. These files
# can, so the conversion must handle them: the module root widens to cover
# both directories and nothing is escalated. See
# docs/architecture/cmake-frontend.md.
#
# DO NOT "fix" this by moving shared/ under proj/. The layout is the input
# shape under test, and it is how real projects that vendor a common
# directory across several builds are actually organised.
add_executable(app
  src/main.cpp
  ../shared/helper.cpp
  ../shared/helper.hpp
)
target_include_directories(app PRIVATE ../shared)
