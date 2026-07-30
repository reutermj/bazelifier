# configure_file is recoverable from --trace-expand, not the File API

## What we hit

To reproduce a project's `configure_file`-generated config headers
Bazel-natively (see
[../architecture/configure-file-and-toolchain-probes.md](../architecture/configure-file-and-toolchain-probes.md)),
the translator needs to know which template produces which output. The
first, alarming finding: **the codemodel-v2/cache-v2 File API models
`configure_file` not at all.** A generated header that a target lists as a
source shows up only as an absolute build-directory path (not even flagged
`isGenerated` — see
[cmake-configure-file-generated-headers.md](cmake-configure-file-generated-headers.md)),
a header that is merely `#include`d (json-c's `config.h`) does not appear
anywhere, and the `backtraceGraph` command list contains no
`configure_file`. There is no template→output mapping and no substitution
data in the File API.

## What's actually true

**`cmake --trace-expand` logs every `configure_file` call with
fully-resolved, variable-expanded absolute paths.** Configuring json-c with
it prints, among CMake's own internal calls:

```
.../json-c/CMakeLists.txt(339):  configure_file(.../cmake/config.h.in        .../build/config.h )
.../json-c/CMakeLists.txt(341):  configure_file(.../cmake/json_config.h.in   .../build/json_config.h )
.../json-c/CMakeLists.txt(520):  configure_file(json.h.cmakein               .../build/json.h @ONLY )
```

Every argument is already expanded (`${PROJECT_SOURCE_DIR}` etc. resolved),
so this is the template→output map directly — *including for `config.h`*,
the header the File API never mentions at all. The calling site
(`.../json-c/CMakeLists.txt:339`) is in the trace too, which is how a
project's own `configure_file` calls are told apart from the ~10 internal
ones CMake makes from its own modules (`CMakeSystem.cmake.in`,
`DartConfiguration.tcl.in`, `CPackConfig.cmake.in`, ...): the project's
calls originate under the project's source directory, the internal ones
under `CMAKE_ROOT`/CMake's own module and template dirs.

So the translator recovers the mapping by configuring a second time with
`--trace-expand` (it already shells out to cmake during discovery) and
keeping the `configure_file` lines whose *template* path is inside the
source tree. It still has to parse each template for its
`#cmakedefine`/`@VAR@` macros — the trace gives the file pair, not the macro
set — but the "which template, which output" question, which the File API
cannot answer and no basename heuristic answers reliably (the templates live
in a `cmake/` subdir while their outputs land in the build root), is
answered exactly.

## Caveats

- `--trace-expand` is verbose (tens of thousands of lines for a real
  project) and prints to stderr; parse it as a stream, filtered to
  `configure_file(` lines under the source dir.
- The trace is a text format, not a stable structured API. `--trace-format=json-v1`
  exists and is the more robust parse target if the plain format proves
  fiddly — but note the JSON trace keys each event by `cmd`/`args`, and a
  first pass looking for `"configure_file"` as a substring can miss it
  depending on formatting, so parse the JSON objects properly.
- This is a *second* configure invocation (or the same one with tracing on).
  It runs at conversion time only — like the rest of the CMake frontend, not
  part of the generated output — so its non-hermeticity is the accepted
  conversion-side limitation, not a property of the deliverable.

## How to look at it

```
cmake --trace-expand -G Ninja -B <build> -S <src> 2>trace.txt >/dev/null
grep 'configure_file(' trace.txt | grep -F "<src>"
```
