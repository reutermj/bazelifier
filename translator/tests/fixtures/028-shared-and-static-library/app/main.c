#include <stdio.h>

#include "greet.h"

/* Both consumers print the same thing on purpose: the runtime comparison
 * cannot see linkage, which is exactly why the fixture also asserts on it. */
int main(void) {
    printf("value=%d\n", greet_value());
    return 0;
}
