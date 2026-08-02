//! Recommended-resolution recipes shipped into every converted module's
//! `resolutions/` directory.
//!
//! A `needs_attention/` item says what the translator could not resolve for
//! THIS project. A recipe here says how gaps of that shape are usually
//! resolved, in Bazel, generally. The two have different lifetimes — an item
//! is specific and disappears when resolved, a recipe evolves with the
//! translator's capabilities — which is why they are separate files rather
//! than more prose inside the escalation.
//!
//! **Recipes are sketches to adapt, never patches to apply.** They cannot
//! know the project's actual macro names, header layout or release. zlib
//! 1.3.2 needs three specific `#cmakedefine`s spliced into its checked-in
//! `zconf.h`; the next release needs different ones. A recipe that tried to
//! be directly applicable would rot; one that explains the shape does not.
//!
//! Shipped PER MODULE rather than once at the tarball root, and the
//! duplication is deliberate. A converted module is the deliverable: it has
//! its own `MODULE.bazel` and is meant to be lifted out and checked into
//! someone else's repo, at which point anything living at the tarball root
//! is gone — while the open `needs_attention` items travel along. Same
//! reasoning that makes each escalation self-contained (see CLAUDE.md).
//!
//! No machine-readable mapping from item to recipe, deliberately. Filenames
//! are descriptive and `README.md` lists them; an agent that can read an
//! escalation can find the matching recipe. A `kind` field threaded through
//! the item schema would be two things to keep in sync — one of them emitted
//! text — bought to save a directory listing.

/// One recipe file: its name inside `resolutions/`, and its content.
pub struct Recipe {
    pub filename: &'static str,
    pub body: &'static str,
}

/// Every recipe shipped into a converted module, `README.md` first.
///
/// Returned wholesale rather than selected by which escalations fired: a
/// module whose items were resolved and deleted still benefits if a
/// re-conversion reopens one, and a complete directory cannot be mistaken
/// for a truncated one.
pub fn all() -> Vec<Recipe> {
    vec![
        Recipe {
            filename: "README.md",
            body: README,
        },
        Recipe {
            filename: "generated-config-header.md",
            body: GENERATED_CONFIG_HEADER,
        },
        Recipe {
            filename: "unmapped-config-macros.md",
            body: UNMAPPED_CONFIG_MACROS,
        },
        Recipe {
            filename: "header-visibility.md",
            body: HEADER_VISIBILITY,
        },
        Recipe {
            filename: "ctest-command-not-a-target.md",
            body: CTEST_COMMAND_NOT_A_TARGET,
        },
        Recipe {
            filename: "shared-library-absorbs-static.md",
            body: SHARED_LIBRARY_ABSORBS_STATIC,
        },
    ]
}

const README: &str = r#"# Recommended resolutions

This directory holds **recipes**: each describes how one *shape* of gap is
usually closed in Bazel. The `needs_attention` item next door describes what
is actually wrong in THIS project.

When the two disagree, **the item wins** — it knows the project and the
recipe does not. A recipe is a shape to adapt, never a patch to apply; it
cannot know your macro names, header layout, or release.

## The rules a resolution must not break

These hold regardless of which recipe you are following:

- **Resolve in the GENERATED output.** Edit this module's `BUILD.bazel` and
  its own copies of files. Never edit the project's own build files —
  `CMakeLists.txt`, `Makefile.am`, `configure.ac`, whichever this project
  uses. They are the input being translated, and "fixing" one leaves the next
  project with the same shape just as broken.
- **Do not vendor build-machine results.** Anything the conversion host
  computed (feature-detection values, a generated config header, an absolute
  path) is a fact about that host, not about this project. Baking it in makes
  the module build correctly only where it was converted.
- **Keep the module portable.** No absolute paths, no reference back to the
  converter's checkout. The module has to work when someone drops it into
  their own repo.


"#;

const GENERATED_CONFIG_HEADER: &str = r##"# Recipe: a config header the build generates

**Matches items about**: a target compiling against a header that exists only
in the CMake build directory, or `#cmakedefine`/`@VAR@` macros with no
mapping.

## The shape of the problem

CMake's `configure_file()` expands a template into a header carrying
feature-detection results (`HAVE_STDLIB_H`, `SIZEOF_LONG`, ...). Those values
are facts about the **toolchain that ran CMake**. The generated header is in
the build directory, which is not part of this module.

Copying that header in is the obvious move and the wrong one: it freezes the
conversion host's answers into a module meant to build anywhere.

## What to use instead

This module already depends on `cc_config`, a Bazel-native probing module
that reproduces CMake's `check_include_file` / `check_symbol_exists` /
`check_type_size` as rules resolving **the consumer's** toolchain. Check this
module's `MODULE.bazel` for its `bazel_dep`. You do not need to build a
probing mechanism; you need to wire this header up to the one that exists.

The rule is `config_header`, and a wired-up example looks like:

```python
load("@cc_config//cc_config:config_header.bzl", "config_header")

config_header(
    name = "config_h",
    template = "config.h.in",
    output = "config.h",
    probes = [
        "@cc_config//catalog:have_stdlib_h",
        "@cc_config//catalog:have_unistd_h",
    ],
)

cc_library(
    name = "mylib",
    srcs = ["mylib.c", ":config_h"],
    ...
)
```

`probes` are catalog targets, one per macro the template declares; the target
name is the macro lowercased. Browse `@cc_config//catalog` for what exists. A
macro with no catalog entry is a real gap — say so rather than guessing at a
lookalike (a project's `MYPROJ_HAVE_STDINT_H` is not the catalog's
`HAVE_STDINT_H`).

## When the template itself is generated

Some projects do not ship a `.in`/`.cmakein` at all — they BUILD one at
configure time and then expand it. zlib assembles `zconf.h.cmakein` from its
checked-in `zconf.h` with `file(READ/WRITE/APPEND)` and byte offsets.

**The translator already handles this**, so you should not have to. It stages
the generated template into the module and points `template =` at the staged
copy, exactly as if the project had shipped it. If your module has a
`config_header` rule whose `template` names a file that is present beside your
`BUILD.bazel`, that is what happened, and there is nothing to do here.

You are reading this section because something ELSE went wrong. The likely
cases, in order:

- **A macro has no catalog probe.** That is the separate unmapped-macro
  escalation, not this one. Resolve it by adding the probe to the catalog or,
  if the macro is a project option rather than a toolchain fact (zlib's
  `Z_PREFIX`), by supplying it through `values` instead.
- **The template was unreadable at conversion time.** Then it is genuinely not
  in the module, and the only honest resolution is to reconstruct it from
  whatever the project DOES ship. Diff the checked-in header against the one
  CMake generated in its build directory, take the checked-in file as your
  template, and add the `#cmakedefine` lines the difference implies — to
  **this module's own copy**, never upstream. For zlib 1.3.2 that difference
  is four lines directly after `#define ZCONF_H`:

  ```c
  /* #undef Z_PREFIX */
  #define HAVE_STDARG_H 1
  #define HAVE_UNISTD_H 1
  ```

**Never model the generation itself.** Reproducing `file(READ/WRITE/APPEND)`
and byte offsets means reimplementing CMake, and the result rots against the
next release. Reproduce the RESULT.

## Sharp edges

These have all bitten. Check each one.

**Several files look like the template, and the wrong ones expand to
nothing.** zlib ships `zconf.h` (the checked-in header) and `zconf.h.in` (the
*autotools* template, a different build system's), while the CMake template
`zconf.h.cmakein` exists only in the build directory. Neither source file
contains a single `#cmakedefine`, so pointing `template =` at either produces
a header with none of the macros — and no error, because a template with
nothing to substitute expands cleanly. Confirm your chosen template actually
contains the directives you expect before wiring it up.

**The augmented and unaugmented headers often share a name.** zlib generates
`zconf.h` into the build directory while `zconf.h` also sits in the source
tree, and CMake puts the build directory FIRST on the include path so the
generated one wins. In Bazel the generated file must likewise be the one the
compile resolves: reference the `config_header` target in `srcs` and make sure
no `includes` entry lets the source-tree copy shadow it.

**A missing macro can be silent.** zlib's own header does:

```c
#if HAVE_UNISTD_H-0     /* may be set to #if 1 by ./configure */
```

An *undefined* macro evaluates to `0` here — no error, no warning. So a header
that is nearly right compiles, links, runs, and quietly takes a different code
path. Do not treat a green build as proof the header is correct.

**Do not vendor the generated header.** It is tempting once you have seen how
small the diff is. Those values are the conversion host's answers; a consumer
on another platform needs their own, which is the entire reason `config_header`
resolves probes at build time.

## Checking your work

Compare the header your rule produces against the one CMake generated in the
build directory. They should agree on which macros are defined. They may
legitimately differ in *values* if the toolchains differ — that is the point
of probing rather than copying.
"##;

const HEADER_VISIBILITY: &str = r#"# Recipe: headers with no public declaration

**Matches items about**: a library with header-like sources that nothing
declares public, where other targets depend on it.

## The shape of the problem

CMake has two ways to declare a header public — a `target_sources(... FILE_SET
... TYPE HEADERS)` or an `install(FILES ... TYPE INCLUDE)` — and this project
used neither. Without one, the File API does not say which headers are the
library's interface and which are internal, so the translator put them all in
`srcs` rather than guessing.

## Why the build is probably already green

Bazel does not enforce the `hdrs`/`srcs` split by default. A header in a
dependency's `srcs` is still an input to a dependent's compile, so consumers
can `#include` it either way. **A green build does not mean this is
resolved** — the gap is an unclear public/private boundary, which is why it
needs a decision rather than an inference.

## What to do

Work out which headers are actually the library's interface — the ones
dependents `#include` — and move those from `srcs` to `hdrs` in this module's
`BUILD.bazel`:

```python
cc_library(
    name = "mylib",
    srcs = ["mylib.c", "internal_detail.h"],
    hdrs = ["mylib.h"],
    includes = ["include"],
)
```

Evidence worth using, roughly in order of strength: which headers the
project's own `install()` rules ship (even if not to an include destination);
which headers other targets in this module actually `#include`; and the
project's own directory layout (`include/` vs `src/`).

When the answer is genuinely unclear, prefer leaving a header in `srcs`. That
keeps the build working and leaves the question open, where promoting a
private header to `hdrs` silently widens the library's contract.
"#;

const CTEST_COMMAND_NOT_A_TARGET: &str = r#"# Recipe: a registered test that is not a binary this module builds

**Matches items about**: CTest tests whose command is not an executable the
conversion produced.

## The shape of the problem

`add_test()` accepts any command. Some are binaries the project builds — the
translator wraps those in an `sh_test` automatically. Others are checked-in
shell scripts, an interpreter, or a system tool (`cmake`, `gcov`, `python`).
Those cannot become a label into this module, because nothing here builds
them.

## Deciding what to do

Ask what the test actually exercises:

- **It tests this project's code through a script that ships with the
  project** (a `.test` shell script, a Python driver). Worth reproducing: add
  an `sh_test` whose `srcs` is the script, with the binaries it invokes in
  `data` so they are staged.

  ```python
  sh_test(
      name = "mytest",
      srcs = ["tests/mytest.sh"],
      data = [":myprogram"],
  )
  ```

- **It tests the CMake build itself** — `cmake --build` of a sample project,
  an install-and-find_package check, a coverage report via `gcov`. These
  verify the *CMake packaging*, which the converted module does not have and
  is not trying to reproduce. Dropping them is correct. Say so explicitly
  rather than leaving it implicit.

- **It runs a system tool on build output.** Judge whether the property being
  checked survives conversion at all. A test asserting a `.so` exports a
  specific symbol set is meaningful and reproducible; one asserting CMake put
  a file at a particular install prefix is not.

## What not to do

Do not fabricate a target so the label resolves. An `sh_test` pointing at
something this module does not build fails at analysis time, which is worse
than a documented omission — and a test that runs but exercises nothing is
worse still.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recipe_has_a_distinct_markdown_filename() {
        let recipes = all();
        let mut names: Vec<&str> = recipes.iter().map(|r| r.filename).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(count, names.len(), "duplicate recipe filename: {names:?}");
        for r in &recipes {
            assert!(
                r.filename.ends_with(".md"),
                "recipes ship as markdown an agent reads directly: {}",
                r.filename
            );
            assert!(!r.body.trim().is_empty(), "{} is empty", r.filename);
        }
    }

    // A recipe is found by `ls resolutions/` and by matching the item's
    // `kind`, so its FILENAME is the whole index — the README used to carry a
    // hand-maintained list and that is now the resolve-escalations skill's
    // job. What still has to hold is that the name says which shape it
    // covers: a recipe called `notes.md` is invisible however well written.
    #[test]
    fn every_recipe_is_named_for_the_shape_it_covers() {
        for r in all().iter().filter(|r| r.filename != "README.md") {
            let stem = r
                .filename
                .strip_suffix(".md")
                .expect("recipes are markdown");
            assert!(
                stem.contains('-') && stem.len() > 8,
                "a recipe's filename is how it is found — {:?} does not name \
                 a gap shape",
                r.filename
            );
            assert!(
                r.body.starts_with(&format!("# Recipe: ")),
                "{} must open by saying which shape it covers, since that is \
                 the first thing read after the filename:\n{}",
                r.filename,
                r.body.lines().next().unwrap_or("")
            );
        }
    }

    // The three rules in the README are the ones a wrong resolution breaks,
    // and each is stated somewhere in the repo's own docs. They ship because
    // the resolving agent cannot see those docs.
    #[test]
    fn the_readme_states_the_rules_a_resolution_must_not_break() {
        let recipes = all();
        let readme = &recipes
            .iter()
            .find(|r| r.filename == "README.md")
            .expect("resolutions/ must ship a README")
            .body;

        // Every build system this converts, not just the first one: the
        // README ships byte-identical into all 35 modules, and an Autotools
        // project has no CMakeLists.txt to be told not to edit.
        for input in ["CMakeLists.txt", "Makefile.am", "configure.ac"] {
            assert!(
                readme.contains(input),
                "must name {input} among the input build files not to edit:\n{readme}"
            );
        }
        assert!(
            readme.contains("vendor"),
            "must say not to vendor build-machine results:\n{readme}"
        );
        assert!(
            readme.contains("portable") || readme.contains("absolute path"),
            "must say the module has to stay portable:\n{readme}"
        );
    }

    // A recipe that reads as a patch invites applying it verbatim to a
    // project whose macros and layout differ — the failure this directory
    // exists to prevent.
    #[test]
    fn the_config_header_recipe_says_to_adapt_rather_than_apply() {
        let recipes = all();
        let recipe = &recipes
            .iter()
            .find(|r| r.filename == "generated-config-header.md")
            .expect("missing config header recipe")
            .body;

        assert!(
            recipe.contains("Reproduce the RESULT"),
            "the recipe must tell the agent to reproduce the OUTCOME rather \
             than model file(READ/WRITE/APPEND):\n{recipe}"
        );
        // bzl-8u1: this section told agents to hand-splice a template long
        // after dee7639 made the translator stage it automatically, which
        // would have them REPLACE correct generated output with a hand-edited
        // copy. A capability landing must not leave the recipe claiming the
        // agent still has to do it.
        assert!(
            recipe.contains("The translator already handles this"),
            "the recipe must say the staged-template case is automatic — \
             telling an agent to do it by hand undoes correct output:\n{recipe}"
        );
        assert!(
            !recipe.contains("there is nothing in the module to point"),
            "stale claim: the translator stages a build-dir template into the \
             module, so `template =` does have something to point at:\n{recipe}"
        );
        assert!(
            recipe.contains("cc_config"),
            "and name the mechanism that resolves it:\n{recipe}"
        );
        // zlib's trap: the template is assembled at configure time, so
        // `template =` has nothing in the module to point at.
        assert!(
            recipe.contains("they BUILD one at"),
            "must cover the case where the template itself is generated:\n{recipe}"
        );

        // The three sharp edges, each verified against zlib 1.3.2. Dropping
        // any one leaves a mistake that produces a GREEN build.
        assert!(
            recipe.contains("the wrong ones expand to"),
            "must warn that the near-miss templates (zconf.h, zconf.h.in) \
             carry no #cmakedefine and expand silently:\n{recipe}"
        );
        assert!(
            recipe.contains("share a name"),
            "must warn that the generated and checked-in headers collide by \
             name, so the wrong one can win the include:\n{recipe}"
        );
        assert!(
            recipe.contains("HAVE_UNISTD_H-0"),
            "must show the idiom where an undefined macro evaluates to 0, \
             which is why a green build proves nothing here:\n{recipe}"
        );
    }
}

/// Recipe for the biggest escalation class by volume: a config header the
/// translator DID reproduce, except for macros with no catalog probe.
///
/// Written from resolving xz, whose `config.h.in` names 153 of them. The
/// value is not the specific answers — those are xz's — but the
/// classification, because triaging 150 names one at a time is what makes
/// this escalation look impossible when it is actually four groups.
const UNMAPPED_CONFIG_MACROS: &str = r##"# Recipe: a config header naming macros the catalog does not have

## When this applies

A `needs_attention` item titled "Config header ... references names not in the
shared catalog". The `config_header` rule is already wired and most of the
template resolved; what remains is a list of macro names — possibly a very
long one. xz's `config.h` had 153.

The list length is misleading. Do NOT work down it one name at a time. Sort it
into the four groups below first; almost every name falls into one of them,
and each group is decided once rather than per-name.

## Group 1: portable toolchain facts -> extend the catalog

A plain header check or libc symbol check that any project might ask:
`HAVE_BYTESWAP_H`, `HAVE_STDBOOL_H`, `HAVE_GETOPT_LONG`, `HAVE_CLOCK_GETTIME`.

Add one line to `cc_config/catalog/BUILD.bazel` (`headers`, `symbols` or
`types`), and the SAME define to the translator's `CATALOG_DEFINES`. The
`catalog_sync_check` test fails if you do one and not the other — that is
working as intended, not an obstacle.

Then re-run the conversion. These disappear from the escalation permanently,
for this project and every later one.

**Be strict about what qualifies.** A compiler builtin
(`HAVE___BUILTIN_BSWAPXX`), a glibc-only extension
(`HAVE_PROGRAM_INVOCATION_NAME`), or anything whose honest answer is
"depends on the libc" does NOT belong in the catalog, however `HAVE_`-shaped
it looks. Putting it there hands the next project a non-portable answer it
never asked for and cannot see. Those go in group 3.

## Group 2: project feature switches -> `values`

Macros that say what to BUILD rather than what the toolchain supports:
`HAVE_DECODER_LZMA2`, `HAVE_ENCODER_X86`, `HAVE_CHECK_CRC64`, `HAVE_SMALL`,
`ENABLE_NLS`. These come from `--enable-foo` / `--with-foo` options, so the
answer is identical on every toolchain.

Put them in the `config_header`'s `values`, matching what the original build
was configured with — the ground-truth artifacts came from that
configuration, so anything else fails the equivalence comparison for a reason
that looks like a miscompile.

## Group 3: target-specific facts -> `values`

True or false for the toolchain the CONVERTED module builds against, but not
portable enough for the catalog: compiler builtins, glibc extensions,
`WORDS_BIGENDIAN`, `_FILE_OFFSET_BITS`, `TUKLIB_FAST_UNALIGNED_ACCESS`.

Also every macro naming a platform the module does not target — BSD, Apple,
Windows, AIX. Set those to `0`.

`0` means "leave undefined", the same as omitting the name; the expander
treats a CMake-false value as unset. Prefer the explicit `0` anyway: it
records that the name was considered.

## Group 4: fallback typedefs -> `0`, always

`int32_t`, `uint64_t`, `_UINT8_T`, `uintptr_t` and friends. autoconf defines
these only when the real type is MISSING. On any toolchain with `stdint.h`
they must stay undefined — defining them typedefs over the real type, and the
error surfaces far away.

## Then build it, because the header is not the finish line

Building the module after resolving is not optional verification, it is part
of the recipe. Two failures show up only here, and neither points back at the
config header:

- **Undefined symbols at link.** Usually not the config header at all — check
  whether the target's `srcs` count matches the number of objects the original
  build produced.
- **`undefined version` from the linker.** The project uses symbol versioning
  and its `.map` file was not carried over. Turning the feature off
  (`HAVE_SYMBOL_VERSIONS_LINUX` -> `0`) produces a correct library that
  exports unversioned symbols. Note that in the resolution: it is a workaround,
  and it changes what the library is for a distro package even though the
  comparison passes.

## What not to do

- Do NOT copy the config header this conversion's host generated. It bakes in
  the conversion machine's toolchain, which is not the one that builds the
  module.
- Do NOT edit the project's `configure.ac` or `Makefile.am`.
- Do NOT map a project-prefixed macro onto a lookalike catalog probe by
  guessing. If `FOO_HAVE_STDINT_H` really is an alias of `HAVE_STDINT_H`, wire
  it to that probe deliberately; the translator refused to guess for you on
  purpose.
"##;

/// How to resolve a shared library that links a static one from the same
/// module. Deliberately lays out the options with their trade-offs rather
/// than prescribing one: which is right depends on what the archive is to
/// the project, which is exactly why the translator escalates.
const SHARED_LIBRARY_ABSORBS_STATIC: &str = r#"# Recipe: a shared library that absorbs a static one

## What you are looking at

A `cc_shared_library` in this module links a `cc_library` that is also in
this module, and Bazel refuses to build it:

```
Two shared libraries in dependencies link the same library statically
The following libraries were linked statically by different
  cc_shared_libraries but not exported
```

The error names generated rules, not anything in the original project, which
is why an item was written for it.

In the original build this was almost certainly automake's
`noinst_LTLIBRARIES` — a **convenience archive**. It is compiled but never
installed, and exists so several targets can share a set of objects. libtool
builds no `.so` for one; the objects are pulled into whatever links it.
gnulib's `libgnu.la` is the common example.

## Deciding

Ask one question: **does anything other than this shared library use the
archive?**

Check the project's own `Makefile.am` for other `_LIBADD` or `_LDADD` lines
naming it, and look at whether any program links it directly.

### Nothing else uses it — fold it in

The usual case. The archive was an organisational device inside one library.

Move its `srcs`, `hdrs`, `includes` and `local_defines` into the shared
library's own `cc_library`, delete the separate rule, and remove it from
`deps`. Watch for `copts` or `local_defines` that differ between the two —
if the archive was compiled with different flags, folding changes how those
sources are built and you need the third option instead.

### Several targets use it — `static_deps`

Keep the `cc_library` and name it in the `cc_shared_library`'s `static_deps`.
One definition of the sources, absorbed into the shared library.

Be aware this is a **whole-library** statement and the original build's is
not: a static archive contributes only the members something references, so
the real `.so` typically contains some of the archive's objects and not
others. The link works either way; the resulting library is slightly larger
than the project's own. Say so in your resolution rather than leaving the
difference unrecorded.

### Consumers call into it — export it

Rare for a `noinst_` library, which is by definition not part of the
installed interface. Only choose this if the project's public headers
declare functions the archive defines.

## Two things that will bite

**The dependency may be transitive.** The archive can be reached through
another library rather than named directly by the shared one. Follow the
`deps` chain before folding anything in — the target that actually
references the symbols may not be the one you started from.

**Do not just delete the archive's rule.** Its sources have to end up
somewhere. Dropping them leaves undefined symbols at link, far from this
item, and a green analysis phase makes that look unrelated.

## Not a resolution

Editing the project's `Makefile.am` or `configure.ac`. The input build files
are immutable; the resolution belongs in the generated `BUILD.bazel`.
"#;
