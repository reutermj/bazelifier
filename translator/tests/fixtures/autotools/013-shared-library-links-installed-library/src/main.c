#include <stdio.h>
#include "usegreet.h"

int main(void) {
    printf("app says: %s, value %d\n", usegreet_text(), usegreet_value());
    return usegreet_value() == 14 ? 0 : 1;
}
