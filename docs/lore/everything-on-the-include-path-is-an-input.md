# Everything on the include path is an input

## The symptom

A conversion where the include path is demonstrably correct still fails:

```
external/json-c+/tests/test1.c:14:10: fatal error: 'parse_flags.h' file not found
```

while the compile command carries `-Iexternal/json-c+/tests` and
`tests/parse_flags.h` is present in the converted module. Every part of the
setup looks right, which is what makes this one confusing.

## The cause

The two build systems disagree about what a header *is*.

- **CMake** compiles `test1.c` and lets the preprocessor find
  `parse_flags.h` on disk via `-I`. The header is never a build input, so
  the File API never reports it as a source of that target — there is
  nothing to report.
- **Bazel** stages only **declared** inputs into the compile sandbox. A
  header no rule names is not in the sandbox, so `-I` points at a directory
  where the file genuinely isn't.

json-c's `tests/CMakeLists.txt` is the live case:

```cmake
foreach(TESTNAME ${ALL_TEST_NAMES})
  add_executable(${TESTNAME} ${TESTNAME}.c)          # ONE source, always
  target_include_directories(${TESTNAME} PUBLIC ${CMAKE_CURRENT_LIST_DIR})
endforeach()
```

Every test target lists exactly one `.c`. This is not a json-c quirk —
listing only `.c` files and leaving headers ambient on the include path is
how a large fraction of C projects are written.

## The rule: the directory is the declaration

`target_include_directories()` is the project's statement that a directory's
headers are inputs to the target. CMake has **no per-file header
declaration** — no way to say a target uses one header from a directory and
not another — so the directory listing is the entire available signal, and
there is nothing to be more precise than.

So the translator copies every header at or below a target's own include
directories (bounded to the source tree) and declares them.

## The rejected approach, and why it was wrong

The first implementation scanned each source for `#include "..."` and staged
only the named headers, on the theory that this was "more precise." It was
reverted (commit `2343e0c`). Four reasons, in increasing order of
importance:

1. **It re-implemented a preprocessor, badly.** No `#if` evaluation, no
   computed includes (`#include MACRO`), quoted form only, and resolution
   against the target's include dirs rather than the including file's own
   directory — which is what the quoted form actually searches first.
2. **It broke the frontend's contract.** The source of truth is the CMake
   File API, not source text (docs/architecture/cmake-frontend.md). Scanning
   `.c` files smuggled a second, weaker source of truth into `cmake_api.rs`.
3. **It invented a distinction CMake doesn't make.** "This target uses that
   specific header" is not a fact the source build system records, so the
   precision was fictional — an inference dressed as a declaration.
4. **It bought nothing.** On json-c the two approaches select the
   **identical** set of headers: all 22 root headers land in the module
   either way, and `tests/` contains exactly one header, which is included.

The cost it was avoiding — copying a header nobody includes — is a sandbox
symlink. `srcs` for a header is a declaration of *availability*, not a
dependency edge.

The reverted version also came with a fixture asserting that an
unreferenced header must **not** be copied. That requirement was invented
alongside the implementation that satisfied it; under the rule above it is
simply wrong, and the fixture no longer makes that claim.

## The walk is recursive, and follows symlinks

Two corrections that each cost a debugging session:

**Recursive.** `-Iinclude` with `#include "sub/foo.h"` is ordinary C — the
header is at `include/sub/foo.h`, and a flat listing of `include/` misses it.
The first implementation listed each include directory non-recursively, which
json-c does not catch because its headers are flat. Fixture 024 covers it.

The reason this one is worth a paragraph: its symptom is *identical* to the
bug this whole entry is about — `fatal error: 'proj/util.h' file not found`
for a header the `-I` flag plainly covers. Someone hitting it reads this
entry, sees the fix already applied, and has nowhere to go.

**Follows symlinks.** `fs::metadata`, not `symlink_metadata`. Bazel stages
every input file into the sandbox **as a symlink**, so refusing to follow
them makes the walk find nothing at all under Bazel while working perfectly
on a plain checkout. That difference is invisible to unit tests — they build
real directories in `temp_dir` — and shows up only in the fixture tier. It
did: fixture 024 stayed red with a passing unit test until the predicate was
changed, which is the fixture tier doing precisely the job the unit tier
structurally cannot.

## Two things that look wrong and are load-bearing

- **The dedup is per-target, not global.** Asking "does *any* target list
  this header?" suppresses it exactly where it's missing: json-c's
  `test1Formatted` enumerates `parse_flags.h` while its sibling `test1` —
  same include dir, one source — does not. A global set silently reproduced
  the original bug on `test1`. Pinned by
  `a_header_enumerated_on_a_sibling_target_is_still_injected_here`.
- **The `starts_with(source_dir)` guard duplicates the later
  `strip_prefix`.** It is a `read_dir` skip, not a correctness check:
  without it, a toolchain include path like `/usr/include` gets fully
  listed and every entry discarded.

## Related but different

`inject_unenumerated_installed_headers` (bzl-fxa.10) is easy to confuse with
this. It acts on `install(FILES ... DESTINATION <include>)` — the project's
authoritative statement that a header is **public** — and populates a
library's `public_headers` (→ `hdrs`).

An include directory carries no such claim. Being on the include path makes a
header an *input*, not part of the public interface, so these land in
`sources`, where a header's only job is to exist in the sandbox.
