#include <config.h>
#include <stdio.h>

int main(void)
{
    /* Arithmetic, not #ifdef: the macro is always defined, and a 1 where
       the build had 0 is the failure this program exists to show. */
    printf("debug=%d\n", FLAGVALUES_ENABLE_DEBUG);
#ifdef HAVE_EXTRA
    puts("extra on");
#else
    puts("extra off");
#endif
#ifdef HAVE_GREETING
    puts("greeting on");
#else
    puts("greeting off");
#endif
    return 0;
}
