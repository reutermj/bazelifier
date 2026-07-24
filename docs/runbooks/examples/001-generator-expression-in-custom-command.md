<!--
Example runbook. This is illustrative, not tied to a real translator run —
it exists to show the format filled in with plausible content. See
docs/runbooks/README.md.
-->

# Runbook: generator expression in custom command output path

- **Status:** resolved
- **Source project:** examples/unit-fixtures/custom-command-genexpr
- **Source location:** `CMakeLists.txt:14`
- **Translator stage:** frontend parsing (custom command handling)

## Gap

The translator found an `add_custom_command()` whose `OUTPUT` path depends
on a generator expression conditioned on build configuration:

```cmake
add_custom_command(
  OUTPUT "$<$<CONFIG:Debug>:debug_>manifest.bin"
  COMMAND generate_manifest --out "$<$<CONFIG:Debug>:debug_>manifest.bin"
  DEPENDS manifest_src.json
)
```

The translator can evaluate simple generator expressions when the
configuration is fixed, but this project doesn't pin a single CMake build
type ahead of time, so the frontend can't determine a single output path to
generate a Bazel rule for.

## Context

- This custom command feeds into target `manifest_lib` (a `cc_library`)
  which `#include`s the generated `manifest.bin` via a generated header.
- The project supports both `Debug` and `Release` CMake configs, but the
  Bazel conversion target for this fixture is a single, non-configurable
  build (no `-c dbg`/`-c opt` distinction planned yet for this fixture).
- `manifest_src.json` is a static, non-generated source file already mapped
  to a Bazel source file.

## What was tried

The translator's generator-expression evaluator handles `$<CONFIG:...>`
only when a single target config is known; here it bailed out rather than
guessing between `manifest.bin` and `debug_manifest.bin`, since picking the
wrong one silently would produce a working-looking but wrong genrule.

## Expected output

A decision on which output path this fixture should use under Bazel (given
Bazel's own `-c dbg`/`-c opt` distinction isn't wired into this conversion
yet), plus a `genrule` fragment producing it, e.g.:

```python
genrule(
    name = "generate_manifest",
    srcs = ["manifest_src.json"],
    outs = ["manifest.bin"],
    cmd = "$(location //tools:generate_manifest) --out $@",
    tools = ["//tools:generate_manifest"],
)
```

## Resolution

Resolved by treating this fixture as always corresponding to CMake's
`Release` config (no `debug_` prefix), since the fixture doesn't yet model
Bazel compilation modes. Added a genrule producing `manifest.bin` directly
(as shown in Expected output).

Follow-up: once the translator needs to support CMake configs that map to
Bazel's `-c dbg`/`-c opt`, this will need a `select()` over
`//command_line_option:compilation_mode` (or equivalent) instead of a fixed
output path. Left as a known limitation rather than solved here — tracked
informally until it blocks a real fixture.
