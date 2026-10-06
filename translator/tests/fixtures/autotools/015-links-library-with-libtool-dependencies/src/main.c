#include <stdio.h>

int useuse_value(void);

int main(void) {
    printf("app2 says: value %d\n", useuse_value());
    return useuse_value() == 15 ? 0 : 1;
}
