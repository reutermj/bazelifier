#include <stdio.h>

#include "greet.h"

/* Includes greet.h (the install-declared, unenumerated public header) and
 * calls into the library. Prints a value so the ground-truth comparison
 * exercises the linked code. */
int main(void) {
    printf("value=%d\n", greet_value());
    return 0;
}
