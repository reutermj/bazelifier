#pragma once

/* Declared here, defined in helper.c — so the fixture also proves the header
 * is staged for a real separate translation unit, not just header-only. */
int helper_value(void);
