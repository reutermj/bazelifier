cmake_minimum_required(VERSION 3.20)
project(library_no_file_set CXX)

# Deliberately does NOT use target_sources(... FILE_SET ... TYPE HEADERS
# ...) to declare greet.hpp as a public header. This fixture exercises
# bazelifier's needs_attention/ escalation for that gap, and the agent
# stage that resolves it — see docs/architecture/cmake-frontend.md.
#
# DO NOT add a FILE_SET here. This file is immutable test input: the gap is
# the point. Resolving it upstream would delete the only coverage of this
# escalation path and teach the translator nothing about the next project
# shaped this way. Resolution belongs in the GENERATED BUILD.bazel.
add_library(greet src/greet.cpp src/greet.hpp)
target_include_directories(greet PUBLIC src)

add_executable(hello src/main.cpp)
target_link_libraries(hello PRIVATE greet)
