# A checked-in file at a generated header's path wins the include search

zlib ships a `zconf.h` in its source tree AND generates one from
`zconf.h.cmakein` at the same path. Its CMakeLists renames the checked-in
copy to `zconf.h.included` before generating, precisely so the build reads
the generated one. The translator copied the checked-in file into the
module (it sits under the target's include directory, so the header pass
carried it) and also emitted the `config_header` rule for the generated
one, and every cc rule listed both.

In Bazel that is not a conflict: the label resolved to the generated
output, but the checked-in file was still a declared input, and the compile
line carries `-Iexternal/zlib+` BEFORE `-Ibazel-out/.../bin/external/zlib+`.
So `#include <zconf.h>` — and `#include "zconf.h"` from the sibling
`zlib.h` — found the checked-in copy. The `.d` files were unambiguous: all
37 compiles opened `external/zlib+/zconf.h`. The generated header, whose
first lines were an `#error` for the unresolved `Z_PREFIX`, was never read,
and zlib built against upstream's fallback. That is the failure CLAUDE.md
calls a check that passes because it is looking at nothing, one level down:
the escalation gate still held, but a resolved module would have carried a
config header nothing compiles against.

It surfaced on 2026-09-10 only because the `_include/` staging had been
masking it: the staged copy of the generated header was reached through
`-I_include`, ahead of the source root, so before bzl-ti9 zlib failed loudly
on the `#error`. Removing the staging (rules_cc 0.2.23 allows
`includes = ["."]`) exposed the underlying collision.

Three corpus projects had it — zlib (`zconf.h`), expat
(`expat_config.h`) and libidn2 (`lib/idn2.h`), the last two as the product
of an in-tree configure, i.e. a host-resolved header: exactly what the probes
exist to replace, shipped alongside them and winning.

The fix is `BuildGraph::displace_sources_shadowed_by_config_headers` in
`model.rs`, applied once by the driver before codegen and the copy pass:
a source reference at a config header's output path is dropped and recorded,
and the generated `BUILD.bazel` says so beside the rule. It is deterministic
because the collision is stated by the input (the `configure_file` /
`AC_CONFIG_HEADERS` output path), not inferred.

Dropping the reference was not enough, which cost a second rebuild to
learn. The file still arrived in all three modules through
`copy_test_runtime_data`, which copies a test's working directory wholesale —
and expat's tests run from the top directory, so its checked-in
`expat_config.h` was in no rule at all and came along anyway. Inside the
sandbox that copy is invisible (not an input), which is why zlib failed
loudly again; outside one it would still win. So the driver also records a
displaced path when the file merely exists in the source tree, and
`remove_displaced_sources` deletes any copy after every pass has run.

Two things to carry forward:

- When a module's config header has an `#error` and the module still
  builds, ask which file the compiler actually opened. The `.d` files under
  `bazel-out/.../_objs/` answer it in seconds; the aquery inputs list does
  not, since both files are inputs.
- A generated file and a source file at one path is a shape Bazel accepts
  silently. Anything else the translator generates at a path the project
  also ships — not only config headers — has the same exposure.
