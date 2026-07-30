cmake_minimum_required(VERSION 3.20)
project(install_declared_header CXX)

# The capability under test: a library declares its public header via
# install(FILES ... TYPE INCLUDE) — the pre-FILE_SET way, used by a huge
# fraction of real C++ projects (tinyxml2 among them). The header is a plain
# source with NO target_sources FILE_SET, so the ONLY signal that it is
# public is the install rule. The translator must read that rule and put the
# header in the generated cc_library's `hdrs`, not `srcs`, and NOT escalate.
#
# The mirror is fixture 003-library-no-file-set: same shape (header in srcs,
# no FILE_SET) but NO install rule, so it stays genuinely ambiguous and
# escalates. The two fixtures differ only in whether an install rule names
# the header — that is exactly what this capability keys on. See
# docs/architecture/cmake-frontend.md.
#
# DO NOT resolve a red conversion by adding a FILE_SET here: this file is
# immutable test input, and install()-declared public headers are the normal
# case the translator must handle.

include(GNUInstallDirs)

add_library(mathlib src/mathlib.cpp include/mathlib.h)
target_include_directories(mathlib PUBLIC include)

# The public-header declaration — no FILE_SET anywhere.
install(
    FILES include/mathlib.h
    TYPE INCLUDE
)
install(TARGETS mathlib)

# A consumer that #includes the public header, so mathlib is depended-on
# (the header-visibility escalation only fires for a depended-on library).
add_executable(app src/main.cpp)
target_link_libraries(app PRIVATE mathlib)
