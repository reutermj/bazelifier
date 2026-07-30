#include <stdio.h>

#include "feature.h"

/* Deliberately does NOT expand FEATURE_GUARD: the point of this fixture is the
 * ESCALATION for the unresolved @VAR@ (bzl-fxa.9), which fires at conversion
 * time regardless of this source. Expanding the literal @FEATURE_INCLUDE_GUARD@
 * would make the module fail to *compile*, which (unlike 003/005) would abort
 * the whole comparison suite's build instead of letting the needs_attention
 * gate report the open item. So the module stays compilable and RED via the
 * gate — the agent resolves the escalation (a values entry) and only then does
 * anything downstream use the macro. */
int main(void) {
    printf("feature fixture\n");
    return 0;
}
