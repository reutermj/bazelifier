//! Bazel codegen: renders a standalone Bazel module (its own `MODULE.bazel`
//! and `BUILD.bazel`) for a CMake project's `BuildGraph`. See
//! docs/architecture/bazel-codegen.md — the whole point is that this output
//! must build on its own, with no reference back to bazelifier's own
//! MODULE.bazel/toolchains.

use std::collections::HashSet;

use crate::model::{self, BuildGraph, Target, TargetKind};

// Pinned versions for the toolchain/rules every generated module currently
// depends on. Hardcoded for now since the translator has no per-project
// toolchain-selection mechanism yet — see docs/architecture/bazel-codegen.md.
const RULES_CC_VERSION: &str = "0.2.22";
const LLVM_VERSION: &str = "0.8.14";
// Only pulled in when the module has CTest-registered tests (the generated
// sh_test wrapper needs it), so a plain library/binary module doesn't
// acquire a dep it never uses.
const RULES_SHELL_VERSION: &str = "0.8.0";
// The probing module for configure_file config headers; only depended on when
// the module has one to generate. See docs/architecture/configure-file-and-toolchain-probes.md.
const CC_CONFIG_VERSION: &str = "0.0.0";

pub struct GeneratedModule {
    pub module_bazel: String,
    pub build_bazel: String,
}

/// Renders both files into memory, before `main` writes either of them.
///
/// That ordering is what makes `render_path_list`'s assertions safe to fire
/// as panics: a graph that would produce a non-portable module aborts the
/// conversion with nothing yet on disk, rather than leaving a half-written
/// output tree behind.
pub fn render(graph: &BuildGraph) -> GeneratedModule {
    GeneratedModule {
        module_bazel: render_module_bazel(graph),
        build_bazel: render_build_bazel(graph),
    }
}

/// Normalizes a CMake `project()` name into a valid Bazel module name.
///
/// Bazel's grammar is stricter than CMake's: a module name may contain only
/// lowercase letters, digits, `.`, `-` and `_`, must begin with a lowercase
/// letter, and must end with a lowercase letter or digit. CMake project names
/// are conventionally capitalized (`FMT`, `GTest`, `ZLIB`), so passing one
/// through unchanged produces a `MODULE.bazel` that fails at module
/// RESOLUTION — before any target is analyzed, with an error naming the
/// generated file rather than the project it came from.
///
/// Deliberately total rather than fallible: this is the module's identity, and
/// anything depending on it reads the name back out of the generated
/// `MODULE.bazel` (`validation_workspace.bzl` does exactly that to emit its
/// `bazel_dep`/`local_path_override`). Returning an error would fail a
/// conversion over a cosmetic mismatch, and escalating would ask an agent to
/// invent a name the rest of the output already assumes. The
/// mapping is lossy but deterministic — two different CMake names can collide
/// (`My-Lib` and `my_lib` both become... different, but `MY.LIB` and `my.lib`
/// do not), which is acceptable because a module holds one project.
fn module_name(project_name: &str) -> String {
    let mut name: String = project_name
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            c @ ('a'..='z' | '0'..='9' | '.' | '-' | '_') => c,
            _ => '_',
        })
        .collect();

    // Must begin with a lowercase letter: a leading digit or separator is
    // prefixed rather than stripped, so `3d-engine` stays distinguishable
    // from `d-engine`.
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        name.insert_str(0, "m_");
    }
    // Must end with a lowercase letter or digit.
    while name
        .ends_with(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
    {
        name.pop();
    }
    name
}

fn render_module_bazel(graph: &BuildGraph) -> String {
    // Bazel's `module()` does not require a version, so one is omitted
    // rather than fabricated when the CMake `project()` didn't declare one.
    let version = match &graph.module.version {
        Some(version) => format!("    version = \"{version}\",\n"),
        None => String::new(),
    };

    // rules_shell only when there are tests to wrap — see RULES_SHELL_VERSION.
    let rules_shell = if graph.tests.is_empty() {
        String::new()
    } else {
        format!("bazel_dep(name = \"rules_shell\", version = \"{RULES_SHELL_VERSION}\")\n")
    };

    // cc_config only when there are configure_file config headers to generate
    // (its probing rules produce them against the consumer's toolchain).
    let cc_config = if graph.config_headers.is_empty() {
        String::new()
    } else {
        format!("bazel_dep(name = \"cc_config\", version = \"{CC_CONFIG_VERSION}\")\n")
    };

    format!(
        "module(\n    name = \"{name}\",\n{version})\n\n\
         bazel_dep(name = \"rules_cc\", version = \"{RULES_CC_VERSION}\")\n\
         bazel_dep(name = \"llvm\", version = \"{LLVM_VERSION}\")\n\
         {rules_shell}{cc_config}\n\
         register_toolchains(\"@llvm//toolchain:all\")\n",
        name = module_name(&graph.module.name),
    )
}

/// The `rules_cc` rule a target kind maps to. Single source of truth: the
/// `load()` statements are derived from it too, so a kind can't be emitted
/// as a rule the generated `BUILD.bazel` never loaded.
fn rule_name(kind: &TargetKind) -> &'static str {
    match kind {
        TargetKind::Executable => "cc_binary",
        TargetKind::Library => "cc_library",
    }
}

fn render_build_bazel(graph: &BuildGraph) -> String {
    let mut out = String::new();

    // One load per distinct rule the graph actually uses, sorted so the
    // output doesn't depend on target order (and matches what buildifier
    // would produce anyway).
    let mut rules: Vec<&str> = graph.targets.iter().map(|t| rule_name(&t.kind)).collect();
    // A target with textually-included sources gets a companion cc_library
    // (render_textual_hdrs_library), which needs its load even in a module
    // that otherwise emits only binaries.
    if graph.targets.iter().any(|t| !t.textual_sources.is_empty()) {
        rules.push("cc_library");
    }
    // A shared library is an extra rule alongside its cc_library, so its load
    // is needed even though the target's own kind is Library.
    if graph.targets.iter().any(|t| t.is_shared) {
        rules.push("cc_shared_library");
    }
    rules.sort_unstable();
    rules.dedup();

    for rule in &rules {
        out.push_str(&format!("load(\"@rules_cc//cc:{rule}.bzl\", \"{rule}\")\n"));
    }
    render_test_load(&mut out, !graph.tests.is_empty());
    if !graph.config_headers.is_empty() {
        out.push_str(
            "load(\"@cc_config//cc_config:config_header.bzl\", \"assert_config_header_test\", \"config_header\")\n",
        );
    }
    if !rules.is_empty() || !graph.tests.is_empty() || !graph.config_headers.is_empty() {
        out.push('\n');
    }

    // Config headers first: the cc targets below reference them.
    for config_header in &graph.config_headers {
        render_config_header(&mut out, config_header);
        out.push('\n');
    }

    // Which of this module's targets are shared libraries, so a consumer
    // linking one can be given the dynamic_deps edge that makes Bazel link
    // against the .so instead of absorbing it statically.
    let shared: HashSet<&str> = graph
        .targets
        .iter()
        .filter(|t| t.is_shared)
        .map(|t| t.name.as_str())
        .collect();

    let staged = render_staged_headers(&mut out, graph);

    for (i, target) in graph.targets.iter().enumerate() {
        if i > 0 || staged {
            out.push('\n');
        }
        render_cc_rule(&mut out, target, &graph.config_headers, &shared);
    }

    for test in &graph.tests {
        out.push('\n');
        render_sh_test(&mut out, test);
    }

    out
}

/// Renders one `config_header` rule: the `cc_config` probe-driven
/// reproduction of a `configure_file`-generated header. See
/// docs/architecture/configure-file-and-toolchain-probes.md.
fn render_config_header(out: &mut String, header: &model::ConfigHeader) {
    out.push_str(&format!(
        "config_header(\n    name = \"{}\",\n",
        config_header_name(header)
    ));
    out.push_str(&format!("    output = \"{}\",\n", header.output));
    out.push_str(&format!("    template = \"{}\",\n", header.template));
    render_string_list(out, "probes", &header.catalog_probes);
    if !header.values.is_empty() {
        out.push_str("    values = {\n");
        for (name, value) in &header.values {
            // Escaped for the same reason every other emitted string is: a
            // value comes from the project, not from us. The Autotools
            // frontend takes these verbatim out of make's variable database,
            // where a quote is ordinary content.
            out.push_str(&format!(
                "        \"{}\": \"{}\",\n",
                escape_starlark(name),
                escape_starlark(value)
            ));
        }
        out.push_str("    },\n");
    }
    out.push_str(")\n");

    render_config_header_assertion(out, header);
}

/// Emits an `assert_config_header_test` for a generated config header.
///
/// Without it, nothing checks the header's CONTENT. The runtime comparison
/// sees only a binary's stdout, and a config header can be wrong in ways that
/// change no output at all — zlib's `#if HAVE_UNISTD_H-0` treats an undefined
/// macro as `0`, so a header missing that macro compiles, links, runs, and
/// quietly takes a different code path. Fixtures 012-014 only catch this
/// because they were written to print their macros; real projects are not.
///
/// What it asserts is deliberately toolchain-INDEPENDENT:
///
/// - no `#cmakedefine` and no `@VAR@` survive, which would mean the expansion
///   did not happen at all;
/// - each `values` substitution appears with its value — those come from the
///   CMake cache (version strings and the like), not from probing, so they are
///   the same for every consumer.
///
/// It deliberately does NOT assert `HAVE_UNISTD_H 1`. Whether a probe resolves
/// true is a fact about the CONSUMER's toolchain, and pinning this host's
/// answer would recreate the exact bug the probing design exists to prevent.
fn render_config_header_assertion(out: &mut String, header: &model::ConfigHeader) {
    let name = config_header_name(header);

    let mut must_not_contain = vec!["#cmakedefine".to_string()];
    // A surviving `@NAME@` means the substitution never ran. Only checked for
    // names we actually supply — a template may legitimately contain an `@`
    // that is not a substitution.
    must_not_contain.extend(header.values.iter().map(|(n, _)| format!("@{n}@")));

    // Only values that can actually fail a `grep -qF`. An EMPTY value is
    // vacuous — `grep -qF -- ""` matches any non-empty file — and a very short
    // one ("1", "ON", "OFF") is an unanchored substring that a real header
    // hits by accident. Emitting either produces an assertion that passes
    // regardless of whether the substitution happened, which is worse than
    // emitting nothing: it reads as coverage. See bzl-0pd.
    //
    // The `@VAR@` check in `must_not_contain` still covers those values, and
    // is the stronger test anyway: a surviving placeholder proves the
    // substitution did not run, whatever the value was.
    const MIN_DISTINGUISHING_LEN: usize = 4;
    let must_contain: Vec<String> = header
        .values
        .iter()
        .map(|(_, value)| value)
        .filter(|value| value.len() >= MIN_DISTINGUISHING_LEN)
        .cloned()
        .collect();

    out.push_str(&format!(
        "\nassert_config_header_test(\n    name = \"{name}_test\",\n    header = \":{name}\",\n"
    ));
    render_string_list(out, "must_contain", &must_contain);
    render_string_list(out, "must_not_contain", &must_not_contain);
    out.push_str(")\n");
}

/// The rule name for a config header's target — its output with non-identifier
/// characters replaced, so `config.h` becomes a valid target `config_h` that
/// won't collide with the output file of the same base name.
pub fn config_header_name(header: &model::ConfigHeader) -> String {
    header
        .output
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Renders the `load` for the test wrapper and one `sh_test` per registered
/// test. Placed after the cc rules. The load is only emitted when there are
/// tests, so a test-free module's BUILD.bazel is unchanged.
fn render_test_load(out: &mut String, has_tests: bool) {
    if has_tests {
        out.push_str("load(\"@rules_shell//shell:sh_test.bzl\", \"sh_test\")\n");
    }
}

/// Renders one `sh_test` wrapping a CTest-registered test. It runs the
/// module's own `run_cmake_test.sh` (see codegen::RUN_CMAKE_TEST_SH),
/// passing the binary, the working directory, and the pass regex; `data`
/// carries the binary and the runtime files the test reads/writes under its
/// working directory.
fn render_sh_test(out: &mut String, test: &model::Test) {
    // The binary the test runs is a sibling cc_binary in this same package.
    let binary_label = format!(":{}", test.target);
    // Its runfiles path is `<module>+/<binary>` — but the wrapper derives
    // the module prefix itself, so only the target name (its runfiles
    // basename) is passed. cc_binary's default executable basename is its
    // target name.
    let binary_rel = &test.target;

    // Everything under the working directory is runtime data the binary may
    // read or write. Globbed rather than enumerated because the translator
    // does not model individual data files. The glob root is the working
    // directory, or the whole module when the test runs at the root.
    let data_glob = if test.working_directory.is_empty() {
        "glob([\"**\"], exclude = [\"BUILD.bazel\", \"MODULE.bazel\"])".to_string()
    } else {
        format!("glob([\"{}/**\"])", test.working_directory)
    };

    let pass_regex = test.pass_regex.as_deref().unwrap_or("");

    // A working directory at the module root is passed as "." rather than an
    // empty string: an empty positional arg gets dropped when Bazel tokenizes
    // `args`, which would shift the pass regex into the working-directory
    // slot. "." names the module root unambiguously and never collapses.
    let working_dir = if test.working_directory.is_empty() {
        "."
    } else {
        &test.working_directory
    };

    // The CTest test name usually equals its executable's name (tinyxml2's
    // `xmltest` test runs the `xmltest` binary), which would collide with
    // the cc_binary target in this same package. Suffix keeps them distinct.
    let test_name = format!("{}_test", test.name);

    out.push_str(&format!(
        "sh_test(\n\
         \x20   name = \"{test_name}\",\n\
         \x20   srcs = [\"run_cmake_test.sh\"],\n\
         \x20   args = [\n\
         \x20        \"{binary_rel}\",\n\
         \x20        \"{working_dir}\",\n\
         \x20        \"{pass_regex}\",\n\
         \x20   ],\n\
         \x20   data = [\n\
         \x20        \"{binary_label}\",\n\
         \x20   ] + {data_glob},\n\
         )\n",
    ));
}

/// Escapes a value for a double-quoted Starlark string.
///
/// Needed because a value can legitimately contain quotes: autoconf compiles
/// with `-DPACKAGE_NAME="xz"`, so the define reaching `local_defines` is
/// literally `PACKAGE_NAME="xz"`. Emitted raw that closes the Starlark string
/// early and the generated BUILD file does not parse — which is a silent
/// class of bug, since it depends on a project's own values rather than on
/// anything the translator does.
fn escape_starlark(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Renders one list-valued attribute, one item per line.
///
/// An empty list emits nothing at all rather than `attr = []`. Every
/// attribute the translator writes is optional in `rules_cc` and defaults
/// to empty, so the two mean the same thing to Bazel — and the generated
/// output is meant to be read and maintained by people, for whom a wall of
/// empty attributes on every rule is noise. This is what lets
/// `render_cc_rule` offer every attribute unconditionally and let the
/// target's own data decide which appear.
fn render_string_list(out: &mut String, attr: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("    {attr} = [\n"));
    for item in items {
        out.push_str(&format!("        \"{}\",\n", escape_starlark(item)));
    }
    out.push_str("    ],\n");
}

/// Renders a path-valued attribute, enforcing
/// [`model::is_module_relative`] on every entry.
///
/// This is the last point every path in the generated output passes
/// through, which makes it the one place a guard catches paths from *any*
/// frontend field — including ones added later — rather than only the
/// cases some test happened to enumerate. Two separate bugs (an
/// `OBJECT_LIBRARY`'s generated `.o` paths, and ordinary sources in a
/// sibling directory) both reached `srcs` as absolute paths before this
/// existed.
///
/// A violation is a translator bug, not bad user input — user-input gaps
/// go to `needs_attention/` instead — so this panics rather than
/// degrading. Deliberately a real `assert!` and not a `debug_assert!`,
/// because Bazel catches only part of what it prevents; see
/// [`model::is_module_relative`] for which part. The cost is a couple of
/// string comparisons per emitted path.
fn render_path_list(out: &mut String, attr: &str, paths: &[String]) {
    for path in paths {
        assert!(
            model::is_module_relative(path),
            "codegen emitted a non-module-relative path in `{attr}`: {path:?}. \
             Paths in the generated module must be relative to its root — the frontend \
             should have excluded this one and escalated it via needs_attention/ instead. \
             See model::is_module_relative."
        );
    }
    render_string_list(out, attr, paths);
}

/// Renders `deps`, turning each sibling target name into a same-package
/// Bazel label. Every target a converted module emits lives in that
/// module's one top-level `BUILD.bazel`, so a dependency is always
/// `":name"` — there is no cross-package case to handle yet.
fn render_deps(out: &mut String, deps: &[String]) {
    let labels: Vec<String> = deps.iter().map(|dep| format!(":{dep}")).collect();
    render_string_list(out, "deps", &labels);
}

// Public by default: a converted module is meant to be depended on, both
// by bazelifier's own validation tooling and, as more projects get
// converted, by other converted modules (matching how CMake targets are
// typically visible project-wide unless the project opts into something
// narrower — CMake has no per-target visibility concept of its own to
// translate).
const PUBLIC_VISIBILITY: &str = "    visibility = [\"//visibility:public\"],\n";

/// Renders the `cc_binary`/`cc_library` for one target.
///
/// Both kinds emit the same attributes, in the same order, from the same
/// [`Target`] fields — `hdrs` is the only difference — so they share one
/// renderer rather than being two functions taking parallel `&[String]`
/// arguments. That earlier shape let the same two attributes be passed in
/// a different order by each function (`includes` before `deps` in one,
/// after it in the other), which nothing but the call site's own ordering
/// prevented from being transposed.
///
/// `includes` is emitted for both kinds, not just libraries: Bazel's
/// transitivity supplies a *consumer* with its dependencies' include dirs
/// but never a target with its own, so an `add_executable` carrying its own
/// `target_include_directories()` fails to compile without it.
fn render_cc_rule(
    out: &mut String,
    target: &Target,
    config_headers: &[model::ConfigHeader],
    shared: &HashSet<&str>,
) {
    // A textually-included source needs `textual_hdrs` — "header files that
    // cannot be compiled on their own", which is exactly what a `.cc` pulled
    // in with `#include "other.cc"` is. `textual_hdrs` is a `cc_library`-only
    // attribute (`cc_binary` has neither it nor `hdrs`), so a companion
    // library carries it and this target depends on it. That also keeps the
    // file out of `srcs`, where Bazel would compile it a second time.
    let textual_dep = render_textual_hdrs_library(out, target);

    out.push_str(&format!("{}(\n", rule_name(&target.kind)));
    out.push_str(&format!("    name = \"{}\",\n", target.name));

    // Every source path is asserted module-relative before it reaches srcs;
    // the config headers are added as same-package labels (`:config_h`), which
    // are inputs so the generated header is present and includable when this
    // target compiles. A target that doesn't include a given header is
    // unharmed by having it available. Both are one `srcs` list.
    for path in &target.sources {
        assert!(
            model::is_module_relative(path),
            "codegen emitted a non-module-relative path in `srcs`: {path:?}. See \
             model::is_module_relative."
        );
    }
    let mut srcs: Vec<String> = target.sources.clone();
    srcs.extend(
        config_headers
            .iter()
            .map(|h| format!(":{}", config_header_name(h))),
    );
    // `includes` adds a search path; it does not make the staged files inputs
    // to the compile. Targets with public headers get that for free via `hdrs`
    // below, but a target with none — xz's liblzma — was handed `-I_include`
    // pointing into a directory it had never been given, so `<config.h>` stayed
    // unresolvable while the staged copy sat there unreferenced.
    if staged_for(target, config_headers) && target.public_headers.is_empty() {
        srcs.push(format!(":{STAGED_HEADERS_TARGET}"));
    }
    render_string_list(out, "srcs", &srcs);

    // `hdrs` is a `cc_library`-only attribute; `cc_binary` has none, and
    // Bazel rejects it as an unknown attribute rather than ignoring it.
    if target.kind == TargetKind::Library {
        let mut hdrs = target.public_headers.clone();
        if staged_for(target, config_headers) && !target.public_headers.is_empty() {
            // The staged copies replace the originals: listing both would give
            // every header two labels for the same file.
            hdrs.clear();
            hdrs.push(format!(":{STAGED_HEADERS_TARGET}"));
        }
        render_path_list(out, "hdrs", &hdrs);
    }
    let mut includes = target.includes.clone();
    if staged_for(target, config_headers) {
        includes.push(STAGED_INCLUDE_DIR.to_string());
    }
    render_path_list(out, "includes", &includes);
    // Not render_path_list: a define (`FOO`, `FOO=1`) is not a path and
    // must not be run through the module-relative assertion.
    render_string_list(out, "local_defines", &target.local_defines);
    let mut deps: Vec<String> = target.dependencies.clone();
    deps.extend(textual_dep);
    render_deps(out, &deps);

    // A dependency CMake declared SHARED gets a dynamic_deps edge ALONGSIDE
    // its deps entry, not instead of it: deps carries CcInfo (headers,
    // include paths), which cc_shared_library does not provide, while
    // dynamic_deps carries CcSharedLibraryInfo. Dropping either breaks the
    // build — see docs/architecture/bazel-codegen.md's "Shared libraries".
    //
    // Only on binaries: cc_library has no dynamic_deps attribute, so a
    // LIBRARY linking a shared library cannot express that in Bazel. Nothing
    // in the corpus hits it yet; when something does it needs an escalation
    // rather than this silent static downgrade (bzl-i4i.4).
    if target.kind == TargetKind::Executable {
        let dynamic: Vec<String> = target
            .dependencies
            .iter()
            .filter(|d| shared.contains(d.as_str()))
            .map(|d| format!(":{}", shared_library_name(d)))
            .collect();
        render_string_list(out, "dynamic_deps", &dynamic);
    }

    out.push_str(PUBLIC_VISIBILITY);
    out.push_str(")\n");

    render_shared_library(out, target);
}

/// Where staged public headers land inside the module. Leading underscore so
/// it cannot collide with a directory the project authored (`include/` is
/// common), and so it reads as generated rather than as project layout.
const STAGED_INCLUDE_DIR: &str = "_include";

/// Whether `target` reaches its headers through [`STAGED_INCLUDE_DIR`].
///
/// One predicate rather than a condition repeated at each use, because the
/// three uses — making the genrule an input, replacing `hdrs`, and adding the
/// `includes` entry — have to agree with what [`render_staged_headers`]
/// actually stages: public headers AND every generated config header. They did
/// not agree. Gating on public headers alone gave xz's liblzma, which declares
/// none, an `-I_include` for a directory holding only a config header it could
/// not reach.
fn staged_for(target: &Target, config_headers: &[model::ConfigHeader]) -> bool {
    target.needs_root_include && !(target.public_headers.is_empty() && config_headers.is_empty())
}

/// The single module-wide genrule staging public headers.
///
/// Module-wide rather than per-target because two targets can legitimately
/// export the SAME header — zlib builds `zlib` and `zlibstatic` from one set
/// of sources — and two genrules writing `_include/zlib.h` is a hard analysis
/// error ("generated file ... conflicts with existing generated file").
const STAGED_HEADERS_TARGET: &str = "_staged_hdrs";

/// Emits the module-wide genrule copying public headers into
/// [`STAGED_INCLUDE_DIR`], so an `includes` entry can name the directory they
/// are in. Returns whether it emitted anything.
///
/// **This is a workaround for a rules_cc limitation and is meant to be
/// removed.** A header at the module ROOT cannot be reached by
/// `#include <angled>`: Bazel passes only `-iquote .` for a root package and
/// rejects `includes = ["."]` outright ("resolves to the workspace root, which
/// would allow this rule and all of its transitive dependents to include any
/// file in your workspace"). zlib's `zlib.h` does `#include <zconf.h>`, so
/// without this the module does not compile. When rules_cc supports reaching a
/// module-root header from an angled include, delete this and emit the headers
/// in place — see bzl-i4i.6 for the revert criteria.
///
/// Fires only when the project actually asked for its root on the include
/// path (`Target::needs_root_include`), so a project that names real
/// subdirectories is untouched. The trigger is deliberately broader than
/// necessary — a project can ask for its root and still use only quoted
/// includes, which need no staging — see bzl-i4i.7.
///
/// Of the headers a target enumerates, only the PUBLIC ones are staged:
/// staging everything would recreate the exact objection Bazel raises when it
/// rejects `"."`, exposing the whole tree to dependents. Generated config
/// headers are staged unconditionally alongside them, because a source header
/// that includes one with angle brackets (zlib's `zlib.h` does) cannot resolve
/// it otherwise.
fn render_staged_headers(out: &mut String, graph: &BuildGraph) -> bool {
    if !graph.targets.iter().any(|t| t.needs_root_include) {
        return false;
    }

    let mut headers: Vec<String> = graph
        .targets
        .iter()
        .filter(|t| t.needs_root_include)
        .flat_map(|t| t.public_headers.iter().cloned())
        .collect();
    headers.sort_unstable();
    headers.dedup();

    // A config header is generated, so it is a LABEL rather than a file — but
    // it is just as public (zlib installs zconf.h to its include destination)
    // and `zlib.h` includes it with angle brackets. Left out, the staged
    // `zlib.h` resolves and then fails on `#include <zconf.h>`, which reads as
    // the staging not working at all rather than as one missing header.

    // (source label or path, staged output path). Built as pairs so `outs`
    // and `cmd` cannot disagree — they did, and Bazel reports it only as a
    // missing declared output far from the cause (bzl-5yv).
    let mut staged: Vec<(String, String)> = headers
        .iter()
        .map(|h| (h.clone(), format!("{STAGED_INCLUDE_DIR}/{h}")))
        .collect();
    staged.extend(graph.config_headers.iter().map(|h| {
        (
            format!(":{}", config_header_name(h)),
            format!("{STAGED_INCLUDE_DIR}/{}", h.output),
        )
    }));
    let outs: Vec<String> = staged.iter().map(|(_, o)| o.clone()).collect();

    if staged.is_empty() {
        return false;
    }
    let srcs: Vec<String> = staged.iter().map(|(s, _)| s.clone()).collect();

    out.push_str(
        "# Staged so `includes` can name the directory these headers are in.\n\
         # Bazel rejects `includes = [\".\"]` and passes only `-iquote .` for a\n\
         # root package, so a header at the module root is unreachable by\n\
         # `#include <angled>`. Workaround for a rules_cc limitation; remove it\n\
         # and emit the headers in place once upstream supports this.\n",
    );
    out.push_str(&format!("genrule(\n    name = \"{STAGED_HEADERS_TARGET}\",\n"));
    render_path_list(out, "srcs", &srcs);
    render_path_list(out, "outs", &outs);
    // One explicit `cp` per header, with both paths written out by codegen.
    //
    // A single loop over `$(SRCS)` cannot work: the destination has to be the
    // header's path RELATIVE TO THE MODULE, and deriving that in shell means
    // stripping a prefix that differs between a module built directly and the
    // same module consumed as an external repo (where `$(RULEDIR)` carries an
    // `external/<module>+/` segment). Both dirname- and RULEDIR-stripping
    // attempts failed exactly there, passing in-tree and failing as a
    // dependency (bzl-5yv).
    //
    // `$(location <src>)` and `$(RULEDIR)/<out>` are both resolved by Bazel,
    // so the command carries no path arithmetic of its own.
    out.push_str("    cmd = \"");
    for (src, out_path) in &staged {
        out.push_str(&format!(
            "mkdir -p $$(dirname $(RULEDIR)/{out_path}) && \
             cp $(location {src}) $(RULEDIR)/{out_path} && "
        ));
    }
    out.push_str("true\",\n");
    out.push_str(PUBLIC_VISIBILITY);
    out.push_str(")\n");
    true
}

/// The `cc_shared_library` name for a library target.
fn shared_library_name(target_name: &str) -> String {
    format!("{target_name}_shared")
}

/// Emits the `cc_shared_library` for a target CMake declared SHARED.
///
/// It WRAPS the `cc_library` rather than replacing it: `cc_shared_library`
/// provides `CcSharedLibraryInfo` and not `CcInfo`, so it can never stand in
/// for the library in a consumer's `deps` — putting it there is an analysis
/// error. Both rules are emitted and consumers reference both.
fn render_shared_library(out: &mut String, target: &Target) {
    if !target.is_shared {
        return;
    }
    out.push_str(&format!(
        "\ncc_shared_library(\n    name = \"{}\",\n",
        shared_library_name(&target.name)
    ));
    render_string_list(out, "deps", &[format!(":{}", target.name)]);
    out.push_str(PUBLIC_VISIBILITY);
    out.push_str(")\n");
}

/// Emits a companion `cc_library` holding `target`'s textually-included
/// sources, returning its name for the target's `deps` — or `None` when there
/// are none.
///
/// A separate rule rather than an attribute on the target itself because
/// `textual_hdrs` exists only on `cc_library`, and the targets that textually
/// include a source are usually executables (fmt's `posix-mock-test`).
/// Emitted immediately before its consumer so the two read together.
fn render_textual_hdrs_library(out: &mut String, target: &Target) -> Option<String> {
    if target.textual_sources.is_empty() {
        return None;
    }
    let name = format!("{}_textual", target.name);
    out.push_str(&format!("cc_library(\n    name = \"{name}\",\n"));
    render_path_list(out, "textual_hdrs", &target.textual_sources);
    out.push_str(PUBLIC_VISIBILITY);
    out.push_str(")\n\n");
    Some(name)
}

/// Renders the `BUILD.bazel` for the generated module's `needs_attention/`
/// directory.
///
/// The directory is a Bazel package even when it holds no items, so
/// `@<module>//needs_attention:all` is always a valid label — the
/// validation tooling depends on it unconditionally and inspects its
/// contents at test-runtime (see docs/architecture/build-verification.md).
/// Hence `allow_empty = True`, which is the whole reason this can't just
/// be an `exports_files`.
///
/// `srcs` also includes `MANIFEST` explicitly (not just the `*.md` glob):
/// an empty `glob` can vanish entirely from a consuming test's runfiles
/// rather than leaving behind an empty directory, so "the directory is
/// absent" and "zero items" become indistinguishable to anything gating on
/// presence. `MANIFEST` is a real, always-written file (see
/// `main::write_needs_attention`), so it's guaranteed to survive into
/// runfiles regardless of item count — the validation script checks for
/// this file's presence, not the directory's, before trusting an absence
/// of `.md` items.
pub fn render_needs_attention_build_bazel() -> String {
    "filegroup(\n    name = \"all\",\n    srcs = glob(\n        [\"*.md\"],\n        \
     allow_empty = True,\n    ) + [\"MANIFEST\"],\n    visibility = [\"//visibility:public\"],\n)\n"
        .to_string()
}

/// Renders the `BUILD.bazel` for the generated module's `ground_truth/`
/// directory, exporting the real cmake+ninja-built artifacts so the
/// equivalence tests can reference them (e.g.
/// `@<module>//ground_truth:hello`).
///
/// `executables` are (target name, artifact path) for each executable. Each
/// gets a `filegroup` named after the TARGET pointing at its artifact, because
/// the two can differ: a target CMake builds under a subdirectory has a build
/// path like `apps/json_parse` while its Bazel target name is `json_parse`.
/// The comparison test in `validation_workspace.bzl` references
/// `@module//ground_truth:<cc_binary name>`, i.e. by the target name, so
/// without this alias the label would not resolve for any subdir target
/// (bzl-fxa.13). Naming the filegroup after the target also sidesteps a
/// basename collision between two subdir targets (`a/foo` and `b/foo`): the
/// distinct CMake target names stay distinct.
///
/// `shared_libs` are the staged shared-library files (`libfoo.so.5`, ...) that
/// a dynamically linked ground-truth binary loads at run time, grouped into a
/// `shared_libs` filegroup the comparison test depends on wholesale — the test
/// can't name them individually, so it adds the group to its runfiles and
/// points LD_LIBRARY_PATH at them. Emitted even when empty so the comparison
/// test can reference it unconditionally.
///
/// `artifacts` (every built file) is still `exports_files`d so a path-based
/// reference keeps working and libraries remain exported.
///
/// Deliberately its own nested package rather than entries in the module's
/// top-level `BUILD.bazel`: validation-only targets must never appear in
/// what a user checks into their own repo. See
/// docs/architecture/build-verification.md.
pub fn render_ground_truth_build_bazel(
    artifacts: &[String],
    executables: &[(String, String)],
    shared_libs: &[String],
) -> String {
    let exports = artifacts
        .iter()
        .map(|p| format!("\"{p}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let shared_srcs = shared_libs
        .iter()
        .map(|p| format!("\"{p}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let mut out = format!("exports_files([{exports}])\n");
    // One filegroup per subdir executable, named by target so the comparison
    // test's by-name label resolves regardless of the artifact's build path.
    // Skipped when name == path (a root-level target): `exports_files` already
    // provides a target of that name, and a second one would collide.
    for (name, path) in executables {
        if name == path {
            continue;
        }
        out.push_str(&format!(
            "\nfilegroup(\n    name = \"{name}\",\n    srcs = [\"{path}\"],\n    \
             visibility = [\"//visibility:public\"],\n)\n"
        ));
    }
    out.push_str(&format!(
        "\nfilegroup(\n    name = \"shared_libs\",\n    srcs = [{shared_srcs}],\n    \
         visibility = [\"//visibility:public\"],\n)\n"
    ));
    out
}

/// The wrapper `sh_test` binary the generated module ships to run a
/// CTest-registered test. It exists because a CTest test is more than "run
/// this binary": it runs at a specific working directory, that directory
/// holds runtime data the binary reads AND writes (tinyxml2's xmltest reads
/// resources/*.xml and writes resources/out/, and segfaults if the output
/// dir is missing), and pass/fail is decided by a regex over stdout, not the
/// exit code alone. See docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md.
///
/// Runfiles are read-only, so the data can't be written in place — the
/// script copies the whole module tree from runfiles into a writable temp
/// dir and runs there. Args: $1 = binary runfiles-relative path, $2 =
/// working directory relative to the module root ("" = module root), $3 =
/// pass regex ("" = none, exit code alone decides).
const RUN_CMAKE_TEST_SH: &str = r#"#!/usr/bin/env bash
# Runs a CMake-registered test (see codegen::RUN_CMAKE_TEST_SH and
# docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md). Not `set -e`:
# the binary's own nonzero exit is data this script evaluates, not a reason
# to abort before checking the pass regex.
set -uo pipefail

binary_name="$1"
working_dir="$2"
# Defaulted, not "$3": Bazel DROPS a trailing empty string when it tokenizes
# `args`, so a test with no PASS_REGULAR_EXPRESSION arrives with argc=2 and a
# bare "$3" aborts under `set -u` before the binary ever runs. The
# working-directory slot dodges this by never being empty (codegen sends "."),
# but the regex is genuinely empty whenever CMake declared none — which is the
# common case, and every prior sh_test producer happened to declare one.
pass_regex="${3:-}"

# This wrapper, the test binary, and the module's runtime data are all in
# the same package, so in runfiles they are siblings in this script's own
# directory. Derive the module's runfiles root from $0 rather than from
# TEST_WORKSPACE: when this module is a *dependency* of the workspace under
# test, its files live under runfiles/<canonical_repo>/ (e.g. tinyxml2+/),
# not runfiles/<TEST_WORKSPACE>/ — and the canonical repo name is something
# the module itself cannot know. $0's directory is correct whether the module
# is the root or a dependency.
module_runfiles="$(cd "$(dirname "$0")" && pwd)"

# Runfiles are read-only and the test writes into its working directory
# (tinyxml2's xmltest writes resources/out/), so stage a writable copy of the
# module's DATA and run with that as the working directory.
#
# The binary itself is deliberately NOT run from the staged copy. A binary
# linked against a cc_shared_library finds its .so through an $ORIGIN-relative
# RUNPATH into Bazel's _solib_* tree; moving the executable out from under
# runfiles severs every one of those paths and it dies with "cannot open
# shared object file". Left in place it resolves with no LD_LIBRARY_PATH at
# all. Verified both ways (bzl-i4i.8).
#
# Safe because the test binaries resolve their data relative to the WORKING
# DIRECTORY, not to their own location — checked across the corpus; the only
# argv[0] uses are usage strings and minigzip's basename-based mode switch,
# neither of which cares where the executable sits.
work_root="${TEST_TMPDIR:-$(mktemp -d)}/work"
mkdir -p "${work_root}"
cp -RL "${module_runfiles}/." "${work_root}/" 2>/dev/null || true
# working_dir is "." for the module root, or a module-relative subdir — never
# empty (an empty positional arg would be dropped when Bazel tokenizes args).
run_dir="${work_root}/${working_dir}"
mkdir -p "${run_dir}"

# In runfiles, not in the staged copy — see above.
binary="${module_runfiles}/${binary_name}"
chmod +x "${binary}" 2>/dev/null || true

output="$(cd "${run_dir}" && "${binary}" 2>&1)"
exit_code=$?

echo "${output}"

if [[ -n "${pass_regex}" ]]; then
  if ! grep -qE "${pass_regex}" <<<"${output}"; then
    echo "FAIL: output did not match PASS_REGULAR_EXPRESSION ${pass_regex}" >&2
    exit 1
  fi
  # With a pass regex, CTest treats a match as success regardless of exit
  # code; mirror that (many test harnesses signal via output, not status).
  exit 0
fi

exit "${exit_code}"
"#;

/// Returns the wrapper test runner script the module ships when it has
/// CTest-registered tests. `main` writes it into the module root.
pub fn render_run_cmake_test_sh() -> String {
    RUN_CMAKE_TEST_SH.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModuleInfo;

    // Real values, not invented ones: autoconf compiles xz with
    // -DPACKAGE_NAME="xz", and Windows-style paths reach the same attributes.
    // Emitted raw, either one closes the Starlark string early and the whole
    // module stops parsing — a failure that depends on the project's own
    // values, so no fixture without such a value can catch it.
    #[test]
    fn escape_starlark_escapes_quotes_and_backslashes() {
        assert_eq!(
            escape_starlark(r#"PACKAGE_NAME="xz""#),
            r#"PACKAGE_NAME=\"xz\""#,
            "an unescaped quote closes the string the value sits in"
        );
        assert_eq!(
            escape_starlark(r"C:\path"),
            r"C:\\path",
            "a backslash must be escaped before the quote pass, or the escape \
             it produces is itself re-escaped"
        );
        assert_eq!(
            escape_starlark("plain"),
            "plain",
            "a value needing no escaping must pass through unchanged"
        );
    }

    // The values dict was the one string-emitting path that skipped escaping.
    // The Autotools frontend fills it verbatim from make's variable database,
    // where a quote is ordinary content.
    #[test]
    fn a_config_header_value_containing_a_quote_is_escaped() {
        let mut out = String::new();
        render_config_header(
            &mut out,
            &model::ConfigHeader {
                output: "config.h".to_string(),
                template: "config.h.in".to_string(),
                template_source: None,
                catalog_probes: vec![],
                values: vec![("PACKAGE_NAME".to_string(), "\"xz\"".to_string())],
            },
        );
        assert!(
            out.contains(r#""PACKAGE_NAME": "\"xz\"""#),
            "the values dict must escape like every other emitted string; got:\n{out}"
        );
    }

    fn graph(version: Option<&str>) -> BuildGraph {
        BuildGraph {
            module: ModuleInfo {
                name: "hello_world".to_string(),
                version: version.map(str::to_string),
            },
            tests: vec![],
            config_headers: vec![],
            targets: vec![Target {
                name: "hello".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/main.cpp".to_string()],
                public_headers: vec![],
                dependencies: vec![],
                includes: vec![],
                local_defines: vec![],
                artifacts: vec!["hello".to_string()],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn renders_build_bazel_with_single_cc_binary() {
        let rendered = render(&graph(None)).build_bazel;
        assert_eq!(
            rendered,
            "load(\"@rules_cc//cc:cc_binary.bzl\", \"cc_binary\")\n\n\
             cc_binary(\n    name = \"hello\",\n    srcs = [\n        \"src/main.cpp\",\n    ],\n    visibility = [\"//visibility:public\"],\n)\n"
        );
    }

    #[test]
    fn renders_cc_library_with_hdrs_and_deps() {
        let graph = BuildGraph {
            module: ModuleInfo {
                name: "lib_example".to_string(),
                version: None,
            },
            tests: vec![],
            config_headers: vec![],
            targets: vec![
                Target {
                    name: "greet".to_string(),
                    kind: TargetKind::Library,
                    sources: vec!["src/greet.cpp".to_string()],
                    public_headers: vec!["include/greet.hpp".to_string()],
                    dependencies: vec![],
                    includes: vec!["include".to_string()],
                    local_defines: vec![],
                    artifacts: vec!["libgreet.a".to_string()],
                    ..Default::default()
                },
                Target {
                    name: "hello".to_string(),
                    kind: TargetKind::Executable,
                    sources: vec!["src/main.cpp".to_string()],
                    public_headers: vec![],
                    dependencies: vec!["greet".to_string()],
                    includes: vec![],
                    local_defines: vec![],
                    artifacts: vec!["hello".to_string()],
                    ..Default::default()
                },
            ],
        };

        let rendered = render(&graph).build_bazel;
        assert!(
            rendered.contains("load(\"@rules_cc//cc:cc_binary.bzl\", \"cc_binary\")"),
            "{rendered}"
        );
        assert!(
            rendered.contains("load(\"@rules_cc//cc:cc_library.bzl\", \"cc_library\")"),
            "{rendered}"
        );
        assert!(
            rendered.contains(
                "cc_library(\n    name = \"greet\",\n    srcs = [\n        \"src/greet.cpp\",\n    ],\n    hdrs = [\n        \"include/greet.hpp\",\n    ],\n    includes = [\n        \"include\",\n    ],\n    visibility = [\"//visibility:public\"],\n)\n"
            ),
            "the whole cc_library block must render exactly, attribute order \
             included — a diff anywhere in it fails here:\n{rendered}"
        );
        assert!(
            rendered.contains(
                "cc_binary(\n    name = \"hello\",\n    srcs = [\n        \"src/main.cpp\",\n    ],\n    deps = [\n        \":greet\",\n    ],\n    visibility = [\"//visibility:public\"],\n)\n"
            ),
            "the whole cc_binary block must render exactly:\n{rendered}"
        );
    }

    // An executable with its own `target_include_directories()` (and no
    // dependencies to inherit include dirs from) — the frontend already
    // resolves this correctly (see cmake_api.rs's
    // own_include_dirs_excludes_inherited_and_dedupes, which asserts it for
    // an EXECUTABLE reply); this is the codegen half of the same path.
    // Exercised end to end by tests/fixtures/004-binary-private-include.
    #[test]
    fn renders_cc_binary_with_own_includes() {
        let graph = BuildGraph {
            module: ModuleInfo {
                name: "binary_private_include".to_string(),
                version: None,
            },
            tests: vec![],
            config_headers: vec![],
            targets: vec![Target {
                name: "app".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/main.cpp".to_string(), "inc/cfg.hpp".to_string()],
                public_headers: vec![],
                dependencies: vec![],
                includes: vec!["inc".to_string()],
                local_defines: vec![],
                artifacts: vec!["app".to_string()],
                ..Default::default()
            }],
        };

        let rendered = render(&graph).build_bazel;
        assert!(
            rendered.contains(
                "cc_binary(\n    name = \"app\",\n    srcs = [\n        \"src/main.cpp\",\n        \"inc/cfg.hpp\",\n    ],\n    includes = [\n        \"inc\",\n    ],\n    visibility = [\"//visibility:public\"],\n)\n"
            ),
            "cc_binary dropped its own include dirs:\n{rendered}"
        );
    }

    // Structural guard: a target's own include dirs must survive codegen for
    // EVERY target kind, not just the ones a test happens to cover. The
    // original bug was precisely that `Target.includes` was populated for
    // executables and then only ever read by the cc_library renderer, so a
    // new TargetKind could silently reintroduce it.
    #[test]
    fn renders_own_includes_for_every_target_kind() {
        for kind in [TargetKind::Executable, TargetKind::Library] {
            let graph = BuildGraph {
                module: ModuleInfo {
                    name: "m".to_string(),
                    version: None,
                },
                tests: vec![],
                config_headers: vec![],
                targets: vec![Target {
                    name: "t".to_string(),
                    kind: kind.clone(),
                    sources: vec!["src/t.cpp".to_string()],
                    public_headers: vec![],
                    dependencies: vec![],
                    includes: vec!["inc".to_string()],
                    local_defines: vec![],
                    artifacts: vec![],
                    ..Default::default()
                }],
            };

            let rendered = render(&graph).build_bazel;
            assert!(
                rendered.contains("    includes = [\n        \"inc\",\n    ],\n"),
                "{kind:?} dropped its own include dirs:\n{rendered}"
            );
        }
    }

    // Compile definitions render as `local_defines` (Layer A), for every
    // target kind, and — crucially — a `NAME=VALUE` define is emitted
    // verbatim via render_string_list, NOT render_path_list: a define is not
    // a path, so the module-relative assert has no business seeing it. The
    // assert wouldn't even catch a bad one reliably — `MAX_LEN=64` passes it
    // (not absolute, no `..`), while a define whose value is an absolute path
    // would trip it and panic. See Target::local_defines and
    // docs/lore/cmake-file-api-compile-definitions-shape.md. Exercised end
    // to end by tests/fixtures/009-compile-definitions.
    #[test]
    fn renders_local_defines_for_every_target_kind() {
        for kind in [TargetKind::Executable, TargetKind::Library] {
            let rendered = render(&graph_with(kind.clone(), |t| {
                t.local_defines = vec!["FEATURE_ON".to_string(), "MAX_LEN=64".to_string()]
            }))
            .build_bazel;
            assert!(
                rendered.contains(
                    "    local_defines = [\n        \"FEATURE_ON\",\n        \"MAX_LEN=64\",\n    ],\n"
                ),
                "{kind:?} did not render its compile definitions as local_defines:\n{rendered}"
            );
        }
    }

    // The negative half: a target with no defines emits no `local_defines`
    // attribute at all — same empty-list-is-omitted contract every other
    // optional attribute follows via render_string_list, so a define-less
    // target's output is byte-identical to before this capability existed.
    #[test]
    fn renders_no_local_defines_attribute_when_empty() {
        let rendered = render(&graph(None)).build_bazel;
        assert!(
            !rendered.contains("local_defines"),
            "a target with no compile definitions must not emit a local_defines attribute:\n{rendered}"
        );
    }

    // `cc_binary` and `cc_library` share one renderer, so the one attribute
    // that is NOT shared needs pinning: `cc_binary` has no `hdrs` attribute
    // and Bazel fails analysis on an unknown one. CMake will happily accept
    // `target_sources(<exe> PUBLIC FILE_SET ... TYPE HEADERS ...)`, so a
    // populated `public_headers` on an executable is reachable input, not a
    // hypothetical.
    #[test]
    fn cc_binary_never_renders_hdrs() {
        let rendered = render(&graph_with(TargetKind::Executable, |t| {
            t.public_headers = vec!["include/api.hpp".to_string()]
        }))
        .build_bazel;

        assert!(rendered.contains("cc_binary("), "{rendered}");
        assert!(
            !rendered.contains("hdrs"),
            "cc_binary has no hdrs attribute; Bazel rejects it:\n{rendered}"
        );
    }

    fn graph_with_test(test: model::Test) -> BuildGraph {
        BuildGraph {
            module: ModuleInfo {
                name: "m".to_string(),
                version: None,
            },
            targets: vec![Target {
                name: "xmltest".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/xmltest.cpp".to_string()],
                public_headers: vec![],
                dependencies: vec![],
                includes: vec![],
                local_defines: vec![],
                artifacts: vec!["xmltest".to_string()],
                ..Default::default()
            }],
            tests: vec![test],
            config_headers: vec![],
        }
    }

    // bzl-fxa.20: model::Test::command says "only the escalation path reads
    // it", which is a claim about codegen, so codegen is where it is checked.
    // The command is an absolute path on the conversion machine before
    // rebasing and a source-tree path after — neither is a Bazel label, so
    // rendering it would emit something unresolvable.
    #[test]
    fn the_test_command_never_reaches_the_generated_build_file() {
        let graph = graph_with_test(model::Test {
            name: "xmltest".to_string(),
            target: "xmltest".to_string(),
            command: "tests/run-xmltest.sh".to_string(),
            working_directory: String::new(),
            pass_regex: None,
        });
        let generated = render(&graph);

        assert!(
            !generated.build_bazel.contains("run-xmltest.sh"),
            "the CTest command is escalation evidence, not codegen input; \
             emitting it would put a non-label path in a rule:\n{}",
            generated.build_bazel
        );
    }

    // A CTest test's name need not match the binary it runs. Every other test
    // here uses tinyxml2's shape, where they are both "xmltest" — which
    // collapses the two identifiers and hid a real bug in
    // validation_workspace.bzl, whose dedup recovered the binary by stripping
    // `_test` off the test name and so silently got the TEST name back.
    #[test]
    fn a_test_named_differently_from_its_binary_still_names_the_binary_in_args() {
        let graph = graph_with_test(model::Test {
            name: "app_test".to_string(),
            target: "xmltest".to_string(),
            command: "/abs/build/xmltest".to_string(),
            working_directory: String::new(),
            pass_regex: None,
        });
        let generated = render(&graph);

        assert!(
            generated.build_bazel.contains("name = \"app_test_test\""),
            "the rule is named after the CTest test:\n{}",
            generated.build_bazel
        );
        // Scoped to the args block: the graph also emits cc_binary(name =
        // "xmltest"), so a bare `contains("xmltest")` over the whole file is
        // satisfied by that rule and asserts nothing about the args.
        let args_block = generated
            .build_bazel
            .split("args = [")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("the sh_test must render an args list");
        assert!(
            args_block.contains("\"xmltest\","),
            "the first arg is the BINARY, which is the only place the binary \
             name survives — stripping `_test` off the rule name would yield \
             `app_test`, not `xmltest`. args block was:\n{args_block}"
        );
        assert!(
            !args_block.contains("\"app_test\","),
            "and the TEST name must not appear there:\n{args_block}"
        );
    }

    // A test with no PASS_REGULAR_EXPRESSION renders an empty third arg, and
    // Bazel DROPS a trailing empty string when it tokenizes `args` — so the
    // wrapper receives argc=2 and must not index $3 bare under `set -u`.
    // tinyxml2 declares a pass regex, so this shape went unexercised until
    // fixture 022 and every generated sh_test without one aborted before
    // running its binary.
    #[test]
    fn the_test_wrapper_tolerates_a_dropped_trailing_pass_regex() {
        assert!(
            RUN_CMAKE_TEST_SH.contains("pass_regex=\"${3:-}\""),
            "the wrapper must default the pass-regex arg rather than read $3 \
             bare, because Bazel drops the trailing empty arg:\n{RUN_CMAKE_TEST_SH}"
        );
    }

    #[test]
    fn renders_sh_test_and_rules_shell_dep_for_a_registered_test() {
        let graph = graph_with_test(model::Test {
            name: "xmltest".to_string(),
            target: "xmltest".to_string(),
            command: "/abs/build/xmltest".to_string(),
            working_directory: String::new(),
            pass_regex: Some(", Fail 0".to_string()),
        });
        let generated = render(&graph);

        assert!(
            generated.module_bazel.contains("rules_shell"),
            "a module with a test must depend on rules_shell:\n{}",
            generated.module_bazel
        );
        assert!(
            generated
                .build_bazel
                .contains("load(\"@rules_shell//shell:sh_test.bzl\", \"sh_test\")"),
            "the sh_test rule must be loaded:\n{}",
            generated.build_bazel
        );
        // The test target is suffixed to avoid colliding with the cc_binary
        // of the same name, and the working dir renders as "." (not an empty
        // arg, which Bazel would drop and shift the pass regex into).
        assert!(
            generated.build_bazel.contains("name = \"xmltest_test\""),
            "the test target must be name-suffixed:\n{}",
            generated.build_bazel
        );
        assert!(
            generated
                .build_bazel
                .contains("\"xmltest\",\n         \".\",\n         \", Fail 0\","),
            "args must be [binary, \".\" for module root, pass regex]:\n{}",
            generated.build_bazel
        );
    }

    // Both directions: a module with NO tests gets neither the rules_shell
    // dep nor the sh_test load — a plain library/binary module is unchanged.
    #[test]
    fn renders_no_test_scaffolding_when_there_are_no_tests() {
        let generated = render(&graph(None));
        assert!(
            !generated.module_bazel.contains("rules_shell"),
            "a test-free module must not depend on rules_shell:\n{}",
            generated.module_bazel
        );
        assert!(
            !generated.build_bazel.contains("sh_test"),
            "a test-free module must not load or emit sh_test:\n{}",
            generated.build_bazel
        );
    }

    fn graph_with(kind: TargetKind, mutate: impl FnOnce(&mut Target)) -> BuildGraph {
        let mut target = Target {
            name: "t".to_string(),
            kind,
            sources: vec!["src/t.cpp".to_string()],
            public_headers: vec![],
            dependencies: vec![],
            includes: vec![],
            local_defines: vec![],
            artifacts: vec![],
            ..Default::default()
        };
        mutate(&mut target);
        BuildGraph {
            module: ModuleInfo {
                name: "m".to_string(),
                version: None,
            },
            tests: vec![],
            config_headers: vec![],
            targets: vec![target],
        }
    }

    // One case per path-valued attribute. `includes` is the one Bazel
    // would accept silently, which is why codegen has to be what refuses
    // it — see model::is_module_relative.
    #[test]
    #[should_panic(expected = "non-module-relative path in `srcs`")]
    fn absolute_path_in_srcs_is_refused() {
        render(&graph_with(TargetKind::Executable, |t| {
            t.sources = vec!["/abs/build/generated.cpp".to_string()]
        }));
    }

    #[test]
    #[should_panic(expected = "non-module-relative path in `hdrs`")]
    fn absolute_path_in_hdrs_is_refused() {
        render(&graph_with(TargetKind::Library, |t| {
            t.public_headers = vec!["/abs/include/greet.hpp".to_string()]
        }));
    }

    #[test]
    #[should_panic(expected = "non-module-relative path in `includes`")]
    fn absolute_path_in_includes_is_refused() {
        render(&graph_with(TargetKind::Library, |t| {
            t.includes = vec!["/abs/include".to_string()]
        }));
    }

    // A relative path can still escape the module root, and unlike an
    // absolute path it looks harmless.
    #[test]
    #[should_panic(expected = "non-module-relative path in `srcs`")]
    fn parent_escaping_path_in_srcs_is_refused() {
        render(&graph_with(TargetKind::Executable, |t| {
            t.sources = vec!["../shared/helper.cpp".to_string()]
        }));
    }

    // The positive half: nothing in a fully-populated, well-formed graph
    // renders as an absolute path. Guards against a future attribute being
    // added that bypasses render_path_list.
    #[test]
    fn well_formed_graph_renders_no_absolute_paths() {
        let graph = BuildGraph {
            module: ModuleInfo {
                name: "m".to_string(),
                version: Some("1.0.0".to_string()),
            },
            tests: vec![],
            config_headers: vec![],
            targets: vec![
                Target {
                    name: "lib".to_string(),
                    kind: TargetKind::Library,
                    sources: vec!["src/lib.cpp".to_string()],
                    public_headers: vec!["include/lib.hpp".to_string()],
                    dependencies: vec![],
                    includes: vec!["include".to_string()],
                    local_defines: vec![],
                    artifacts: vec!["liblib.a".to_string()],
                    ..Default::default()
                },
                Target {
                    name: "app".to_string(),
                    kind: TargetKind::Executable,
                    sources: vec!["src/main.cpp".to_string()],
                    public_headers: vec![],
                    dependencies: vec!["lib".to_string()],
                    includes: vec!["inc".to_string()],
                    local_defines: vec![],
                    artifacts: vec!["app".to_string()],
                    ..Default::default()
                },
            ],
        };

        let rendered = render(&graph);
        for (what, text) in [
            ("BUILD.bazel", &rendered.build_bazel),
            ("MODULE.bazel", &rendered.module_bazel),
        ] {
            for quoted in text.split('"').skip(1).step_by(2) {
                assert!(
                    !quoted.starts_with('/') || quoted.starts_with("//"),
                    "{what} contains an absolute path: {quoted:?}"
                );
            }
        }
    }

    // The empty case is the one that matters: the validation tooling
    // depends on `@<module>//needs_attention:all` unconditionally, so a
    // conversion with nothing to triage still has to produce a package
    // whose glob is legal.
    #[test]
    fn needs_attention_build_bazel_globs_allow_empty() {
        let rendered = render_needs_attention_build_bazel();
        assert!(rendered.contains("name = \"all\""), "{rendered}");
        assert!(rendered.contains("allow_empty = True"), "{rendered}");
        assert!(rendered.contains("\"MANIFEST\""), "{rendered}");
    }

    #[test]
    fn ground_truth_build_bazel_exports_every_artifact() {
        // A root-level executable (name == artifact path): no extra filegroup,
        // because exports_files already provides a `:hello` target and a second
        // one of that name would collide.
        let rendered = render_ground_truth_build_bazel(
            &["hello".to_string(), "libgreet.a".to_string()],
            &[("hello".to_string(), "hello".to_string())],
            &[],
        );
        assert!(
            rendered.contains("exports_files([\"hello\", \"libgreet.a\"])"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("name = \"hello\""),
            "a root-level target needs no filegroup (would collide with exports_files):\n{rendered}"
        );
        // A static-only module still gets the (empty) shared_libs filegroup so
        // the comparison test can depend on it unconditionally.
        assert!(rendered.contains("name = \"shared_libs\""), "{rendered}");
        assert!(rendered.contains("srcs = []"), "{rendered}");
    }

    #[test]
    fn ground_truth_build_bazel_aliases_subdir_executables_by_target_name() {
        // bzl-fxa.13: a target CMake builds under a subdir has artifact path
        // `apps/json_parse` but Bazel target name `json_parse`. The comparison
        // test references `//ground_truth:json_parse`, so a filegroup named
        // after the TARGET must point at the subdir artifact — otherwise the
        // label doesn't resolve.
        let rendered = render_ground_truth_build_bazel(
            &["apps/json_parse".to_string()],
            &[("json_parse".to_string(), "apps/json_parse".to_string())],
            &[],
        );
        assert!(
            rendered.contains(
                "filegroup(\n    name = \"json_parse\",\n    srcs = [\"apps/json_parse\"],"
            ),
            "a subdir executable must be exposed by its target name:\n{rendered}"
        );
    }

    #[test]
    fn ground_truth_build_bazel_groups_shared_libraries() {
        // The staged .so chain is both exported and collected into shared_libs,
        // which the comparison test adds to its runfiles for LD_LIBRARY_PATH.
        let rendered = render_ground_truth_build_bazel(
            &[
                "app".to_string(),
                "libgreet.so".to_string(),
                "libgreet.so.5".to_string(),
                "libgreet.so.5.2.0".to_string(),
            ],
            &[("app".to_string(), "app".to_string())],
            &[
                "libgreet.so".to_string(),
                "libgreet.so.5".to_string(),
                "libgreet.so.5.2.0".to_string(),
            ],
        );
        assert!(
            rendered.contains("srcs = [\"libgreet.so\", \"libgreet.so.5\", \"libgreet.so.5.2.0\"]"),
            "the shared_libs filegroup must list every staged .so name:\n{rendered}"
        );
    }

    // A fixture whose targets produce no artifacts still gets the package.
    #[test]
    fn ground_truth_build_bazel_handles_no_artifacts() {
        let rendered = render_ground_truth_build_bazel(&[], &[], &[]);
        assert!(rendered.contains("exports_files([])"), "{rendered}");
        assert!(rendered.contains("name = \"shared_libs\""), "{rendered}");
    }

    #[test]
    fn renders_module_bazel_without_version_when_absent() {
        let rendered = render(&graph(None)).module_bazel;
        let module_block = rendered.split(")\n\n").next().unwrap();
        assert!(module_block.contains("name = \"hello_world\""), "{module_block}");
        assert!(
            !module_block.contains("version ="),
            "a project with no CMAKE_PROJECT_VERSION must omit version entirely:\n{module_block}"
        );
        assert!(rendered.contains("bazel_dep(name = \"rules_cc\""), "{rendered}");
        assert!(rendered.contains("bazel_dep(name = \"llvm\""), "{rendered}");
        assert!(
            rendered.contains("register_toolchains(\"@llvm//toolchain:all\")"),
            "{rendered}"
        );
    }

    // bzl-2wi.1: Bazel's module-name grammar is stricter than CMake's project
    // name. fmt declares project(FMT), and an unnormalized name fails at
    // module RESOLUTION in the validation root — taking down every fixture,
    // not just the offending one.
    #[test]
    fn module_name_normalizes_a_cmake_project_name_for_bazel() {
        // The live case.
        assert_eq!(module_name("FMT"), "fmt");
        // Mixed case and characters Bazel allows are preserved lowercased.
        assert_eq!(module_name("TinyXML-2"), "tinyxml-2");
        assert_eq!(module_name("json-c"), "json-c");
        assert_eq!(module_name("My.Lib_1"), "my.lib_1");
        // Anything else becomes an underscore rather than vanishing, so two
        // distinct project names stay distinct.
        assert_eq!(module_name("My Lib++"), "my_lib");
        // Must begin with a lowercase letter: prefixed, not stripped, so
        // `3d` does not collide with `d`.
        assert_eq!(module_name("3D"), "m_3d");
        assert_eq!(module_name("_private"), "m__private");
        // Must end with a letter or digit.
        assert_eq!(module_name("trailing-"), "trailing");
    }

    #[test]
    fn renders_module_bazel_uses_the_normalized_name() {
        let mut g = graph(None);
        g.module.name = "FMT".to_string();
        let rendered = render(&g).module_bazel;
        assert!(
            rendered.contains("name = \"fmt\""),
            "an unnormalized name makes the generated MODULE.bazel unloadable, \
             and the error names this file rather than the CMake project:\n{rendered}"
        );
    }

    // bzl-2wi.5: a textually-included source cannot go in `srcs` — Bazel
    // treats a .cc there as a translation unit, compiling it a second time
    // and never staging it for the compile that includes it. It goes in
    // `textual_hdrs`, which only cc_library has, hence the companion rule.
    #[test]
    fn a_textually_included_source_becomes_a_companion_textual_hdrs_library() {
        let mut g = graph(None);
        g.targets = vec![Target {
            name: "impl-test".to_string(),
            kind: TargetKind::Executable,
            sources: vec!["test/impl-test.c".to_string()],
            textual_sources: vec!["src/impl.c".to_string()],
            ..Default::default()
        }];
        let rendered = render(&g).build_bazel;

        assert!(
            rendered.contains(
                "cc_library(\n    name = \"impl-test_textual\",\n    textual_hdrs = [\n        \"src/impl.c\",\n    ],"
            ),
            "the companion library must carry the file in textual_hdrs:\n{rendered}"
        );
        assert!(
            rendered.contains("\":impl-test_textual\""),
            "the target must depend on its companion, or the file is never \
             staged for the compile that includes it:\n{rendered}"
        );
        assert!(
            !rendered.contains("        \"src/impl.c\",\n    ],\n    deps"),
            "the file must NOT also appear in srcs — Bazel would compile it a \
             second time, producing duplicate symbols:\n{rendered}"
        );
        assert!(
            rendered.contains("load(\"@rules_cc//cc:cc_library.bzl\", \"cc_library\")"),
            "a binary-only module still needs the cc_library load once a \
             companion is emitted:\n{rendered}"
        );
    }

    // bzl-5yv: `outs` kept a header's subdirectory while the copy flattened
    // it, so Bazel failed with "declared output ... was not created". zlib hid
    // it because its public headers sit at the module root, where the two
    // spellings coincide. Also pins the negative bzl-41q found missing: a
    // project that never asked for its root on the include path gets nothing.
    #[test]
    fn staged_headers_keep_their_subdirectory_and_only_fire_when_needed() {
        let staged_target = |root: bool| {
            let mut g = graph(None);
            g.targets = vec![Target {
                name: "lib".to_string(),
                kind: TargetKind::Library,
                needs_root_include: root,
                sources: vec!["lib.c".to_string()],
                public_headers: vec!["proj/api.h".to_string()],
                ..Default::default()
            }];
            render(&g).build_bazel
        };

        let rendered = staged_target(true);
        assert!(
            rendered.contains("\"_include/proj/api.h\","),
            "the staged output must keep the header's directory, or an angled \
             #include <proj/api.h> cannot resolve:\n{rendered}"
        );
        assert!(
            rendered.contains("cp $(location proj/api.h) $(RULEDIR)/_include/proj/api.h"),
            "and the copy must write exactly the path `outs` declares — when \
             they disagree Bazel reports only a missing output, far from the \
             cause:\n{rendered}"
        );

        let without = staged_target(false);
        assert!(
            !without.contains("_staged_hdrs"),
            "a project that never asked for its module root on the include \
             path must not get a staging genrule — the workaround exists only \
             for the case Bazel cannot express:\n{without}"
        );
    }

    // bzl-0pd: an empty value made `grep -qF -- ""` match any non-empty file,
    // so the assertion passed whether or not the substitution ran. Short
    // values ("1", "OFF") are unanchored substrings a real header hits by
    // accident. Both read as coverage while checking nothing.
    #[test]
    fn config_header_assertion_omits_values_that_cannot_fail() {
        let mut g = graph(None);
        g.config_headers = vec![model::ConfigHeader {
            output: "config.h".to_string(),
            template: "config.h.in".to_string(),
            template_source: None,
            catalog_probes: vec![],
            values: vec![
                ("VERSION".to_string(), "3.7.0".to_string()),
                ("EMPTY".to_string(), String::new()),
                ("FLAG".to_string(), "1".to_string()),
                ("OPTION".to_string(), "OFF".to_string()),
            ],
        }];
        let rendered = render(&g).build_bazel;

        assert!(
            rendered.contains("must_contain = [\n        \"3.7.0\",\n    ],"),
            "a distinguishing value is still asserted, and is the only entry:\n{rendered}"
        );
        // Scoped to must_contain: the same literals legitimately appear
        // elsewhere (must_not_contain carries every @NAME@).
        let must_contain = rendered
            .split("must_contain = [")
            .nth(1)
            .and_then(|rest| rest.split("],").next())
            .expect("the assertion must render a must_contain list");
        for vacuous in ["\"\"", "\"1\"", "\"OFF\""] {
            assert!(
                !must_contain.contains(vacuous),
                "{vacuous} cannot fail a grep -qF against a real header, so \
                 emitting it reads as coverage while checking nothing:\n\
                 must_contain = [{must_contain}]"
            );
        }
        // The placeholder check is what actually proves substitution ran, and
        // it must still cover every value including the omitted ones.
        for name in ["@VERSION@", "@EMPTY@", "@FLAG@", "@OPTION@"] {
            assert!(
                rendered.contains(name),
                "{name} must stay in must_not_contain — a surviving placeholder \
                 proves the substitution did not run, whatever the value:\n{rendered}"
            );
        }
    }

    // bzl-i4i.1: linkage is not a different Bazel rule — both forms are a
    // cc_library — so a shared library silently converting to static builds
    // fine and produces identical output. Both directions are pinned here
    // because the runtime comparison provably cannot see the difference.
    #[test]
    fn a_shared_library_gets_a_cc_shared_library_and_its_consumer_gets_dynamic_deps() {
        let mut g = graph(None);
        g.targets = vec![
            Target {
                name: "greet_shared".to_string(),
                kind: TargetKind::Library,
                is_shared: true,
                sources: vec!["lib/greet.c".to_string()],
                ..Default::default()
            },
            Target {
                name: "greet_static".to_string(),
                kind: TargetKind::Library,
                sources: vec!["lib/greet.c".to_string()],
                ..Default::default()
            },
            Target {
                name: "app_shared".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["app/main.c".to_string()],
                dependencies: vec!["greet_shared".to_string()],
                ..Default::default()
            },
            Target {
                name: "app_static".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["app/main.c".to_string()],
                dependencies: vec!["greet_static".to_string()],
                ..Default::default()
            },
        ];
        let rendered = render(&g).build_bazel;

        assert!(
            rendered.contains("cc_shared_library(\n    name = \"greet_shared_shared\","),
            "a SHARED library needs a cc_shared_library wrapping it:\n{rendered}"
        );
        assert!(
            rendered.contains("load(\"@rules_cc//cc:cc_shared_library.bzl\", \"cc_shared_library\")"),
            "and its load:\n{rendered}"
        );
        assert!(
            !rendered.contains("name = \"greet_static_shared\""),
            "a STATIC library must NOT get one:\n{rendered}"
        );
        assert!(
            rendered.contains("dynamic_deps = [\n        \":greet_shared_shared\",\n    ],"),
            "the consumer of the shared library needs the dynamic_deps edge, \
             or Bazel absorbs the library statically:\n{rendered}"
        );
        // deps AND dynamic_deps, not one or the other: cc_shared_library
        // provides CcSharedLibraryInfo and not CcInfo, so dropping deps loses
        // the headers and is an analysis error.
        assert!(
            rendered.contains("\":greet_shared\","),
            "the consumer must ALSO keep its deps edge for CcInfo:\n{rendered}"
        );
        let static_consumer = rendered
            .split("name = \"app_static\"")
            .nth(1)
            .expect("app_static must render");
        assert!(
            !static_consumer.contains("dynamic_deps"),
            "a consumer of a STATIC library must not get dynamic_deps:\n{rendered}"
        );

        // bzl-41q: a LIBRARY depending on a shared library must not get
        // dynamic_deps either — cc_library has no such attribute and Bazel
        // rejects it as a hard analysis error. The check above cannot catch
        // that: app_static is an executable, so the Executable guard is not
        // what suppresses it there.
        let mut with_lib = g.clone();
        with_lib.targets.push(Target {
            name: "midlib".to_string(),
            kind: TargetKind::Library,
            sources: vec!["mid.c".to_string()],
            dependencies: vec!["greet_shared".to_string()],
            ..Default::default()
        });
        let rendered = render(&with_lib).build_bazel;
        let midlib = rendered
            .split("name = \"midlib\"")
            .nth(1)
            .expect("midlib must render");
        assert!(
            !midlib.contains("dynamic_deps"),
            "cc_library has no dynamic_deps attribute — emitting it fails \
             analysis outright:\n{rendered}"
        );
    }

    // bzl-fxa.22: nothing checked a generated config header's CONTENT. The
    // runtime comparison sees only stdout, and zlib's `#if HAVE_UNISTD_H-0`
    // treats a missing macro as 0 — so a wrong header runs and is wrong.
    #[test]
    fn a_config_header_gets_an_assertion_that_cannot_bake_in_host_answers() {
        let mut g = graph(None);
        g.config_headers = vec![model::ConfigHeader {
            output: "config.h".to_string(),
            template: "config.h.in".to_string(),
            template_source: None,
            catalog_probes: vec!["@cc_config//catalog:have_unistd_h".to_string()],
            values: vec![("PROJECT_VERSION".to_string(), "3.7.0".to_string())],
        }];
        let rendered = render(&g).build_bazel;

        assert!(
            rendered.contains("assert_config_header_test(\n    name = \"config_h_test\","),
            "every config_header needs a companion assertion:\n{rendered}"
        );
        assert!(
            rendered.contains("\"assert_config_header_test\", \"config_header\""),
            "and the load must bring both rules in:\n{rendered}"
        );
        // A surviving directive means the expansion never ran.
        assert!(
            rendered.contains("\"#cmakedefine\""),
            "must assert no #cmakedefine survives:\n{rendered}"
        );
        assert!(
            rendered.contains("\"@PROJECT_VERSION@\""),
            "must assert the substitution actually happened:\n{rendered}"
        );
        assert!(
            rendered.contains("must_contain = [\n        \"3.7.0\",\n    ],"),
            "cache-derived values are toolchain-independent, so their presence \
             is safe to assert:\n{rendered}"
        );
        // The whole point of probing is that this differs per consumer.
        // Asserting it would recreate the bug cc_config exists to prevent.
        assert!(
            !rendered.contains("HAVE_UNISTD_H 1") && !rendered.contains("have_unistd_h 1"),
            "must NOT assert a probe's resolved value — that is a fact about \
             the consumer's toolchain, not about this project:\n{rendered}"
        );
    }

    #[test]
    fn renders_module_bazel_with_version_when_present() {
        let rendered = render(&graph(Some("1.2.3"))).module_bazel;
        assert!(rendered.contains("version = \"1.2.3\""), "{rendered}");
    }
}
