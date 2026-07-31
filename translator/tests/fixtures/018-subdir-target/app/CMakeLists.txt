# The executable lives in this subdirectory, so CMake builds it to `app/app`
# (build-relative) while its target name stays `app` — the mismatch bzl-fxa.13
# reconciles between the ground-truth export and the comparison-test label.
add_executable(app main.c)
