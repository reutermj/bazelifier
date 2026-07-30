# configure_file-generated headers: how they surface (two ways, one silent)

## What we hit

json-c (the second corpus project) is an autoconf-style C library: 60+
`check_include_file`/`check_symbol_exists`/`check_type_size` probes feed
`configure_file` calls that generate `config.h`, `json_config.h`, and
`json.h` (the *public* header, from `json.h.cmakein`). The `.c` sources
`#include "config.h"`, and these headers exist **only in the CMake build
directory** — none are in the source tree. The naive expectation was a
single, silent compile failure (`config.h: No such file or directory`).
Reality is more interesting: the same project surfaces the problem **two
different ways**, and only one of them is silent.

## What's actually true

**`configure_file` outputs are NOT flagged `isGenerated` in the File API.**
This is the crux and it is counterintuitive. Contrast an
`add_custom_command(OUTPUT gen.cpp)` source, which the codemodel reports
with `isGenerated: true` (fixture 007). A `configure_file` header gets no
such flag. So the `is_generated` branch in `to_target` — the one that feeds
`generated_sources_needs_attention` — never fires for them.

Where a generated header shows up depends on whether the project listed it
as a target source:

1. **Listed as a target source** (`json_config.h`, `json.h` — json-c puts
   them in `add_library(... ${JSON_C_HEADERS})`). The File API reports them
   in `sources[]`, as **absolute paths into the build directory**
   (`.../converted_cmake_build/json_config.h`), *not* generated. The
   translator's module-root rebasing (`rebase_to_module_root`) then can't
   place an absolute path inside the module and escalates it via
   `sources_outside_deliverable_needs_attention`. Not silent — but the
   escalation **misdiagnoses** it (see below).

2. **Only `#include`d, never a target source** (`config.h`). It appears
   nowhere in the target reply — only implicitly, resolved at compile time
   through the build-dir include path (`PROJECT_BINARY_DIR`). The translator
   has no way to see it at all, so it copies the `.c` files, emits the
   `cc_library`, and the module **fails to compile** with no escalation.
   This is the genuinely silent case.

So one project needs *both*: recognizing a generated header that arrives as
an out-of-deliverable source, AND detecting the header a `.c` file includes
that only ever existed in the build dir.

## Why the sources-outside escalation is wrong for case 1

`sources_outside_deliverable_needs_attention` frames the fix as one of:
widen the deliverable root (the file is a ship-alongside sibling), or vendor
the file (it's a system/prebuilt artifact). **Neither fits a
`configure_file` output.** Widening the root can't help — the header isn't
in the source tree under any root. Vendoring "the file" means copying *this
machine's* generated header, which bakes in this host's feature-detection
results (`HAVE_STRINGS_H`, `SIZEOF_LONG`, ...) — the opposite of the
hermetic, portable config the generated `MODULE.bazel` is supposed to have.
The honest resolutions are project-specific and not in that escalation's
menu: reproduce the substitution as a Bazel rule, or check in a
conservative fixed config, or (least good) capture the host's. Recognizing
the `configure_file` case *before* the generic sources-outside check is what
would route it correctly.

## How configure_file itself works (for whoever implements this)

`configure_file(<in> <out>)` copies `<in>` to `<out>`, substituting
`@VAR@`/`${VAR}` with CMake variable values and turning `#cmakedefine FOO`
into `#define FOO`/`/* #undef FOO */` depending on whether `FOO` is set.
The values come from the cache the probes populated. The translator already
*runs* configure, so the resolved header exists in its `build_dir` at
conversion time (that is where the escalation's absolute paths point) —
capturing it is mechanically trivial; the question is whether a host-derived
config is an acceptable conversion output, which is a judgement call, not a
mechanics problem.

## How to check this sort of thing

Configure the project and read a target reply for a header you know is
generated:

```
python3 -c "import json,glob; d=json.load(open(glob.glob('<build>/.cmake/api/v1/reply/target-<lib>-*.json')[0])); \
  print([(s['path'], s.get('isGenerated', False)) for s in d['sources'] if s['path'].endswith('.h')])"
```

A `configure_file` output shows as an absolute build-dir path with
`isGenerated` **absent/false** — the tell that distinguishes it from both a
checked-in header (relative path) and an `add_custom_command` output
(`isGenerated: true`). A header that's only `#include`d won't appear at all;
to find those, grep the sources for `#include "…"` and check which names
resolve only under the build dir.
