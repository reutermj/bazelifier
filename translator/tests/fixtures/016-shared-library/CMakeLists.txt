cmake_minimum_required(VERSION 3.20)
project(shared_library_fixture VERSION 5.2.0 LANGUAGES C)

# The capability under test is bzl-fxa.11: the FIRST fixture with a SHARED
# library. CMake builds libgreet.so (SONAME libgreet.so.5) and links app
# against it dynamically, baking an absolute RUNPATH into the build directory
# that is gone by the time the equivalence test runs. The ground-truth binary
# must still be able to load its .so at test time — copy_ground_truth_artifacts
# stages the versioned .so chain into ground_truth/ and the comparison test
# adds it to LD_LIBRARY_PATH. The Bazel side links the cc_library statically
# (cc_binary's default), so only the ground-truth binary needs the staging.
# See docs/architecture/build-verification.md.
#
# SOVERSION/VERSION give the real-world libgreet.so -> libgreet.so.5 ->
# libgreet.so.5.2.0 symlink chain, so the fixture exercises the SONAME (what
# the binary's DT_NEEDED actually names), not just the plain libgreet.so.
#
# DO NOT make this build statically to sidestep the .so handling: a shared
# library is the case under test, and this file is immutable test input.

add_library(greet SHARED src/greet.c)
set_target_properties(greet PROPERTIES SOVERSION 5 VERSION 5.2.0)

add_executable(app src/main.c)
target_link_libraries(app PRIVATE greet)
