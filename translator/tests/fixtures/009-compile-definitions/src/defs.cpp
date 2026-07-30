#include "defs.hpp"

// PUBLIC and PRIVATE defines are both compiled into this translation unit.
// Guarded with #ifdef rather than used bare so a MISSING define changes the
// returned value (observable at runtime via the executable's output) instead
// of failing to compile — a compile failure would break the comparison
// test's own data dependency before the ground-truth check could run, and so
// couldn't distinguish "define dropped" from "build broke for another
// reason". See fixture 007's CMakeLists.txt for the same reasoning applied to
// a generated source.
int defs_sum() {
#ifdef PUBLIC_VALUE
    int pub = PUBLIC_VALUE;
#else
    int pub = 0;
#endif
#ifdef PRIVATE_VALUE
    int priv = PRIVATE_VALUE;
#else
    int priv = 0;
#endif
    return pub + priv;
}
