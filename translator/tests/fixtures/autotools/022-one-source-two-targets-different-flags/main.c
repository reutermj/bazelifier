#include <stdio.h>
int variant_1(void);
int variant_2(void);
int main(void) {
    printf("one %d two %d\n", variant_1(), variant_2());
    return 0;
}
