cmake_minimum_required(VERSION 3.23)
project(with_library CXX)

add_library(greet src/greet.cpp)
target_sources(greet
  PUBLIC
    FILE_SET public_headers TYPE HEADERS
    BASE_DIRS include
    FILES include/greet.hpp
)
target_include_directories(greet PUBLIC include)

add_executable(hello src/main.cpp)
target_link_libraries(hello PRIVATE greet)
