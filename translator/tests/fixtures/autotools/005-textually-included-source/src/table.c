#include "table.h"

#define ROW_COUNT 4

/* The construct under test: a SOURCE file included textually. The included
 * file is not compiled separately, so it appears in no compile command and
 * in no _SOURCES — only this line states the dependency. */
#include "impl.c"

int table_sum(void) { return sum_rows(); }
