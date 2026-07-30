//! Bazel codegen: renders a standalone Bazel module (its own `MODULE.bazel`
//! and `BUILD.bazel`) for a CMake project's `BuildGraph`. See
//! docs/architecture/bazel-codegen.md — the whole point is that this output
//! must build on its own, with no reference back to bazelifier's own
//! MODULE.bazel/toolchains.

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

    format!(
        "module(\n    name = \"{name}\",\n{version})\n\n\
         bazel_dep(name = \"rules_cc\", version = \"{RULES_CC_VERSION}\")\n\
         bazel_dep(name = \"llvm\", version = \"{LLVM_VERSION}\")\n\
         {rules_shell}\n\
         register_toolchains(\"@llvm//toolchain:all\")\n",
        name = graph.module.name,
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
    rules.sort_unstable();
    rules.dedup();

    for rule in &rules {
        out.push_str(&format!("load(\"@rules_cc//cc:{rule}.bzl\", \"{rule}\")\n"));
    }
    render_test_load(&mut out, !graph.tests.is_empty());
    if !rules.is_empty() || !graph.tests.is_empty() {
        out.push('\n');
    }

    for (i, target) in graph.targets.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        render_cc_rule(&mut out, target);
    }

    for test in &graph.tests {
        out.push('\n');
        render_sh_test(&mut out, test);
    }

    out
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
        out.push_str(&format!("        \"{item}\",\n"));
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
fn render_cc_rule(out: &mut String, target: &Target) {
    out.push_str(&format!("{}(\n", rule_name(&target.kind)));
    out.push_str(&format!("    name = \"{}\",\n", target.name));
    render_path_list(out, "srcs", &target.sources);
    // `hdrs` is a `cc_library`-only attribute; `cc_binary` has none, and
    // Bazel rejects it as an unknown attribute rather than ignoring it.
    if target.kind == TargetKind::Library {
        render_path_list(out, "hdrs", &target.public_headers);
    }
    render_path_list(out, "includes", &target.includes);
    // Not render_path_list: a define (`FOO`, `FOO=1`) is not a path and
    // must not be run through the module-relative assertion.
    render_string_list(out, "local_defines", &target.local_defines);
    render_deps(out, &target.dependencies);
    out.push_str(PUBLIC_VISIBILITY);
    out.push_str(")\n");
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
/// Deliberately its own nested package rather than entries in the module's
/// top-level `BUILD.bazel`: validation-only targets must never appear in
/// what a user checks into their own repo. See
/// docs/architecture/build-verification.md.
pub fn render_ground_truth_build_bazel(artifacts: &[String]) -> String {
    let exports = artifacts
        .iter()
        .map(|p| format!("\"{p}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("exports_files([{exports}])\n")
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
pass_regex="$3"

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
# (tinyxml2's xmltest writes resources/out/), so stage a writable copy and
# run there. The binary is copied along with everything else, then run from
# the staged tree so its relative data paths resolve.
work_root="${TEST_TMPDIR:-$(mktemp -d)}/work"
mkdir -p "${work_root}"
cp -RL "${module_runfiles}/." "${work_root}/" 2>/dev/null || true
# working_dir is "." for the module root, or a module-relative subdir — never
# empty (an empty positional arg would be dropped when Bazel tokenizes args).
run_dir="${work_root}/${working_dir}"
mkdir -p "${run_dir}"

binary="${work_root}/${binary_name}"
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

    fn graph(version: Option<&str>) -> BuildGraph {
        BuildGraph {
            module: ModuleInfo {
                name: "hello_world".to_string(),
                version: version.map(str::to_string),
            },
            tests: vec![],
            targets: vec![Target {
                name: "hello".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/main.cpp".to_string()],
                public_headers: vec![],
                dependencies: vec![],
                includes: vec![],
                local_defines: vec![],
                artifacts: vec!["hello".to_string()],
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
                },
            ],
        };

        let rendered = render(&graph).build_bazel;
        assert!(rendered.contains("load(\"@rules_cc//cc:cc_binary.bzl\", \"cc_binary\")"));
        assert!(rendered.contains("load(\"@rules_cc//cc:cc_library.bzl\", \"cc_library\")"));
        assert!(rendered.contains(
            "cc_library(\n    name = \"greet\",\n    srcs = [\n        \"src/greet.cpp\",\n    ],\n    hdrs = [\n        \"include/greet.hpp\",\n    ],\n    includes = [\n        \"include\",\n    ],\n    visibility = [\"//visibility:public\"],\n)\n"
        ));
        assert!(rendered.contains(
            "cc_binary(\n    name = \"hello\",\n    srcs = [\n        \"src/main.cpp\",\n    ],\n    deps = [\n        \":greet\",\n    ],\n    visibility = [\"//visibility:public\"],\n)\n"
        ));
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
            targets: vec![Target {
                name: "app".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/main.cpp".to_string(), "inc/cfg.hpp".to_string()],
                public_headers: vec![],
                dependencies: vec![],
                includes: vec!["inc".to_string()],
                local_defines: vec![],
                artifacts: vec!["app".to_string()],
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
                targets: vec![Target {
                    name: "t".to_string(),
                    kind: kind.clone(),
                    sources: vec!["src/t.cpp".to_string()],
                    public_headers: vec![],
                    dependencies: vec![],
                    includes: vec!["inc".to_string()],
                    local_defines: vec![],
                    artifacts: vec![],
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
    // verbatim, NOT run through render_path_list's module-relative assert
    // (which would panic on the `=`). See Target::local_defines and
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
            }],
            tests: vec![test],
        }
    }

    #[test]
    fn renders_sh_test_and_rules_shell_dep_for_a_registered_test() {
        let graph = graph_with_test(model::Test {
            name: "xmltest".to_string(),
            target: "xmltest".to_string(),
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
        };
        mutate(&mut target);
        BuildGraph {
            module: ModuleInfo {
                name: "m".to_string(),
                version: None,
            },
            tests: vec![],
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
        let rendered =
            render_ground_truth_build_bazel(&["hello".to_string(), "libgreet.a".to_string()]);
        assert_eq!(rendered, "exports_files([\"hello\", \"libgreet.a\"])\n");
    }

    // A fixture whose targets produce no artifacts still gets the package.
    #[test]
    fn ground_truth_build_bazel_handles_no_artifacts() {
        assert_eq!(render_ground_truth_build_bazel(&[]), "exports_files([])\n");
    }

    #[test]
    fn renders_module_bazel_without_version_when_absent() {
        let rendered = render(&graph(None)).module_bazel;
        let module_block = rendered.split(")\n\n").next().unwrap();
        assert!(module_block.contains("name = \"hello_world\""));
        assert!(!module_block.contains("version ="));
        assert!(rendered.contains("bazel_dep(name = \"rules_cc\""));
        assert!(rendered.contains("bazel_dep(name = \"llvm\""));
        assert!(rendered.contains("register_toolchains(\"@llvm//toolchain:all\")"));
    }

    #[test]
    fn renders_module_bazel_with_version_when_present() {
        let rendered = render(&graph(Some("1.2.3"))).module_bazel;
        assert!(rendered.contains("version = \"1.2.3\""));
    }
}
