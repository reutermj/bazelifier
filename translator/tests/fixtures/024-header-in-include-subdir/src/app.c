#include <stdio.h>

/* Reached via -Iinclude, but nested one level below it. CMake finds it on
 * disk; the translator has to walk the include directory to stage it. */
#include "proj/util.h"

int main(void) {
    printf("value=%d\n", util_value());
    return 0;
}
