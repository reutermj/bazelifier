# configure_file and toolchain probes

How the translator handles CMake's `configure_file`-generated config headers
— the autoconf-style `config.h` that a huge class of C projects compile
against. **Implemented** (bzl-fxa.2): fixture 012 covers the clean path, 013
the `#if`-consumed shape, 014 false-token values, and 015 the escalation when
a `@VAR@` can't be resolved. json-c exercises it on a real project. What
follows describes shipped behavior, not a proposal.

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
  `symbol` (declared by `headers`) compiles/links. Takes an optional
  `defines` (feature-test macros like `_GNU_SOURCE`): glibc gates a set of
  symbols (`vasprintf`, `strdup`, `uselocale`, ...) behind `_GNU_SOURCE`, and
  the autoconf projects the catalog serves set it globally, so the catalog's
  symbol probes compile under it — otherwise a gated symbol probes absent and
  the project's `*_compat.h` defines a colliding fallback (bzl-fxa.7). A probe
  with `defines` is still one shared target, so once-per-toolchain sharing
  holds. See [../lore/gnu-source-gated-symbols-differ-by-toolchain.md](../lore/gnu-source-gated-symbols-differ-by-toolchain.md).
- `check_type_size(name, type)` — emits the type's size for the target ABI.

Each is a real Bazel rule that resolves the C/C++ toolchain (via
`rules_cc`'s `find_cc_toolchain`) and runs a compile or link action against
it, exactly as CMake's probes do, but against *Bazel's* toolchain. The
mechanics are proven: the `@llvm` toolchain already ships a working
autoconf-in-Bazel implementation for its own libstdc++ build (compile/link
probes that capture the compiler's exit status as a `true`/`false` result
file rather than failing the action). We **build `cc_config` fresh**, using
that only as a correctness reference for the `cc_common`/toolchain API — not
depending on it, since it is a private, per-GCC-version, libstdc++-scoped
subsystem of another module. See
[../lore/llvm-toolchain-ships-autoconf-probes.md](../lore/llvm-toolchain-ships-autoconf-probes.md).

The generated module consumes it: its `MODULE.bazel` gets a
`bazel_dep(name = "cc_config", ...)`, and its `BUILD.bazel` wires the
project's own `.in`/`.cmakein` templates through the probe rules to produce
`config.h` (substituting `#cmakedefine`/`@VAR@` from probe results) and
feeds that into the `cc_library`.

### Run once per toolchain, not once per project

The load-bearing property. If the probe for `HAVE_ENDIAN_H` is a **single
shared target** in `cc_config` (e.g. `@cc_config//probes:have_endian_h`),
then every converted project that needs it references the *same* target,
Bazel builds its action once per configuration, and the probe runs **once
per toolchain across the entire build graph** — not N times for N projects
that happen to need the same fact.

**Sharing is at the target level, deliberately — not action-level dedup.**
The tempting alternative is "on-demand": let each converted project *declare*
its own `check_include_file(name = "have_endian_h", ...)` in its own
generated `BUILD.bazel`, no catalog to maintain. That does not share. Two
such targets live in different packages (`@json_c//…`, `@tinyxml2//…`) and
each must declare its own output file, whose path includes the package name;
if the probe's compile action writes to that per-package output, the two
actions have different command lines, different action keys, and Bazel runs
the probe twice. Action-level dedup could be recovered by splitting the
shared compile from the per-package copy, but that is extra rule machinery
buying back a property target-level sharing gives for free.

So probes are **shared targets in `cc_config`**, referenced (not
redeclared) by converted projects. `cc_config` ships a fixed, hand-written
set of the common autoconf facts (see "Settled during design"); the
maintenance concern is bounded because that set — the same
`HAVE_<header>`/`HAVE_<symbol>`/`SIZEOF_<type>` catalog autoconf and CMake's
own modules enumerate — covers the large majority of what projects check,
and an uncovered fact is a one-line addition the escalation can point to.

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

The substitution runs at build time (it reads the probe *result files*), in
`cc_config`'s `config_header` rule. It is a small Python helper
(`expand_config_header.py`): `@VAR@`/`${VAR}` from the values map, and
`#cmakedefine`/`#cmakedefine01` resolved from probe results (a directive is
defined when a probe returned true or its name has a non-empty value,
matching CMake). Python — run via `aspect_rules_py`, which unlike stock
`rules_python`'s `py_binary` works as an exec-config build tool — was chosen
over shell/awk for legibility as the directive set grows; the cost, weighed
and accepted, is that a hermetic Python interpreter becomes a build-time
dependency every converted module using `config_header` inherits.

## What a template references (from json-c's ~48 macros + its `@VAR@`s)

Three kinds, by where the value comes from — see "Non-probe substitutions":

- **Probe-derived** (`#cmakedefine`) — computed against the consumer's
  toolchain by `cc_config`:
  - **`HAVE_<header>`** (~20) — `check_include_file`.
  - **`HAVE_<symbol>` / `HAVE_DECL_<x>`** (~15) — `check_symbol_exists`.
  - **`SIZEOF_<type>`** (~6) — `check_type_size`.
- **Option-derived** — CMake `option()` user choices (`ENABLE_RDRAND`,
  `ENABLE_THREADING`); a Bazel config knob or fixed default, not a probe.
- **Cache-value** (`@VAR@`) — plain CMake variables, chiefly version
  strings (json.h's `@JSON_C_MAJOR_VERSION@` etc.); substituted from values
  the translator reads at conversion time, toolchain-independent.

## What exists now

This is its own subsystem, not a translator tweak. All of it has landed:

1. The **`cc_config` module** (local to this repo — see Ownership below for
   why it isn't published yet): probe rules (`check_include_file`,
   `check_symbol_exists`, `check_type_size`), toolchain resolution, the
   shared-target sharing model, and `config_header`, which expands a template
   from probe results plus a translator-supplied value map. `probe_alias`
   republishes one probe's result under a second macro name, for a project
   that stamps a catalog fact into a project-prefixed define
   (json-c's `JSON_C_HAVE_INTTYPES_H` from `HAVE_INTTYPES_H`).
2. **Synthetic fixtures**, one per shape: 012 (a probe `#cmakedefine` and a
   plain `@VAR@`), 013 (`#cmakedefine FOO @FOO@` consumed by `#if FOO`),
   014 (CMake-false option tokens normalized to empty), 015 (an unresolved
   `@VAR@`, which must escalate rather than ship a literal `@NAME@`).
3. **Translator codegen**: `configure_file` calls recovered from the
   configure trace, macros mapped to catalog probes, the `bazel_dep`, the
   templates copied, and the `config_header` rules wired into consumers.
4. **json-c as the integration proof** — its library, static library and
   `json_parse` compile hermetically under the module's own toolchain with no
   host-captured config, and match the CMake build's runtime output.

The catalog is a fixed, hand-written set (see below). A project naming a fact
it lacks escalates rather than being guessed at; extending the catalog means
editing `cc_config/catalog/BUILD.bazel` **and** `CATALOG_DEFINES` in
`cmake_api.rs`, which `//:catalog_sync_check` enforces.

## Ownership

`cc_config` is a module **bazelifier itself publishes**, and every converted
project depends on it (`bazel_dep`) — a shared dependency is what makes
once-per-toolchain real; a vendored-per-module copy would re-probe per
project. The generated modules therefore gain a genuine dependency beyond
`rules_cc`/`llvm`, worth noting in bazel-codegen.md when this lands.

For now `cc_config` lives **inside this repo** (a local module, e.g. under
`cc_config/` with a `local_path_override` the same way the validation
workspace wires fixtures), not a separately-released artifact. Converted
modules in the validation tarball reference the local copy; publishing it as
a standalone versioned module is a later step, once its rule surface has
settled.

## Non-probe substitutions

`configure_file` substitutes more than feature-detection results. Besides
`#cmakedefine` (probes) and CMake `option()`s, templates carry `@VAR@`
references to plain CMake variables — version strings (`@PROJECT_VERSION@`,
json-c's `@JSON_C_MAJOR_VERSION@`), computed values, and the like — and
these **must be handled too**, not deferred: json.h itself is generated this
way, and a config header with an unsubstituted `@VERSION@` is as broken as
one missing a `HAVE_` define.

These values are simpler than probes — they are known at conversion time
from the CMake cache (the translator already reads `cache-v2` for
`CMAKE_PROJECT_VERSION` and `CMAKE_ROOT`), so the substitution is a
straight value lookup, not a toolchain probe. The design splits template
variables into three kinds: **probe-derived** (`#cmakedefine HAVE_*` →
`cc_config` probes, computed against the consumer's toolchain),
**option-derived** (CMake `option()` user choices → a Bazel config knob or
fixed default), and **cache-value** (`@VAR@` plain variables → substituted
from values the translator captures at conversion time, since they do not
depend on the consumer's toolchain). Enumerating exactly which cache
variables real projects reference — beyond the versions json-c needs — is
the part to firm up while building this, but plain-variable substitution is
in scope from the start, not a follow-on.

## Settled during design

**Catalog form: a fixed, hand-written broad set in `cc_config`, not a
generated one.** `cc_config` ships probe targets for the common autoconf
facts — the `HAVE_<header>`/`HAVE_<symbol>`/`SIZEOF_<type>` set that
autoconf and CMake's own modules already enumerate — which covers the
overwhelming majority of what real projects check. Generating the catalog
from the corpus's needs would couple the translator to mutating a
checked-in module for a payoff that isn't there at this scale; it stays a
later option if the fixed set proves a bottleneck (YAGNI now). A project
needing a fact the set doesn't cover is a one-line addition to `cc_config`,
and the escalation for an unhandled config macro can name exactly which line
to add.

**Cache-value substitution is generic, not an enumerated allowlist.**
`configure_file` substitutes `@VAR@` from whatever CMake variable of that
name exists, so the translator does the same: look the name up in the
captured cache and substitute its value. Version strings are just the common
case, not a special one — a generic lookup handles them and anything else a
template references without a curated list to maintain.
