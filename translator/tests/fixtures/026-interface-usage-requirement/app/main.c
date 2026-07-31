#include <stdio.h>

/* Reachable only because `iface` puts pub/ on this target's include path.
 * `iface` is INTERFACE, so it exists in no codemodel target and creates no
 * dependency edge — the include dir arrives with a target_link_libraries
 * backtrace and nothing to attribute it to. */
#include "pub.h"

int main(void) {
    printf("value=%d\n", pub_value());
    return 0;
}
