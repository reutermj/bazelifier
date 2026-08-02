/* Textually #included by table.c, never compiled on its own.
 *
 * Deliberately NOT a standalone translation unit: it opens no headers and
 * refers to ROW_COUNT, which only its includer defines. If the translator
 * ever puts this in `srcs`, Bazel compiles it separately and the build fails
 * here rather than silently producing a duplicate symbol — which is the
 * point of the fixture. */
static int rows[ROW_COUNT] = {1, 2, 3, 4};

static int sum_rows(void) {
  int total = 0;
  for (int i = 0; i < ROW_COUNT; i++) {
    total += rows[i];
  }
  return total;
}
