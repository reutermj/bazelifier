#include <stdio.h>

#include "config.h"

/* Uses `#if` (not `#ifdef`) on the config-header macros, so the config header
 * must define HAVE_STDLIB_H to a numeric 1 (from the boolean probe's result),
 * not to a literal @HAVE_STDLIB_H@ — which would be a compile error here. This
 * is the json-c pattern that motivated cc_config bzl-fxa.6. The printed values
 * make the ground-truth comparison fail if a define is wrong. */
int main(void) {
#if HAVE_STDLIB_H
    int stdlib_branch = 1;
#else
    int stdlib_branch = 0;
#endif
#if HAVE_XLOCALE_H
    int xlocale_branch = 1;
#else
    int xlocale_branch = 0;
#endif
    printf("stdlib_branch=%d xlocale_branch=%d\n", stdlib_branch, xlocale_branch);
    return 0;
}
