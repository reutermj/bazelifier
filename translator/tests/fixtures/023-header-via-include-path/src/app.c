#include <stdio.h>

/* util.h is NOT in this target's source list — it is reachable only through
 * target_include_directories(app PRIVATE helpers). That is the case under
 * test: CMake finds it on disk, Bazel must be told to stage it. */
#include "util.h"

int main(void) {
    printf("value=%d\n", util_value());
    return 0;
}
