#include <config.h>
#include <stdio.h>

int main(void)
{
#ifdef HAVE_GREETING
    puts("greeting on");
#else
    puts("greeting off");
#endif
    return 0;
}
