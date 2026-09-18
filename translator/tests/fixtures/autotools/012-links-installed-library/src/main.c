#include <stdio.h>
#include <greet.h>

int main(void) {
    printf("app says: %s, value %d\n", greet_text(), greet_value() * 6);
    return greet_value() == 7 ? 0 : 1;
}
