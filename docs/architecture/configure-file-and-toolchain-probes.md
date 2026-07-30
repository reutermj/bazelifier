# configure_file and toolchain probes

How the translator will handle CMake's `configure_file`-generated config
headers — the autoconf-style `config.h` that a huge class of C projects
compile against. **This is a design, not yet implemented** (tracked in
bzl-fxa.2). It records the decision and the shape so the implementation
doesn't have to re-derive them.

See [../lore/cmake-configure-file-generated-headers.md](../lore/cmake-configure-file-generated-headers.md)
for how these headers surface in the File API (two ways, one silent), which
is the input to this design.

## The problem

json-c (the second corpus project) generates `config.h`, `json_config.h`,
and `json.h` via `configure_file`, driven by ~60 feature-detection probes
(`check_include_file`, `check_symbol_exists`, `check_type_size`). Its `.c`
sources `#include "config.h"` and branch on ~48 distinct macros —
`HAVE_ENDIAN_H`, `HAVE_GETRANDOM`, `SIZEOF_LONG`, `HAVE_STRINGS_H`, and so
on. The templates are almost entirely `#cmakedefine HAVE_<x>` lines; the
values are **facts about the target platform and toolchain**.

The generated Bazel module must build hermetically and — the point that
decides everything below — must be **portable to any consumer's toolchain**,
because the whole product is standalone modules other people build with
their own setup.

## Decision: Bazel-native, not host-capture

The translator *runs* CMake, so at conversion time the resolved
`config.h` already exists in its build directory. Copying it into the module
would be trivial. **We do not do that.** Those values are *this build
machine's* feature-detection results; baking them into the deliverable makes
the module build correctly only where it was converted and silently wrong
elsewhere (wrong `SIZEOF_LONG` on a 32-bit target, a missing `HAVE_*` that
the consumer's libc actually has). That is the opposite of the hermetic,
portable module the rest of the pipeline guarantees.

So the config header's values are determined **against the consumer's own
toolchain**, the same toolchain the rest of the generated module compiles
with — computed at the consumer's `bazel build` time, not captured from
ours.

Bazel has no built-in equivalent of autoconf's compile-probes, so we build
one.

## Architecture: a shared probing module (`cc_config`)

A new, standalone Bazel module — working name `cc_config` — provides the
autoconf primitives as Bazel rules:

- `check_include_file(name, header)` — succeeds iff `#include <header>`
  compiles under the resolved toolchain.
- `check_symbol_exists(name, symbol, headers)` — iff a reference to
  `symbol` (declared by `headers`) compiles/links.
- `check_type_size(name, type)` — emits the type's size for the target ABI.

Each is a real Bazel rule that resolves the C/C++ toolchain (via
`rules_cc`'s `find_cc_toolchain` — confirmed available) and runs a compile
or link action against it, exactly as CMake's probes do, but against
*Bazel's* toolchain. A rule can run these actions; this is mechanically
feasible today.

The generated module consumes it: its `MODULE.bazel` gets a
`bazel_dep(name = "cc_config", ...)`, and its `BUILD.bazel` wires the
project's own `.in`/`.cmakein` templates through the probe rules to produce
`config.h` (substituting `#cmakedefine`/`@VAR@` from probe results) and
feeds that into the `cc_library`.

### Run once per toolchain, not once per project

The load-bearing property. If the probe for `HAVE_ENDIAN_H` is a **single
shared target** in `cc_config` (e.g. `@cc_config//probes:have_endian_h`),
then every converted project that needs it references the *same* target.
Bazel builds an action once per configuration and caches it, so the probe
runs **once per toolchain across the entire build graph** — not N times for
N projects that happen to need the same fact. This falls out of Bazel's
action caching *provided the probes are shared targets*, not per-project
macro expansions that each synthesize their own action. Designing for shared
probe targets (a fixed catalog of common probes, or interned by
header/symbol name) is therefore a requirement, not an optimization.

## What the translator must do

Two layers, and the translator side is the smaller one:

1. **Detect the config headers.** They do NOT arrive flagged `isGenerated`
   (see the lore doc). A header the project lists as a target source shows up
   as an absolute path under the CMake build directory — recognize that
   *before* the generic sources-outside-deliverable check, which currently
   misdiagnoses it (bzl-fxa.3). A header that is only `#include`d
   (json-c's `config.h`) never appears in the target reply at all and needs
   finding by other means (the `#include` directive whose name resolves only
   under the build dir).
2. **Generate the wiring.** Emit the `bazel_dep(cc_config)`, copy the
   project's template files into the module (the `.in`/`.cmakein`, which
   *are* in the source tree — unlike their outputs), and generate the
   `BUILD.bazel` rules that turn templates + probe results into the config
   header and route it into the library.

The probe results → `#cmakedefine` substitution is template expansion over a
`{macro: value}` map a probe rule provides; `expand_template` or a small
custom rule reading a provider.

## Macro categories (from json-c's ~48)

- **`HAVE_<header>`** (~20) — `check_include_file`.
- **`HAVE_<symbol>` / `HAVE_DECL_<x>`** (~15) — `check_symbol_exists`.
- **`SIZEOF_<type>`** (~6) — `check_type_size`.
- **`ENABLE_<feature>` / options** — these are CMake `option()`s (user
  choices: `ENABLE_RDRAND`, `ENABLE_THREADING`), not probes. Map to a Bazel
  config knob or a fixed conservative default; do not probe for them.

## Scope and sequencing

This is large — its own subsystem, not a translator tweak. Expected order:

1. This design doc, reviewed.
2. The `cc_config` module: the probe rules, toolchain resolution, the
   shared-target sharing model, and template substitution — with its own
   tests (a probe for a header that exists and one that doesn't; the
   once-per-toolchain sharing demonstrated across two consumers).
3. A **synthetic** `configure_file` fixture (a template + one probe + a
   source that includes the output) — the focused driver, per the repo's
   "a capability isn't finished until a fixture exercises it," covering both
   the source-listed and include-only header shapes.
4. Translator codegen for detection + wiring.
5. **json-c as the integration proof** — its library compiles hermetically,
   under the module's own toolchain, with no host-captured config.

## Open questions

**Open question:** probe catalog vs. on-demand. A fixed catalog of common
probes shares cleanly but must be maintained; interning arbitrary
header/symbol names by string keeps sharing without a catalog but needs the
rule machinery to dedup targets. Decide when building `cc_config`.

**Open question:** `cc_config` ownership. Is it a module bazelifier
publishes and every converted project depends on (a real external
dependency of the deliverables), or vendored per module? A shared dep is
what makes once-per-toolchain real; a vendored copy re-probes per project.
The sharing requirement argues for a published shared module — which means
the generated modules gain a genuine third-party `bazel_dep` beyond
`rules_cc`/`llvm`, worth stating in bazel-codegen.md when this lands.

**Open question:** non-probe substitutions. `@VAR@` values that are neither
probes nor options (version strings, paths) — enumerate what real projects
need beyond json-c before generalizing the substitution.
