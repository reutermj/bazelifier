#define _GNU_SOURCE
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

#include "config.h"

/* Mirrors json-c's vasprintf_compat.h: when the config header reports vasprintf
 * ABSENT, define a fallback. Under _GNU_SOURCE glibc DOES declare vasprintf, so
 * the fallback collides ("static declaration follows non-static declaration")
 * and the build fails — exactly what a false HAVE_VASPRINTF produces. The
 * fixture only compiles when the probe correctly detected vasprintf present
 * (via _GNU_SOURCE). */
#ifndef HAVE_VASPRINTF
static int vasprintf(char **strp, const char *fmt, va_list ap) {
    (void)strp;
    (void)fmt;
    (void)ap;
    return -1;
}
#endif

/* vasprintf's third argument is a va_list, so it must be called through a
 * varargs function that builds one — not with a bare value. */
static int format(char **out, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = vasprintf(out, fmt, ap);
    va_end(ap);
    return n;
}

int main(void) {
    char *s = NULL;
    int n = format(&s, "n=%d", 7);
    if (n >= 0 && s) {
        printf("%s\n", s);
        free(s);
    } else {
        printf("vasprintf failed\n");
    }
    return 0;
}
