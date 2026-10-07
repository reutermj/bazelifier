#include <stdio.h>
#include "feature.h"
int main(void) {
#ifdef FEATURE
    puts("feature on");
#else
    puts("feature off");
#endif
    return 0;
}
