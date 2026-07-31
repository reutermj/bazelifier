#!/bin/sh
# Registered with CTest as `script_test`. A checked-in script in the SOURCE
# tree, not a built target — the direction that must ESCALATE, because the
# translator has no cc_binary to point an sh_test at. Mirrors json-c's
# tests/*.test, which source a helper and compare output against a checked-in
# .expected file; kept trivial here since what's under test is the
# translator's classification, not the script's own logic.
echo "script ok"
