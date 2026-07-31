/* No separate public header: like fixture 016, this keeps the fixture on the
 * shared-library ground-truth path and off the header-visibility
 * classification gap (fixture 003), which is a different capability. The
 * caller declares the prototype itself. */
int greet_value(void) {
    return 42;
}
