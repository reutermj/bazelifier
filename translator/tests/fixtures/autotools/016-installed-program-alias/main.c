#include <stdio.h>
#include <string.h>

int main(int argc, char **argv) {
    (void)argc;
    /* Behaviour chosen by the name it was run as, as unxz and prterun are. */
    const char *slash = strrchr(argv[0], '/');
    const char *name = slash ? slash + 1 : argv[0];
    if (strcmp(name, "farewell") == 0) {
        puts("goodbye");
        return 3;
    }
    puts("hello");
    return 0;
}
