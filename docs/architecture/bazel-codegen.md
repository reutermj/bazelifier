# Bazel codegen

Covers how bazelifier turns the internal build-graph model (see
[cmake-frontend.md](cmake-frontend.md)) into Bazel `BUILD` files.

## Goals

- Emit a **standalone Bazel module**, not just a `BUILD.bazel` — every
  conversion gets its own `MODULE.bazel` (`bazel_dep`s on `rules_cc`,
  `llvm`, etc., plus `register_toolchains`) so the output builds with no
  reference back to bazelifier's own workspace. See
  [build-verification.md](build-verification.md) for why this is the whole
  point and how it's actually verified (unpacked and built completely
  outside this repo).
- Emit idiomatic, human-readable `BUILD` files — output is meant to be
  reviewed and maintained by people, not treated as a black box.
- Prefer native Bazel rules (`cc_library`, `cc_binary`, `cc_test`, etc.) over
  custom genrule-wrapping of the original build system wherever the
  translator can confidently produce them. Implemented: `cc_binary`
  (CMake `EXECUTABLE`) and `cc_library` (both `STATIC_LIBRARY` and
  `SHARED_LIBRARY`). Linkage is not a different rule — both forms are a
  `cc_library` — but the distinction is preserved rather than normalized
  away; see "Shared libraries" below. `cc_library` gets `srcs`, `hdrs` (only headers the
  project *declared* public, via a `FILE_SET` or an `install()` to an
  include destination — see [cmake-frontend.md](cmake-frontend.md)),
  `includes`
  (the target's own, not inherited ones — transitive via Bazel), and
  `deps` (resolved sibling target names, rendered as `":name"`).
  `cc_binary` gets `srcs`, `includes`, and `deps` — `includes` is not a
  library-only concern, since Bazel's transitivity supplies a *consumer*
  with its dependencies' include dirs but never a target with its own (see
  `004-binary-private-include` in
  [build-verification.md](build-verification.md#fixtures)).
- Where the translator can't confidently produce a native rule, escalate via
  `needs_attention/` (see
  [needs-attention-interface.md](needs-attention-interface.md)) rather than
  silently emitting something wrong or overly conservative.
- Generated targets default to `visibility = ["//visibility:public"]`: a
  converted module is meant to be depended on — both by bazelifier's own
  validation tooling, and, as more projects get converted, by other
  converted modules (a converted library's module can become a real
  `bazel_dep` of a converted app's module). CMake has no per-target
  visibility concept of its own to translate, so there's no source-level
  signal to narrow this from.

## Generated module layout

For a CMake project with one or more targets, the translator currently
produces:

The module's root is derived rather than assumed to be the CMake project
directory — see [cmake-frontend.md](cmake-frontend.md). It usually is the
project directory, but when the build references files from a sibling that
ships in the same deliverable, the root widens to cover both and the
project's own sources move under a subdirectory (`proj/src/main.cpp`
alongside `shared/helper.cpp`). Everything below is relative to that root.

```
<out_module>/
  MODULE.bazel        module(name=...) [+ version, if CMAKE_PROJECT_VERSION
                       was set] + bazel_dep(rules_cc, llvm) +
                       register_toolchains
  BUILD.bazel          the user-facing converted output (cc_binary/cc_library)
  src/...              only the source files the build graph references,
  include/...          at their original paths relative to the module root.
                       NOT a recursive copy of the CMake project directory,
                       and the project's own CMakeLists.txt is not among
                       them — see cmake-frontend.md's "only referenced files
                       enter the module"
  _include/           copies of headers that must be reachable by an angled
                       include — public headers AND generated config headers.
                       Present ONLY when a project declared its module root as
                       an include directory — a rules_cc workaround, not
                       project layout. See "_include/: a staging workaround"
  project_notes/      oddities of THIS project where the obvious answer is
    <NNN>-<slug>.md    wrong — knowledge no general guidance reaches, shipped
                       because the resolving agent has no access to this repo.
                       ABSENT when the project has none, rather than empty:
                       an empty directory reads as "someone looked". See
                       needs-attention-interface.md
  ground_truth/
    BUILD.bazel        exports_files(...) only — NOT part of the
                       user-facing output, validation-only (see
                       build-verification.md)
    <artifact>          the real cmake+ninja-built binary/library
  needs_attention/
    BUILD.bazel        a single allow_empty filegroup — NOT part of the
                       user-facing output, validation-only
    <NNN>-<slug>.md     present only if the translator hit a gap for
                       THIS conversion it couldn't confidently resolve
                       — see cmake-frontend.md's needs_attention/ section
                       and needs-attention-interface.md
```

Both subdirectories are deliberately separate nested packages (their own
`BUILD.bazel`) rather than exported from the top-level `BUILD.bazel`, so
validation-only targets never appear in what a user actually checks into
their own repo. `needs_attention/`'s package is written unconditionally,
even for a conversion with nothing to triage: the validation tests depend
on `@<module>//needs_attention:all` whether or not any item exists, which
is what the `allow_empty` glob is for.

## Every emitted path must be module-relative

A converted module is meant to be checked into someone else's repo, so no
path in its generated files may reference the machine that produced it.
The CMake File API makes this easy to get wrong: it reports a source path
relative to the project **only when the file is inside it**, and absolute
otherwise (see [cmake-frontend.md](cmake-frontend.md)).

`model::is_module_relative` is the single definition of that contract — it
rejects absolute paths and `..` components — and the frontend uses it to
decide what to escalate. Codegen then enforces it again in
`render_path_list`, which every path-valued attribute goes through:

- It is the **last point every path passes through**, so it catches paths
  from any frontend field, including ones added later, rather than only
  the cases a test enumerated. Two separate bugs (an `OBJECT_LIBRARY`'s
  generated `.o` paths, and ordinary sources in a sibling directory)
  reached `srcs` as absolute paths before it existed, each needing its own
  targeted fix.
- It **panics rather than degrading**. A violation here is a translator
  bug, not bad input — input gaps go to `needs_attention/`. Failing loudly
  with no `BUILD.bazel` written beats emitting a module that is silently
  non-portable.
- It is a real `assert!`, not a `debug_assert!`, because Bazel only catches
  *part* of this downstream. An absolute path in a **label** attribute
  (`srcs`, `hdrs`, `deps`) is an analysis error — verified on Bazel 9.2.0:
  `target names may not start with '/'`. But `includes` is a plain string
  list, and `includes = ["/abs/path"]` **builds successfully**. That module
  then works on the machine that generated it and nowhere else, which is
  exactly the "green for the wrong reason" outcome
  [build-verification.md](build-verification.md#why-unpack-it-rather-than-validate-in-tree)
  is about. So the check cannot be left to Bazel.

Because the assert is always on, every real conversion exercises it — the
fixtures don't need a separate "no absolute paths" assertion, since the
translator refuses to produce such output in the first place.

## Shared libraries

A CMake `SHARED_LIBRARY` becomes a `cc_library` plus a `cc_shared_library`
wrapping it, and consumers that link it get **both** `deps` and
`dynamic_deps`:

```python
cc_library(name = "zlib", srcs = [...], hdrs = [...])
cc_shared_library(name = "zlib_shared", deps = [":zlib"])

cc_binary(
    name = "zlib_example",
    deps = [":zlib"],                 # CcInfo: headers, include paths
    dynamic_deps = [":zlib_shared"],  # CcSharedLibraryInfo: link the .so
)
```

Both edges are required and neither substitutes for the other:
`cc_shared_library` provides `CcSharedLibraryInfo` and **not** `CcInfo`, so
putting it in `deps` is an analysis error, and dropping `deps` loses the
headers.

The distinction is preserved rather than normalized to static because
dropping it is invisible to the equivalence check: zlib builds the same
sources both ways and its two consumers produce byte-identical output. What
does differ is the symbol table — zlib's shared link applies a version script
that hides internals the static archive exposes.

`dynamic_deps` is emitted only on executables, because `cc_library` has no
such attribute. A library that links a shared library therefore cannot express
that in Bazel and would downgrade to a static link; no corpus project hits it
yet, since every consumer of zlib's shared `libz` is an executable.

A generated `sh_test` wrapping a dynamically linked binary needs
`LD_LIBRARY_PATH`: the binary finds its `.so` through an `$ORIGIN`-relative
`RUNPATH`, and `RUN_CMAKE_TEST_SH` stages runfiles into a writable tree (the
binary writes into its working directory), which breaks those relative paths.

## `_include/`: a staging workaround

A converted module may carry an `_include/` directory holding copies of its
public headers, with `includes = ["_include"]` on the targets that own them.
**This is a workaround for a rules_cc limitation, not project layout**, and it
is meant to be removed.

A header at a module's ROOT cannot be reached by `#include <angled>`: Bazel
passes only `-iquote .` for a root package and rejects `includes = ["."]`
outright ("resolves to the workspace root, which would allow this rule and all
of its transitive dependents to include any file in your workspace"). zlib's
`zlib.h` does `#include <zconf.h>`, so without the staging the module does not
compile.

It fires only when a project declared its own module root as an include
directory. What it stages is a decision that has already changed once — see
`render_staged_headers` and `staged_for` in `translator/src/codegen.rs`,
which own the trigger and record why gating it on public headers alone left
xz's liblzma with an `-I_include` for a directory it could not reach. When
rules_cc supports reaching a module-root header from an angled include,
delete the staging and emit the headers in place.

## Formatting and linting

Generated (and hand-written) `BUILD`/`MODULE.bazel`/`.bzl` files are checked
and formatted with [buildifier](https://github.com/keith/buildifier-prebuilt),
via `bazel_dep(name = "buildifier_prebuilt", ..., dev_dependency = True)` in
`MODULE.bazel`. Two root targets:

- `bazel run //:buildifier` — formats/fixes files in place.
- `bazel test //:buildifier_check` — fails (non-destructively) if anything
  needs formatting; intended for CI.

This covers the repo's own Bazel files. It does **not** yet cover the
translator's *generated* output: `codegen.rs` writes its `BUILD.bazel` and
`MODULE.bazel` directly, formatted by hand-written string rendering rather
than passed through buildifier. Since generated output is meant to be
reviewed and maintained by people, it should eventually go through
buildifier on the way out rather than relying on the renderer to stay
idiomatic by hand.

## Open questions

- **Rule set / ruleset dependencies:** `rules_cc` is used today; needs
  revisiting as more target kinds (libraries, tests) get added.
- **Pinned toolchain versions:** `RULES_CC_VERSION`/`LLVM_VERSION` in
  `translator/src/codegen.rs` are hardcoded constants — there's no
  per-project toolchain-selection mechanism yet. Fine for now (one set of
  fixtures, one toolchain), but will need a real design once fixtures need
  different C++ standards/toolchain requirements.
- **Module versioning beyond the top-level project:** only
  `CMAKE_PROJECT_VERSION` (the top-level `project()`'s version) is read
  today — see [cmake-frontend.md](cmake-frontend.md).

This doc is intentionally thin until there's more real codegen to
describe — expand it as the translator grows rather than speculating ahead
of the code.
