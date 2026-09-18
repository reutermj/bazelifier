#include <stdio.h>
#include "greet.h"

int main(void) {
    printf("%s (%d)\n", greet_text(), greet_value());
    return 0;
}
