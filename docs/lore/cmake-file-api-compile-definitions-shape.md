# What the File API reports for compile definitions (and what it hides)

## What we hit

tinyxml2 (the first real project, see the corpus work) declares
`target_compile_definitions(tinyxml2 PUBLIC _FILE_OFFSET_BITS=64 ...)`.
The translator's `CompileGroup` deserializes only `includes`
(`cmake_api.rs`), so those defines are *silently* dropped — no escalation,
just a `cc_library` compiled without them, which the equivalence check
could then miss if the difference doesn't surface at runtime. Before
writing the fix, probing a real File API reply (CMake 3.28.3 + Ninja) to
see the actual shape turned up the part that isn't in the docs and would
have made a naive implementation wrong.

## What's actually true

**Defines live at `compileGroups[].defines[]`, shaped `{define,
backtrace}`** — `define` is the full `NAME` or `NAME=VALUE` string, sorted,
already de-duplicated:

```json
"defines": [
  { "backtrace": 2, "define": "PRIV_DEF=2" },
  { "backtrace": 2, "define": "PUB_DEF=1" }
]
```

So a `defines: Vec<CompileGroupDefine>` field with `#[serde(default)]` (a
target with none omits the key) reads them out. That part is trivial.

**The trap: the File API has already resolved and flattened away the
PUBLIC / PRIVATE / INTERFACE origin.** It reports the defines *effective on
each target's own compile line*, not who declared them or how they
propagate. Concretely, for `lib` with
`target_compile_definitions(lib PUBLIC PUB_DEF=1 INTERFACE IFACE_DEF PRIVATE PRIV_DEF=2)`
and an `app` that links `lib` PRIVATE:

| Target      | defines in its `compileGroups`      |
| ----------- | ----------------------------------- |
| `lib`       | `PRIV_DEF=2`, `PUB_DEF=1`           |
| `app`       | `IFACE_DEF`, `PUB_DEF=1`            |

Read this carefully, because it's the whole point:

- On `lib`, its PUBLIC (`PUB_DEF`) and PRIVATE (`PRIV_DEF`) defines are
  **indistinguishable** — both just appear. `IFACE_DEF` (INTERFACE, never
  compiled into lib) is absent.
- The PUBLIC/PRIVATE split is only recoverable *externally*: `PUB_DEF`
  reappears on `app`'s compile line (it propagated), `PRIV_DEF` does not.

**Consequence for codegen.** Bazel's `cc_library` distinguishes
`local_defines` (compile this target only) from `defines` (propagate to
consumers). The File API hands you neither label — only the flattened
effective set per target. To emit that distinction you must reconstruct it
by tracing each define's `backtrace` through `backtraceGraph` to the
command that introduced it, exactly as `own_include_dirs` /
`is_inherited_via_link_libraries` already do for includes (which have the
identical "own vs inherited is invisible in the flat list" problem — see
`cmake_api.rs`). It is the same technique, not new research: two layers.

- **Layer A** — read `compileGroups[].defines`, emit everything as
  `local_defines` on the owning target. Conservative but correct for the
  target's *own* compilation; nothing propagates. This alone stops the
  silent drop (tinyxml2's `_FILE_OFFSET_BITS=64` reaches the compile line).
  A `PUBLIC` define would then be re-derived independently on each consumer
  from *its* compile group rather than propagated — redundant, not wrong,
  as long as every consumer is itself converted.
- **Layer B** — trace backtraces to split propagating (PUBLIC/INTERFACE)
  from private, emit `defines` vs `local_defines`. Needed for correctness
  when a consumer is *outside* the converted set, and to match CMake's
  actual propagation rather than approximate it.

## Generator expressions are resolved too — but only for the active config

tinyxml2 also has `$<$<CONFIG:Debug>:TINYXML2_DEBUG>` and
`$<$<BOOL:${BUILD_SHARED_LIBS}>:TINYXML2_IMPORT>`. The File API evaluates
these against the single configuration CMake was configured with, so a
`Debug`-only define simply won't appear in a default (`""`/`Release`)
reply, and a `BUILD_SHARED_LIBS`-gated one depends on that cache var. The
reply is therefore a projection at one config, not the conditional itself —
a `select()`-based translation would need multiple configured replies (one
per config) to even see the other branches, which the pipeline does not
currently capture. Worth stating in whatever escalation covers this so an
agent doesn't hunt for a `Debug` define that the reply structurally cannot
contain.

## How to check this sort of thing

Same recipe as
[cmake-file-api-generated-source-shape.md](cmake-file-api-generated-source-shape.md):
empty `<build>/.cmake/api/v1/query/codemodel-v2`, `cmake -G Ninja -B
<build> -S <src>`, read `<build>/.cmake/api/v1/reply/target-*.json`. To see
propagation you need at least two targets with a `target_link_libraries`
edge between them and defines at each visibility, then compare the
`defines` array on the library against the one on its consumer — the
difference is the propagation the flat reply is hiding.
