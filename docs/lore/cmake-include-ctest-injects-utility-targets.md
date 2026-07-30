# `include(CTest)` injects ~28 phantom UTILITY targets

## What we hit

Ingesting tinyxml2 as the first corpus project (bzl-c54.4) produced a
conversion with **29 `needs_attention` items** — 28 of them
`unsupported_target_needs_attention` for targets named `Continuous`,
`Experimental`, `Nightly`, and every combination of those with
`Start`/`Update`/`Configure`/`Build`/`Test`/`Coverage`/`MemCheck`/`Submit`
(plus `NightlyMemoryCheck`). Only the 29th — the header-visibility
escalation on `tinyxml2.h` — was a real gap. The other 28 drowned it.

## What's actually true

These come from a single line in tinyxml2's root `CMakeLists.txt`:

```cmake
include(CTest)
```

`include(CTest)` doesn't just enable testing — it pulls in CTest's
**dashboard model** (the CDash client), which calls `add_custom_target()`
for each step of the Nightly/Continuous/Experimental dashboard workflows.
Every one of those is a CMake `UTILITY` target: a named build step with no
compiled artifact. They appear in the codemodel-v2 reply exactly like any
other target, so the translator escalated each as an unsupported type.

Three properties make them recognizable as noise rather than work:

- **type `UTILITY`** (from `add_custom_target`),
- **no artifacts** (they build nothing on disk),
- **nothing depends on them** (they're top-level developer conveniences).

That triad — UTILITY + no artifacts + no dependents — is the general shape
of a developer-convenience target, and it is *not* CTest-specific: Doxygen
(`doc`), clang-format (`format`), and similar modules inject the same
shape. So the handler keys on the shape, not on CTest's specific target
names (which would be a brittle allowlist). It splits by *provenance*,
though — because the shape alone can't tell CMake's injected targets from a
project's own hand-written `doc`/`format` target, and the two want different
handling: an inert target whose defining command is under `CMAKE_ROOT` (a
CMake module injected it) is dropped **silently**, while an inert target the
project itself defined is **aggregated** into one escalation, so the drop is
a decision rather than an oversight. Either way, a UTILITY target that *does*
produce a consumed file (has artifacts, or has dependents) is real and still
escalates individually — see bzl-c54.6 and `is_cmake_provided`/`is_inert_target`.

`enable_testing()` alone does NOT do this; it only turns on `add_test()`
registration. It's specifically `include(CTest)` (or `include(Dart)`, its
predecessor) that adds the dashboard targets. A project that wants
`add_test()` without the dashboard clutter uses `enable_testing()` directly
— which is why tinyxml2's *own* `test/CMakeLists.txt` (a standalone
sub-project) uses `enable_testing()` and does not spawn these, while the
root, which uses `include(CTest)`, does.

## How to check this sort of thing

Same File API recipe as the sibling lore docs. To see the injected
targets, configure a project whose `CMakeLists.txt` has `include(CTest)`
and list the `target-*.json` replies: the dashboard targets show up as
`"type": "UTILITY"` with empty `artifacts` and no inbound `dependencies`
from other targets. Contrast a second configure with `include(CTest)`
commented out (or replaced by bare `enable_testing()`) — the ~28 targets
vanish, confirming the source.
