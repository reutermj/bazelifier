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
- `config_header(name, template, output, probes, values)` — expands
  `template` into `output`, resolving `#cmakedefine` from probe results and
  `@VAR@` from `values`.

Each probe is a **shared target**: the intended usage is that a converted
project references a probe from a shared catalog rather than declaring its
own, so the probe's compile action is analyzed once and runs **once per
toolchain across the whole build graph**, not once per project. (The shared
catalog of common facts is still to be added; the sharing *mechanism* is
proven — see below.)

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
