#include <stdio.h>

/* An EXAMPLE, not a test. If this ever appears as a registered test of this
 * module, the per-directory scoping of automake's am__ indirection has
 * regressed -- examples/ declares no TESTS. */
int main(void) {
    printf("demo_example ok\n");
    return 0;
}
