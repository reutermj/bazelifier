#include <stdio.h>

/* Textual include of a SOURCE file, by a relative path escaping this
 * directory. Gives the test access to `scale`, which is static and so
 * unreachable by linking. CMake does not list this file in impl-test's
 * sources — it must be staged, not compiled separately. */
#include "../src/impl.c"

int main(void) {
    printf("scale=%d public=%d\n", scale(5), impl_public_value());
    return 0;
}
