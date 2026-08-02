#include <config.h>
#include <mystring.h>
#include <stdio.h>

int main(void)
{
    printf("%s %d\n", MYSTRING_VERSION, HELPER_ADD(1));
    return 0;
}
