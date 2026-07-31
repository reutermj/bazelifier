#pragma once

/* On app's include path but included by nothing. Must NOT be pulled into the
 * module: the injection is driven by what sources actually #include, not by
 * sweeping the include directories. */
static inline int unused_value(void) {
    return 7;
}
