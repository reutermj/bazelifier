#include "impl.h"

/* Deliberately static: invisible to anything that links against this
 * translation unit, which is why the test includes the .c textually. */
static int scale(int value) {
    return value * 2;
}

int impl_public_value(void) {
    return scale(21);
}
