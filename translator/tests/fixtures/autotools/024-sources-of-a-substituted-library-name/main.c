#include <stdio.h>
int core(void);
int extra(void);
int main(void) { printf("core %d extra %d\n", core(), extra()); return 0; }
