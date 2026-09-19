#include <stdio.h>
#include <greet.h>
#include "usegreet.h"

static char buffer[128];

const char *usegreet_text(void) {
    snprintf(buffer, sizeof buffer, "%s and again %s", greet_text(), greet_text());
    return buffer;
}

int usegreet_value(void) {
    return greet_value() * 2;
}
