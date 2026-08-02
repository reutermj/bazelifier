//! Internal build-graph model: the contract between any frontend and Bazel
//! codegen.
//!
//! Deliberately states nothing in terms of a particular build system. That was
//! an untested claim while CMake was the only frontend; the Autotools frontend
//! now produces a `BuildGraph` that renders through unmodified codegen, so the
//! neutrality is real rather than aspirational. Adding a field here that only
//! one frontend can populate is how that would be lost.
//!
//! See docs/architecture/cmake-frontend.md and docs/architecture/bazel-codegen.md.

use std::path::{Component, Path, PathBuf};

use crate::needs_attention::NeedsAttention;

/// Whether `path` satisfies the contract every path-valued field of
/// [`Target`] is required to meet: relative to the converted module's root,
/// and not escaping it.
///
/// This is the single invariant standing between the translator and
/// non-portable output. The CMake File API reports a source path relative
/// to the project only when the file is actually inside it and absolute
/// otherwise, so an unvalidated passthrough bakes the build machine's
/// filesystem layout into a module that is supposed to be checked into
/// someone else's repo.
///
/// Bazel only catches PART of this on its own, which is why the check exists
/// at all rather than being left to it: an absolute path in a label
/// attribute is an analysis error, but `includes` is a plain string list and
/// takes one silently, so the module builds on the machine that generated it
/// and nowhere else.
///
/// That asymmetry is a fact about a specific Bazel version, verified and
/// dated in docs/architecture/bazel-codegen.md. Stated there rather than
/// here so there is one place to re-check if a future Bazel starts
/// validating `includes` — at which point the argument for an `assert!` over
/// a `debug_assert!` weakens.
pub fn is_module_relative(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute() && !path.components().any(|c| matches!(c, Component::ParentDir))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TargetKind {
    #[default]
    Executable,
    Library,
    // Other CMake target types (OBJECT_LIBRARY, INTERFACE_LIBRARY, ...) are
    // known hard cases for now — see docs/architecture/cmake-frontend.md.
}

/// One translated build target.
///
/// Every path-valued field except `artifacts` is relative to the converted
/// module's root — see [`is_module_relative`], which is the contract they
/// all have to meet. That root is DERIVED rather than assumed to be the
/// project directory, so these are not the paths either frontend's source
/// reported: `cmake_api::rebase_to_module_root` rewrites them after the fact,
/// while the Autotools frontend rebases as it builds the graph, because
/// recursive make reports each path against the directory its command ran
/// in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub kind: TargetKind,
    /// Whether the project declared this library shared — CMake's
    /// `add_library(... SHARED)`, automake's `lib_LTLIBRARIES`. Only
    /// meaningful when `kind` is [`TargetKind::Library`].
    ///
    /// A separate field rather than a `TargetKind::Library(Linkage)` payload
    /// because linkage does not change which Bazel rule the target becomes —
    /// both forms are a `cc_library`. Shared-ness is an ADDITIONAL
    /// `cc_shared_library` wrapping it plus a `dynamic_deps` edge on the
    /// consumers that link it, so it rides alongside the kind rather than
    /// inside it. See `codegen::render_shared_library`.
    pub is_shared: bool,
    /// The filename a consumer's `DT_NEEDED` will name for this library —
    /// `liblzma.so.5`, not the Bazel target name. `None` when the frontend
    /// could not determine it, or when the target is not shared.
    ///
    /// Carried because the target NAME cannot stand in for it. That name has
    /// already been sanitised for Bazel label legality, and for Autotools it
    /// starts life as an automake target like `liblzma.la` — so the derived
    /// output is `libliblzma.la_shared.so`, which embeds a control-file
    /// extension that means nothing in Bazel and matches no SONAME any
    /// consumer expects.
    ///
    /// The Autotools frontend reads it from the `.la`'s own `dlname=`, which
    /// is the same field ground-truth staging already consults. Empty for
    /// CMake, which has not needed it yet: `add_library(foo SHARED)` produces
    /// `libfoo.so`, and the target name already carries that.
    pub soname: Option<String>,
    /// Private source file paths (compiled, not exposed to consumers).
    pub sources: Vec<String>,
    /// Files this target's sources `#include` TEXTUALLY rather than compile —
    /// a `.c`/`.cc` pulled in with `#include "other.cc"`, which CMake
    /// deliberately keeps out of the target's source list so it is not also
    /// built separately (fmt's `posix-mock-test` does
    /// `#include "../src/os.cc"`).
    ///
    /// Kept apart from `sources` because the two need different Bazel
    /// attributes: a `.cc` in `srcs` is a translation unit Bazel compiles, so
    /// it is never staged as an input to the sibling compile that includes
    /// it. These become `textual_hdrs` — "header files that cannot be
    /// compiled on their own", which is what a textually-included source is.
    pub textual_sources: Vec<String>,
    /// Public header file paths — only ones the PROJECT declared public, and
    /// only from an authoritative declaration. In CMake that is a
    /// `target_sources(... FILE_SET ... TYPE HEADERS)` with
    /// `PUBLIC`/`INTERFACE` visibility, or an `install(FILES ... DESTINATION
    /// <include dir>)` (the latter also covers headers no target enumerated
    /// at all — see `inject_unenumerated_installed_headers`). In automake it
    /// is an `include_HEADERS` primary — the install destination is the same
    /// claim, spelled differently.
    ///
    /// A header with neither signal is NOT here, however obviously public it
    /// looks: being reachable on an include path makes a header an input, not
    /// part of the interface, so those land in `sources` instead. See
    /// docs/architecture/cmake-frontend.md on why that distinction can't be
    /// guessed at.
    pub public_headers: Vec<String>,
    /// Names of other targets in this project that this target links
    /// against, resolved back to target NAMES — from the File API's opaque
    /// dependency ids for CMake, and from the artifacts a link command names
    /// for Autotools.
    pub dependencies: Vec<String>,
    /// Whether this target declared its own module root as an include
    /// directory, which Bazel cannot express.
    ///
    /// `includes = ["."]` is a hard analysis error ("resolves to the workspace
    /// root, which would allow this rule and all of its transitive dependents
    /// to include any file in your workspace"), and Bazel passes only
    /// `-iquote .` for a root package — never `-I`. So a header at the module
    /// root is unreachable by `#include <angled>`, which zlib's `zlib.h` does
    /// for `zconf.h`. Codegen works around it by staging the target's PUBLIC
    /// headers AND every generated config header into a private subdirectory
    /// it can name; see `codegen::render_staged_headers`. Both halves matter:
    /// treating public headers as the whole set left xz's liblzma, which
    /// declares none, with an include path into a directory it was never
    /// given.
    ///
    /// Recorded rather than acted on here because it is a fact about the
    /// project (it asked for its root on the include path), while the
    /// workaround is a fact about Bazel.
    pub needs_root_include: bool,
    /// Include directories the target compiles with — CMake's
    /// `target_include_directories`, or the `-I` flags on an Autotools
    /// compile line. Emitted as
    /// the `includes` attribute, which Bazel propagates transitively to
    /// consumers — so this only needs to be captured on the target that
    /// declared it, not duplicated onto every dependent. See
    /// docs/architecture/cmake-frontend.md.
    pub includes: Vec<String>,
    /// Preprocessor definitions effective on this target's own compilation
    /// (from `target_compile_definitions`), emitted as `local_defines`.
    ///
    /// `local_defines`, not `defines`, and the distinction is deliberate:
    /// the File API reports the flattened *effective* set per target with
    /// the PUBLIC/PRIVATE/INTERFACE origin erased (see
    /// docs/lore/cmake-file-api-compile-definitions-shape.md), so we can't
    /// yet tell which ones propagate. Making them all non-propagating and
    /// letting each converted consumer re-derive its own from its own
    /// compile group is self-consistent — every target gets exactly the set
    /// CMake computed for it. It is wrong only for a consumer *outside* the
    /// converted set, which never sees a PUBLIC define it should have
    /// inherited; recovering propagation (`defines` vs `local_defines`) via
    /// the backtrace graph is a separate step, tracked in bzl-c54.3.
    pub local_defines: Vec<String>,
    /// Build-output artifact paths (e.g. the built binary), relative to
    /// the CMake build directory. Used to locate ground-truth artifacts
    /// for validation — see docs/architecture/build-verification.md.
    pub artifacts: Vec<String>,
}

/// Identifies the standalone Bazel module the translator produces for a
/// CMake project (its own `MODULE.bazel`, independent of bazelifier's own).
/// See docs/architecture/bazel-codegen.md.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInfo {
    /// Derived from CMake's `project()` name.
    pub name: String,
    /// `CMAKE_PROJECT_VERSION`, when the CMake project's `project()` call
    /// specified a `VERSION`. `None` when it didn't — Bazel's `module()`
    /// does not require a version, so codegen omits it rather than
    /// fabricating one.
    pub version: Option<String>,
}

/// One CMake-registered test (`add_test`), recovered from `ctest
/// --show-only=json-v1` rather than the File API, which has no test model —
/// see docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md. Only the
/// subset needed to reproduce the test's pass/fail decision in Bazel is
/// carried; the long tail of CTest properties (FAIL_REGULAR_EXPRESSION,
/// WILL_FAIL, ENVIRONMENT, fixtures, ...) is deferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Test {
    /// The CTest test name (`add_test(NAME ...)`).
    pub name: String,
    /// Name of the generated target this test runs — the basename of the
    /// test command's executable. A test only reaches the model when this
    /// matches a `cc_binary` the module also emits (a test whose command is
    /// anything else is escalated instead, see
    /// `ctest_command_not_a_target_needs_attention`), so the generated
    /// `sh_test` can wrap that binary rather than re-locating it.
    pub target: String,
    /// The test command's executable as CTest reported it, absolute. Kept
    /// alongside `target` because the basename alone cannot answer whether
    /// the command is a binary this module builds: json-c's tests are
    /// checked-in shell scripts in the SOURCE tree
    /// (`<src>/tests/test1.test`) while tinyxml2's is a built executable in
    /// the BUILD tree (`<build>/xmltest`), and the two are indistinguishable
    /// once reduced to a file name. Only the escalation path reads it.
    pub command: String,
    /// The directory the binary must run in, relative to the module root
    /// (from CTest's `WORKING_DIRECTORY`, rebased). The build's runtime data
    /// (e.g. tinyxml2's `resources/`) lives here. Empty means the module
    /// root itself.
    pub working_directory: String,
    /// A substring/regex the binary's output must match for the test to
    /// pass, from CTest's `PASS_REGULAR_EXPRESSION`. `None` when the test
    /// declares none (then the exit code alone decides). This is the
    /// project's own pass criterion, translated rather than invented.
    pub pass_regex: Option<String>,
}

/// How a config-header template DECLARES the names it wants resolved, which
/// decides what an unresolved one looks like in the output.
///
/// Named for the construct rather than the build system, because the two do
/// not line up: autoconf generates headers BOTH ways, `#undef` through
/// `AC_CONFIG_HEADERS` and `@VAR@` through `AC_CONFIG_FILES`, and jansson
/// ships one of each. A variant called `Cmake` would then have to be passed
/// by the Autotools frontend, which reads as a bug and invites someone to
/// "fix" it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigDialect {
    /// `#cmakedefine NAME`, from CMake's `configure_file`. An unresolved one
    /// stays in the output as the literal directive.
    #[default]
    Cmakedefine,
    /// `#undef NAME`, from autoconf's `AC_CONFIG_HEADERS`. An unresolved one
    /// stays `#undef`, so the macro is simply absent — a silent miscompile
    /// rather than a broken build, which is the harder direction.
    Undef,
    /// `@VAR@` substitution only, from `AC_CONFIG_FILES` or a CMake template
    /// with no `#cmakedefine`. An unresolved one is left LITERAL and breaks
    /// every source that includes the header.
    Substitution,
}

/// A `configure_file`-generated config header the translator reproduces via a
/// `cc_config//:config_header` rule, so it's computed against the consumer's
/// toolchain rather than baked from the conversion host. Recovered from the
/// configure trace (not the File API) — see docs/architecture/
/// configure-file-and-toolchain-probes.md. Every path is module-relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigHeader {
    /// The generated header's name (e.g. `config.h`), as the source files
    /// `#include` it and as the emitted rule's `output`.
    pub output: String,
    /// The `.in`/`.cmakein` template it's generated from, module-relative.
    pub template: String,
    /// Where to copy the template FROM, when it is not already in the source
    /// tree at `template`.
    ///
    /// Some projects generate the template itself at configure time, into the
    /// build directory, and then `configure_file` that (zlib assembles
    /// `zconf.h.cmakein` with `file(READ/WRITE/APPEND)`). A Bazel label cannot
    /// point outside the module, so the template is staged in like a source
    /// and `template` names its position once staged.
    ///
    /// Staging the TEMPLATE is not the same as vendoring the generated HEADER,
    /// which stays forbidden: a template carries only `#cmakedefine`
    /// directives, so it is still expanded against the CONSUMER's toolchain by
    /// `cc_config`. The header carries this host's probe answers, which is
    /// what makes copying it wrong. The two look alike in a diff.
    pub template_source: Option<std::path::PathBuf>,
    /// Catalog probe labels (`@cc_config//catalog:have_endian_h`) for the
    /// template's `#cmakedefine`s the catalog covers.
    pub catalog_probes: Vec<String>,
    /// Substitutions the frontend resolved at conversion time — package
    /// metadata, version strings, anything that is a fact about the PROJECT
    /// rather than about the consumer's toolchain — as (name, value).
    ///
    /// **The value is stored exactly as it must appear in the generated
    /// header**, quoting included. That is the invariant, and it is not the
    /// same as "the value the build system reported", because the two
    /// dialects differ:
    ///
    /// - CMake templates carry their own quotes —
    ///   `#cmakedefine FOO "@FOO@"` — so the cache value goes in raw.
    /// - autoconf's `#undef FOO` has nowhere to put them, and `AC_DEFINE`
    ///   emits a C string literal, so a non-numeric value arrives already
    ///   wrapped in `"`. A numeric one does not: `SIZEOF_LONG` and
    ///   `ASSUME_RAM` are used in `#if` and arithmetic, where a string
    ///   literal is a type error rather than a quoting nicety.
    ///
    /// Stated here because the field is shared and the callers disagree,
    /// which is invisible at the type level: `String` on both sides. A
    /// consumer that assumes either convention is wrong for one frontend —
    /// `codegen::render_config_header_assertion` filters short values to
    /// avoid vacuous `grep` needles, and those two quote characters shift
    /// where that threshold falls.
    pub values: Vec<(String, String)>,
    /// Which construct the template declares names with, and so what an
    /// unresolved one leaves behind.
    ///
    /// Carried on the header rather than inferred by codegen, because the
    /// frontend is the only place that knows: it read the template. Codegen
    /// guessed `#cmakedefine` for everything, so four Autotools projects
    /// shipped an assertion that their header contains no `#cmakedefine` —
    /// true of every autoconf template ever written, and therefore a gate
    /// that cannot fail (bzl-lvm).
    pub dialect: ConfigDialect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildGraph {
    pub module: ModuleInfo,
    pub targets: Vec<Target>,
    pub tests: Vec<Test>,
    /// `configure_file`-generated config headers to reproduce; see
    /// [`ConfigHeader`]. Empty for a project with no `configure_file`.
    pub config_headers: Vec<ConfigHeader>,
    /// Tests the project registered that the translator could NOT express,
    /// and escalated instead — a CTest test whose command is a shell script
    /// rather than a binary this module builds.
    ///
    /// Carried on the graph rather than only named in the escalation because
    /// two things downstream need to know they exist. The generated module
    /// must depend on `rules_shell`, since the resolution the escalation
    /// recommends IS an `sh_test` and a module without that dep cannot even
    /// load one. And the files those tests run — the scripts, their helpers,
    /// their expected output — have to be copied in, which
    /// `copy_test_runtime_data` skipped precisely because they were absent
    /// from `tests`.
    ///
    /// So this is not a second list of tests to render. Nothing emits a rule
    /// for these; they exist so the module arrives RESOLVABLE.
    pub unexpressed_tests: Vec<Test>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_module_relative_accepts_paths_inside_the_module() {
        assert!(is_module_relative("src/main.cpp"));
        assert!(is_module_relative("include/greet.hpp"));
        // The widened-root case: a module rooted above the CMake project
        // holds the project's own sources under a subdirectory.
        assert!(is_module_relative("proj/src/main.cpp"));
    }

    // CMake only reports a project-relative path when the file is inside
    // the top-level source dir — an absolute path means it isn't, and a
    // `..` component would escape the module root the same way.
    #[test]
    fn is_module_relative_rejects_paths_outside_the_module() {
        assert!(!is_module_relative("/abs/shared/helper.cpp"));
        assert!(!is_module_relative("../shared/helper.cpp"));
        assert!(!is_module_relative("src/../../escape.cpp"));
    }
}

/// What a frontend hands the rest of the pipeline.
///
/// Lives here rather than in a frontend because it is the CONTRACT between
/// them: `cmake_api` and `autotools` both produce one, and `main` consumes it
/// without knowing which ran.
#[derive(Debug)]
pub struct Discovery {
    pub graph: BuildGraph,
    /// Gaps to escalate for this conversion — see
    /// docs/architecture/needs-attention-interface.md.
    pub needs_attention: Vec<NeedsAttention>,
    /// Absolute path to the converted module's root — where
    /// `copy_referenced_sources` reads the referenced files from. Equal to
    /// the project directory unless the build referenced files above
    /// it that still ship with the project; see `rebase_to_module_root`.
    pub module_root: PathBuf,
}
