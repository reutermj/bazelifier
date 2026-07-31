#include <stdio.h>

/* A subdirectory executable (target name `app`, built to `app/app`). Prints a
 * line so the ground-truth comparison has output to match — the point of the
 * fixture is that the comparison target resolves at all for a subdir target. */
int main(void) {
    printf("hi from subdir\n");
    return 0;
}
