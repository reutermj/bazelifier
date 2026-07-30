#include <stdio.h>

#include "config.h"

/* Prints which options resolved to defined. If the translator/expander let a
 * false-token option (ENABLE_RDRAND=OFF) define the macro instead of undef'ing
 * it, rdrand would print 1 and the ground-truth comparison would fail. */
int main(void) {
#ifdef ENABLE_RDRAND
    int rdrand = 1;
#else
    int rdrand = 0;
#endif
#ifdef ENABLE_FEATURE
    int feature = 1;
#else
    int feature = 0;
#endif
    printf("rdrand=%d feature=%d\n", rdrand, feature);
    return 0;
}
