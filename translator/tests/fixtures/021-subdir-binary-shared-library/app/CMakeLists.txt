# Built to `app/app` (build-relative) while linking libgreet.so from the
# parent directory, whose staged .so chain lands at the ground_truth/ root —
# one level up from this binary. That gap is bzl-0x7; see the parent
# CMakeLists.txt.
add_executable(app main.c)
target_link_libraries(app PRIVATE greet)
