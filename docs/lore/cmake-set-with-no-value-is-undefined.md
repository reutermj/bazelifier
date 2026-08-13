# `set(VAR)` with no value produces `#undef`, not `#define`

CMake's `set(VAR)` **unsets** the variable. For `#cmakedefine` that means
`/* #undef VAR */`, which is the opposite of what the line looks like it does.

Verified against real CMake rather than inferred:

```cmake
set(EMPTY_SET)
set(SET_TO_ONE 1)
```
```c
/* #undef EMPTY_SET */
#define SET_TO_ONE
```

## Why it matters to a translator

It is a trap for anyone reasoning about `#cmakedefine` from `CMakeLists.txt`
*text*. `set(HAVE_FOO)` reads as an assertion — the surrounding comment often
says as much — and behaves as an omission. Only the generated header says
which.

This is the CMake instance of the rule the whole frontend is built on: read
the build system's RESOLVED output, never its input. `cmake_api.rs` reads the
File API and `configure_file.rs` reads the trace plus the real template, so
neither is fooled. The hazard is for a human (or an agent) resolving an
escalation by reading the project's CMake and reasoning about intent.

The failure is quiet in both directions:

- Resolve it as `#define VAR 1` because the comment says the feature exists,
  and the module's header disagrees with the one the project's own build
  produces.
- The runtime comparison may still pass, because a compatibility fallback
  usually computes the same answer by another route. So the check that
  should catch it does not.

## The live instance: json-c's `apps/`

`apps/` is dual-mode — buildable inside json-c or standalone against an
already-installed json-c of unknown vintage:

```cmake
if ("${PROJECT_NAME}" STREQUAL "json-c")
    # We know we have this in our current sources:
    set(HAVE_JSON_TOKENER_GET_PARSE_END)          # ← asserts NOTHING
else()
    check_symbol_exists(json_tokener_get_parse_end "json_tokener.h"
                        HAVE_JSON_TOKENER_GET_PARSE_END)
endif()
```

The probe in the standalone branch is sound: an older installed json-c may
genuinely lack the function. The in-tree branch intends to skip the probe
because the answer is known, and gets `#undef`.

Consequence, confirmed in the ground truth CMake produced
(`/* #undef HAVE_JSON_TOKENER_GET_PARSE_END */`): `apps/json_parse.c` takes
its fallback

```c
#define json_tokener_get_parse_end(tok) ((tok)->char_offset)
```

reaching into the struct directly — which `json_tokener.h:110` marks
**deprecated**, pointing at `json_tokener_get_parse_end()`, the very function
at `json_tokener.h:137` that is present the whole time.

Recorded as a project note that ships into json-c's converted module —
`translator/project_notes/json-c/001-set-with-no-value-defeats-a-feature-assertion.md`
— which carries both halves: what the agent must do here (leave it
undefined) and the fix json-c should take upstream. For the CONVERSION the
correct answer is still `#undef`, because a conversion reproduces what the
project's build does, not what it meant to do.
