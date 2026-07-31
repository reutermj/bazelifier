# CMake finds headers on disk; Bazel stages only declared inputs

## The symptom

A conversion where the include path is demonstrably correct still fails:

```
external/json-c+/tests/test1.c:14:10: fatal error: 'parse_flags.h' file not found
```

while the compile command carries `-Iexternal/json-c+/tests`, and
`tests/parse_flags.h` is present in the converted module. Every part of the
setup looks right, which is what makes this one confusing.

## The cause

The two build systems disagree about what a header *is*.

- **CMake** compiles `test1.c` and lets the preprocessor find `parse_flags.h`
  on disk via `-I`. The header is never a build input, so the File API never
  reports it as a source of that target — there is nothing to report.
- **Bazel** stages only **declared** inputs into the compile sandbox. A header
  no rule names is simply not in the sandbox, so `-I` points at a directory
  where the file genuinely isn't.

So the translator faithfully emitted what CMake said (`srcs = ["tests/test1.c"],
includes = ["tests"]`) and produced a build that cannot work.

## Why json-c hits it and the earlier fixtures didn't

json-c's `tests/CMakeLists.txt`:

```cmake
foreach(TESTNAME ${ALL_TEST_NAMES})
  add_executable(${TESTNAME} ${TESTNAME}.c)          # ONE source, always
  target_include_directories(${TESTNAME} PUBLIC ${CMAKE_CURRENT_LIST_DIR})
endforeach()
```

Every test target lists exactly one `.c` file. `test1.c` does an
**unconditional** `#include "parse_flags.h"` while only *calling*
`parse_flags()` under `#ifdef TEST_FORMATTED` — so plain `test1` needs the
header on its include path but never needs the object file. The sibling
`test1Formatted` target lists `parse_flags.c` and `parse_flags.h` explicitly,
which is why it built fine and made the failure look inconsistent.

This is not a json-c quirk. Listing only `.c` files and leaving headers
ambient on the include path is how a large fraction of C projects are written.

## Why the scanner doesn't evaluate `#if`

`quoted_includes` is deliberately not a preprocessor: it reports an
`#include` inside a disabled branch too. That direction is the safe one —
the result decides which headers to **stage**, and staging a header the
compiler never reads costs nothing, while missing one breaks the build.
`test1.c` is exactly that shape.

It also only reads the quoted form. `#include <...>` names a
system/toolchain header, which the module must not carry.

## Why not just copy every header under every include dir

That was the tempting shortcut, and it breaks a contract that already exists:
`copy_referenced_sources` puts a file in the module because *something in the
build graph named it*, which is what keeps `.git/`, stale build outputs and
editor scratch out without the translator knowing anything about version
control. Sweeping include directories would make the module a mirror of the
source tree.

So the injection is driven by what the sources actually `#include`, scoped to
the target's own include dirs. Fixture 023 pins both halves: `util.h` is
included and must be staged; `unused.h` sits on the same include path,
is included by nobody, and must stay out.

## Related but different

`inject_unenumerated_installed_headers` (bzl-fxa.10) solves an adjacent
problem and is easy to confuse with this one. It acts on the project's
`install(FILES ... DESTINATION <include>)` declarations — the project's own
authoritative statement that a header is **public** — and adds those to a
library's `public_headers` (→ `hdrs`).

This one has no such authority. An `#include` is evidence of a private
compile-time dependency only, so the header lands in `sources`, where its
only job is to exist in the sandbox.
