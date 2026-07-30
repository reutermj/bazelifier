#include <stdio.h>

#include "config.h"

/* Prints values derived from the generated config header, so the ground-truth
 * comparison fails if the config header is missing or wrong (a define absent,
 * a value unsubstituted). Guarded with #ifdef so a missing define changes the
 * output rather than breaking compilation. */
int main(void) {
#ifdef HAVE_STDLIB_H
    int have_stdlib = 1;
#else
    int have_stdlib = 0;
#endif
#ifdef HAVE_STRDUP
    int have_strdup = 1;
#else
    int have_strdup = 0;
#endif
    printf("stdlib=%d strdup=%d sizeof_long=%d version=%s\n", have_stdlib,
           have_strdup, SIZEOF_LONG, APP_VERSION);
    return 0;
}
