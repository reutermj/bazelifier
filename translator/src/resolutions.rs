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
            filename: "header-visibility.md",
            body: HEADER_VISIBILITY,
        },
        Recipe {
            filename: "ctest-command-not-a-target.md",
            body: CTEST_COMMAND_NOT_A_TARGET,
        },
    ]
}

const README: &str = r#"# Recommended resolutions

This directory holds **recipes** for the kinds of gap that appear in
`../needs_attention/`. Each `needs_attention` item describes one specific
unresolved thing in THIS project; the file here describes how that shape of
gap is usually closed in Bazel.

## How to use these

1. Read the `needs_attention` item. It is the authority on what is actually
   wrong here — the item's own guidance wins over anything in this directory.
2. Find the recipe whose name matches the shape of the gap (the filenames are
   descriptive; there is no index to look up).
3. **Adapt it.** A recipe cannot know this project's macro names, header
   layout, or release. It shows the shape of a correct answer, not a patch to
   apply.

## The rules a resolution must not break

These hold regardless of which recipe you are following:

- **Resolve in the GENERATED output.** Edit this module's `BUILD.bazel` and
  its own copies of files. Never edit the project's `CMakeLists.txt` — it is
  the input being translated, and "fixing" it leaves the next project with
  the same shape just as broken.
- **Do not vendor build-machine results.** Anything the conversion host
  computed (feature-detection values, a generated config header, an absolute
  path) is a fact about that host, not about this project. Baking it in makes
  the module build correctly only where it was converted.
- **Keep the module portable.** No absolute paths, no reference back to the
  converter's checkout. The module has to work when someone drops it into
  their own repo.

## If no recipe fits

That is expected — recipes exist for shapes seen often enough to be worth
writing down, not for everything. Resolve from the item's own guidance.
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

Some projects `configure_file` a checked-in `.in`/`.cmakein`. That is the easy
case: the file is already in this module, point `template =` at it and stop
reading here.

Others **build the template at configure time** and then expand it. When that
happens there is nothing in the module to point `template =` at, and the
files that look like the template are near-misses.

zlib is the worked example. Its `CMakeLists.txt` does, in outline:

```cmake
file(READ  ${zlib_SOURCE_DIR}/zconf.h ZCONF_CONTENT LIMIT 245)   # first 245 bytes
file(WRITE ${zlib_BINARY_DIR}/zconf.h.cmakein ${ZCONF_CONTENT})
file(APPEND ${zlib_BINARY_DIR}/zconf.h.cmakein "#cmakedefine Z_PREFIX 1\n")
file(APPEND ${zlib_BINARY_DIR}/zconf.h.cmakein "#cmakedefine HAVE_STDARG_H 1\n")
file(APPEND ${zlib_BINARY_DIR}/zconf.h.cmakein "#cmakedefine HAVE_UNISTD_H 1\n")
file(READ  ${zlib_SOURCE_DIR}/zconf.h ZCONF_CONTENT OFFSET 244)  # the rest
file(APPEND ${zlib_BINARY_DIR}/zconf.h.cmakein ${OUT_CONTENT})

configure_file(${zlib_BINARY_DIR}/zconf.h.cmakein ${zlib_BINARY_DIR}/zconf.h)
```

That is the checked-in `zconf.h` split at a byte offset with three
`#cmakedefine` lines spliced into the seam — the offset lands just after the
include guard.

**Reproduce the RESULT, not the process.** Do not model `file(READ/WRITE/
APPEND)` or byte offsets. Take the checked-in header as your template, add
the `#cmakedefine` lines the splice would have inserted, and let
`config_header` expand it:

```python
config_header(
    name = "zconf_h",
    template = "zconf.h",   # this module's own copy, with the lines added
    output = "zconf.h",
    probes = [
        "@cc_config//catalog:have_stdarg_h",
        "@cc_config//catalog:have_unistd_h",
    ],
)
```

Work out which lines to add by diffing the project's checked-in header against
the one CMake generated in its build directory. For zlib 1.3.2 the whole
difference is four lines, inserted directly after `#define ZCONF_H`:

```c
/* #undef Z_PREFIX */
#define HAVE_STDARG_H 1
#define HAVE_UNISTD_H 1
```

Edit **this module's copy**. Never the project's upstream sources.

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

    // The recipes exist to be FOUND without an index, so README has to name
    // them. A recipe added without a mention here is invisible in practice.
    #[test]
    fn the_readme_mentions_every_recipe_it_ships_with() {
        let recipes = all();
        let readme = recipes
            .iter()
            .find(|r| r.filename == "README.md")
            .expect("resolutions/ must ship a README");

        for r in recipes.iter().filter(|r| r.filename != "README.md") {
            let stem = r.filename.trim_end_matches(".md");
            assert!(
                readme.body.contains(stem) || readme.body.contains("descriptive"),
                "README gives no way to find {}: an agent has only the filenames",
                r.filename
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

        assert!(
            readme.contains("CMakeLists.txt"),
            "must say not to edit the input build files:\n{readme}"
        );
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
            recipe.contains("Reproduce the RESULT, not the process"),
            "the recipe must tell the agent to reproduce the OUTCOME rather \
             than model file(READ/WRITE/APPEND):\n{recipe}"
        );
        assert!(
            recipe.contains("cc_config"),
            "and name the mechanism that resolves it:\n{recipe}"
        );
        // zlib's trap: the template is assembled at configure time, so
        // `template =` has nothing in the module to point at.
        assert!(
            recipe.contains("build the template at configure time"),
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
