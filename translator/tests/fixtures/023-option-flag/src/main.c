#include "config.h"
#include <stdio.h>

int main(void)
{
#ifdef ENABLE_GREETING
    puts("greeting on");
#else
    puts("greeting off");
#endif
    return 0;
}
