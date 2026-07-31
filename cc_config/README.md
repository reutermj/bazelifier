# cc_config

A local Bazel module providing autoconf-style **feature probes** that resolve
against the *consumer's* C/C++ toolchain, plus a rule that expands a CMake
`configure_file` template into a config header from those probe results.

It exists so a bazelifier-converted CMake project's generated config header
(`config.h`, `json_config.h`, ...) is computed for whoever builds the
converted module — not baked from the machine that ran the conversion. See
[../docs/architecture/configure-file-and-toolchain-probes.md](../docs/architecture/configure-file-and-toolchain-probes.md)
for the full design and
[../docs/lore/llvm-toolchain-ships-autoconf-probes.md](../docs/lore/llvm-toolchain-ships-autoconf-probes.md)
for the toolchain-probe mechanics.

## Rules (`cc_config/probe.bzl`, `cc_config/config_header.bzl`)

- `check_include_file(name, header, define)` — defines `define` iff
  `#include <header>` compiles (a compile probe).
- `check_symbol_exists(name, symbol, headers, define)` — defines `define` iff
  `symbol` links (a compile+link probe; a declared-but-undefined function
  fails).
- `check_type_size(name, type, headers, define)` — sets `define` to
  `sizeof(type)`, found at compile time (no execution, so it works
  cross-target).
- `probe_alias(name, probe, define)` — republishes an existing probe's result
  under a different macro name, adding no action. For a project that stamps a
  catalog fact into a project-prefixed define so its public headers can't
  collide with a consumer's own `HAVE_*` (json-c sets `JSON_C_HAVE_INTTYPES_H`
  from `HAVE_INTTYPES_H`). Aliasing rather than re-probing keeps it one shared
  probe per toolchain and makes it impossible for the two names to disagree.
- `config_header(name, template, output, probes, values)` — expands
  `template` into `output`, resolving `#cmakedefine` from probe results and
  `@VAR@` from `values`.

Each probe is a **shared target**: a converted project references a probe
from the shared catalog (`@cc_config//catalog:have_endian_h`, named for its
define lowercased) rather than declaring its own, so the probe's compile
action is analyzed once and runs **once per toolchain across the whole build
graph**, not once per project (proven — see below).

## Catalog (`catalog/`)

`@cc_config//catalog` holds a fixed, hand-maintained set of the common
autoconf facts — the header/symbol/type checks projects like json-c do —
each a public probe target named for its define. A project needing a fact not
listed adds one line to `catalog/BUILD.bazel`. `catalog_smoke_test` asserts a
few known-stable facts (e.g. `endian.h` present, `xlocale.h` absent on glibc,
`sizeof(long) == 8`) against the toolchain the module builds with.

## Tests

Rule behavior is covered by Bazel tests:

```sh
bazel test @cc_config//cc_config:all
```

The once-per-toolchain **sharing** property can't be a Bazel test (it asserts
on Bazel's own action graph via `bazel aquery` — bazel-inside-bazel), so it's
a script, run from the repo root:

```sh
bash cc_config/sharing_test.sh
```

It builds two config headers that reference the same probe and confirms via
aquery that exactly one probe action exists and both headers consume its
output.
