cmake_minimum_required(VERSION 3.20)
project(library_no_file_set CXX)

# Deliberately does NOT use target_sources(... FILE_SET ... TYPE HEADERS
# ...) to declare greet.hpp as a public header. This fixture exercises
# bazelifier's needs_attention/ escalation for that gap — see
# docs/architecture/cmake-frontend.md.
add_library(greet src/greet.cpp src/greet.hpp)
target_include_directories(greet PUBLIC src)

add_executable(hello src/main.cpp)
target_link_libraries(hello PRIVATE greet)
