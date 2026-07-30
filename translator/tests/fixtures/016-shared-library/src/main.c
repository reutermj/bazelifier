#include <stdio.h>

/* Prototype declared here rather than in a shared header, so this fixture
 * tests only the shared-library ground-truth path (bzl-fxa.11) and not the
 * header-visibility classification gap. greet_value lives in libgreet.so; the
 * ground-truth binary loads it at runtime, so if the .so isn't staged/found
 * the binary exits 127 and never prints and the comparison fails. */
int greet_value(void);

int main(void) {
    printf("value=%d\n", greet_value());
    return 0;
}
