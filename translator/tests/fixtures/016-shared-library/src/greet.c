/* No separate public header: this fixture isolates the SHARED-LIBRARY
 * ground-truth path (bzl-fxa.11). A public header would additionally trip the
 * header-visibility classification gap (see fixture 003), which is a different
 * capability; the caller declares the prototype itself instead. */
int greet_value(void) {
    return 42;
}
