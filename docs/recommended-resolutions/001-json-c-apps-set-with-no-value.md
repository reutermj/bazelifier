# json-c: `set()` with no value defeats an in-tree feature assertion

- **Project:** json-c
- **Version examined:** `25602128ed580416222601b4da5a50cdb1bc2a58` (tag `json-c-0.19-20260627`)
- **File:** `apps/CMakeLists.txt:20`
- **Impact:** `apps/json_parse.c` silently uses a deprecated fallback in every
  in-tree build, including release builds.
- **Found by:** converting json-c to Bazel; the macro escalated as unresolvable
  and reading the ground truth showed why.

## What happens

`apps/` is dual-mode — it builds either as part of json-c or standalone
against an already-installed json-c, which may be older and lack recent
functions. It branches accordingly:

```cmake
if ("${PROJECT_NAME}" STREQUAL "json-c")
# Part of an overall json-c build
set(APPS_LINK_LIBS "${PROJECT_NAME}")

# We know we have this in our current sources:
set(HAVE_JSON_TOKENER_GET_PARSE_END)

else()
...
check_symbol_exists(json_tokener_get_parse_end "json_tokener.h"
                    HAVE_JSON_TOKENER_GET_PARSE_END)
```

The standalone probe is correct. The in-tree branch means to skip it because
the answer is already known — but `set(VAR)` with no value **unsets** `VAR`,
so `#cmakedefine HAVE_JSON_TOKENER_GET_PARSE_END` in
`apps/cmake/apps_config.h.in` expands to `#undef`.

Confirmed in the header an ordinary in-tree build produces:

```c
/* #undef HAVE_JSON_TOKENER_GET_PARSE_END */
```

And confirmed as general CMake behaviour, not something specific to this
project:

```cmake
set(EMPTY_SET)      #  ->  /* #undef EMPTY_SET */
set(SET_TO_ONE 1)   #  ->  #define SET_TO_ONE
```

## Why it matters

`apps/json_parse.c:49` then takes its compatibility path:

```c
#ifndef HAVE_JSON_TOKENER_GET_PARSE_END
#define json_tokener_get_parse_end(tok) ((tok)->char_offset)
#endif
```

which reaches into `struct json_tokener` directly. `json_tokener.h:110`
marks that access **deprecated**, pointing readers at
`json_tokener_get_parse_end()` — declared at `json_tokener.h:137` and
present the entire time.

So the in-tree build uses the deprecated route the project is actively
steering users away from, in the one configuration where the modern function
is guaranteed to exist. It is silent: the fallback computes the same value
today, so nothing fails and nothing warns. It stops being silent if
`char_offset` ever moves or the struct becomes opaque, which is presumably
why the accessor exists.

## Suggested fix

Give the variable a value, so `#cmakedefine` sees it as set:

```cmake
set(HAVE_JSON_TOKENER_GET_PARSE_END 1)
```

`#cmakedefine01` would be an alternative if a 0/1 value is wanted in the
header, but the consumer only tests `#ifndef`, so plain `set(... 1)` is the
smaller change.

## Note for this repo

**The conversion still emits `#undef`, and that is correct.** A conversion
reproduces what the project's build system does, not what it intended. If
json-c fixes this and we re-pin, the converted module follows automatically.

The agent-facing half of this finding ships inside the converted module as
`project_notes/001-set-with-no-value-defeats-a-feature-assertion.md`
(source: `translator/project_notes/json-c/`). Same fact, different audience:
this file asks json-c to fix it, that one stops an agent "fixing" it in the
module — where the wrong answer compiles, runs, and passes the runtime
comparison because the fallback computes the same value.

If json-c takes the fix, retire both together.

The CMake footgun behind it is recorded in
[docs/lore/cmake-set-with-no-value-is-undefined.md](../lore/cmake-set-with-no-value-is-undefined.md).
