#include <stdio.h>

/* helper.h is beside this file. Nothing declares lib/ as an include
 * directory — the quoted form of #include searches this file's own directory
 * first, which is why CMake needs no declaration and Bazel needs the header
 * staged anyway. */
#include "helper.h"

int main(void) {
    printf("value=%d\n", helper_value());
    return 0;
}
