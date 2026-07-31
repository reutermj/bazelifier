cmake_minimum_required(VERSION 3.20)
project(installed_header_fixture LANGUAGES C)

# The capability under test is bzl-fxa.10 + bzl-fxa.14: a public header the
# project install()s to an include destination but that NO target lists as a
# source. This is the common C shape — a target enumerates only its .c files
# and leaves headers ambient on the include path (see
# docs/architecture/cmake-frontend.md). CMake finds greet.h via -I at compile
# time; the File API never reports it as a target source, so without the fix
# the translator never copies it and the converted library fails to compile the
# moment greet.c #includes it. json-c is the real case (its CMakeLists even
# drops two headers from the library list by an ordering bug).
#
# Two things must work together:
#  - fxa.14: the install DESTINATION is the ABSOLUTE CMAKE_INSTALL_FULL_INCLUDEDIR
#    (.../include/greet), not a bare `include`, so is_include_destination must
#    recognize the absolute form.
#  - fxa.10: greet.h, being install-declared public but unenumerated, is
#    injected as a public header on the library whose include path it sits on.
#
# DO NOT resolve a failure by adding greet.h to add_library() or a FILE_SET:
# this file is immutable test input, and "headers are ambient on the include
# path" is exactly the case the translator must handle.

include(GNUInstallDirs)

add_library(greet greet.c)
target_include_directories(greet PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})

add_executable(app main.c)
target_link_libraries(app PRIVATE greet)

install(FILES greet.h DESTINATION ${CMAKE_INSTALL_FULL_INCLUDEDIR}/greet)
install(TARGETS greet app)
