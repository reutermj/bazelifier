# `HAVE_JSON_TOKENER_GET_PARSE_END` is undefined, and that is correct

**Applies to:** json-c 0.19 (`apps_config.h`)

Reading `apps/CMakeLists.txt` suggests this macro should be defined, and the
obvious resolution — "json-c clearly has `json_tokener_get_parse_end()`, it is
declared in `json_tokener.h`, so set it to 1" — is wrong.

`apps/` builds two ways. Standalone, against an installed json-c that may be
too old, it probes. In-tree it means to skip the probe because the answer is
known:

```cmake
# We know we have this in our current sources:
set(HAVE_JSON_TOKENER_GET_PARSE_END)
```

`set(VAR)` with **no value** unsets the variable, so `#cmakedefine` emits
`#undef`. The comment says one thing and the line does the opposite.

Ground truth, from the header json-c's own build produces:

```c
/* #undef HAVE_JSON_TOKENER_GET_PARSE_END */
```

**So leave it undefined.** `apps/json_parse.c` then uses its compatibility
fallback, which is what the project's own build does. Defining it would make
this module disagree with the project — and the disagreement would be
invisible, because the fallback computes the same value, so the runtime
comparison cannot tell them apart.

This is an upstream bug, reported in the bazelifier checkout under
`docs/recommended-resolutions/001-json-c-apps-set-with-no-value.md`. If json-c
fixes it and the pin moves, ground truth changes and this note stops applying.
