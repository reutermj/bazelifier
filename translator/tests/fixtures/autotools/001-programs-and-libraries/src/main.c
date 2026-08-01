#include <stdio.h>

#include "greet.h"

int shout_value(void);

int main(void) {
    printf("greet=%d shout=%d\n", greet_value(), shout_value());
    return 0;
}
