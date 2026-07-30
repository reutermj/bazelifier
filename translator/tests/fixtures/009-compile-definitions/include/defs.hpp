#pragma once

// Returns a value the library computes from its own compile definitions
// (PUBLIC_VALUE + PRIVATE_VALUE). Declared here so the executable can call
// it; the executable also independently observes PUBLIC_VALUE and
// INTERFACE_FLAG on its own compile line (propagated from `defs`).
int defs_sum();
