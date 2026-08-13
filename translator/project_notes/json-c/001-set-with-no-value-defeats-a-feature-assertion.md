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

## Upstream

This is a bug in json-c, and the section is here rather than in a separate
report so the fix and the fact cannot drift apart — they are about the same
line, and json-c changing it invalidates both at once.

- **Version examined:** `25602128ed580416222601b4da5a50cdb1bc2a58`
  (tag `json-c-0.19-20260627`)
- **File:** `apps/CMakeLists.txt:20`

`apps/` is dual-mode: standalone it probes for the function, because an
installed json-c may be too old to have it; in-tree it means to skip the
probe because the answer is known. The in-tree branch writes `set(VAR)` with
no value, which unsets rather than sets, so the probe is skipped AND the
macro is left undefined.

`apps/json_parse.c:49` then takes its compatibility path and reads
`struct json_tokener`'s `char_offset` directly — which `json_tokener.h:110`
marks **deprecated**, pointing at `json_tokener_get_parse_end()`, declared at
`json_tokener.h:137` and present the whole time. So the in-tree build uses
the deprecated route in the one configuration where the modern function is
guaranteed. Silent today, because the fallback computes the same value.

Suggested fix, giving the variable a value so `#cmakedefine` sees it as set:

```cmake
set(HAVE_JSON_TOKENER_GET_PARSE_END 1)
```

**This does not change what the conversion should do.** A conversion
reproduces what a build DOES, not what it meant to. If json-c takes the fix
and the pin moves, ground truth changes and this whole note stops applying —
retire it then, not before.
