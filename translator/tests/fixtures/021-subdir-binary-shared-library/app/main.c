#include <stdio.h>

/* Lives in a subdirectory (target `app`, built to `app/app`) but links
 * libgreet.so from the parent. greet_value resolves at LOAD time, so if the
 * ground-truth run can't find the .so — staged at ground_truth/ while this
 * binary sits at ground_truth/app/ — the binary exits 127 without printing
 * and the comparison reports an exit-code mismatch. Prototype declared here
 * rather than in a shared header, for the reason in src/greet.c. */
int greet_value(void);

int main(void) {
    printf("value=%d\n", greet_value());
    return 0;
}
