#include <stdio.h>
#include "subconf.h"

int main(void) {
#ifdef HAVE_UNISTD_H
    puts("unistd yes");
#endif
    printf("answer %d\n", SUB_ANSWER);
    return 0;
}
