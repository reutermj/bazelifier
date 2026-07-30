#include <stdio.h>

#include "feature.h"

/* Uses FEATURE_GUARD, whose value comes from the unresolved @VAR@. Until the
 * escalation is resolved (a values entry in the generated config_header) the
 * header carries a literal @FEATURE_INCLUDE_GUARD@ and this source will not
 * compile at all — which is the point: the gap must surface, not slip through. */
int main(void) {
    printf("guard=%d\n", FEATURE_GUARD);
    return 0;
}
