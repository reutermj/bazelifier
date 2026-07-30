#include <cstdio>

#include "defs.hpp"

// The executable sees PUBLIC and INTERFACE defines (propagated from `defs`),
// but NOT PRIVATE_VALUE. As in defs.cpp, each is guarded so a missing define
// changes the printed output rather than breaking compilation.
int main() {
#ifdef PUBLIC_VALUE
    int pub = PUBLIC_VALUE;
#else
    int pub = 0;
#endif
#ifdef INTERFACE_FLAG
    int iface = 1;
#else
    int iface = 0;
#endif
    // PRIVATE_VALUE must NOT be visible here — if it is, the translator
    // over-propagated a private define, and this prints priv=1 instead of 0.
#ifdef PRIVATE_VALUE
    int priv_leaked = 1;
#else
    int priv_leaked = 0;
#endif
    std::printf("sum=%d pub=%d iface=%d priv_leaked=%d\n", defs_sum(), pub, iface,
                priv_leaked);
    return 0;
}
