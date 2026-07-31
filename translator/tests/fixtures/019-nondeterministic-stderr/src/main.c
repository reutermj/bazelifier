#include <stdio.h>
#include <unistd.h>

/* Reproduces the shape of json-c's json_parse (bzl-fxa.12): a program that
 * prints a nondeterministic diagnostic to stderr alongside deterministic
 * output. json_parse prints "maxrss: <ru_maxrss> KB"; here it's the PID, which
 * differs on every exec — so the value differs between the binary's own two
 * runs AND between the ground-truth and Bazel builds. The comparison must
 * exclude that line (it varies run-to-run) while still requiring the stable
 * lines on stdout and stderr to match. */
int main(void) {
    /* Deterministic stdout — must match exactly. */
    printf("parsed 3 objects\n");
    /* Nondeterministic stderr line — excluded by the self-calibrating compare. */
    fprintf(stderr, "diag: pid=%d\n", (int)getpid());
    /* Deterministic stderr line — must still match. */
    fprintf(stderr, "done\n");
    return 0;
}
