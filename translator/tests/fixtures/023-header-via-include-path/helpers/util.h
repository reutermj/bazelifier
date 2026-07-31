#pragma once

/* Header-only on purpose: the fixture is about the header being STAGED, not
 * about linking, so there is no util.c to add to srcs and muddy what failed. */
static inline int util_value(void) {
    return 42;
}
