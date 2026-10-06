#include <stdio.h>
/* Root-relative: resolves only through -iquote$(top_srcdir), not beside
   this file. */
#include "src/defs/answer.h"

int main(void) {
    printf("answer %d\n", ANSWER);
    return 0;
}
