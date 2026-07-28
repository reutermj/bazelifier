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

pub struct GeneratedModule {
    pub module_bazel: String,
    pub build_bazel: String,
}

/// Renders a standalone `MODULE.bazel` + `BUILD.bazel` pair for the given
/// build graph.
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

    format!(
        "module(\n    name = \"{name}\",\n{version})\n\n\
         bazel_dep(name = \"rules_cc\", version = \"{RULES_CC_VERSION}\")\n\
         bazel_dep(name = \"llvm\", version = \"{LLVM_VERSION}\")\n\n\
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
    if !rules.is_empty() {
        out.push('\n');
    }

    for (i, target) in graph.targets.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        render_cc_rule(&mut out, target);
    }

    out
}

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
/// degrading. It is deliberately a real `assert!` and not a
/// `debug_assert!`: the failure it prevents is silently non-portable
/// output, and Bazel does not catch that for string attributes like
/// `includes` (only for label attributes). The cost is a couple of string
/// comparisons per emitted path.
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
/// `includes` matters just as much for an executable as for a library: an
/// `add_executable` target with its own `target_include_directories()`
/// needs the `-I` path to compile at all. It isn't only relevant to
/// targets that get depended on — Bazel's transitivity means a *consumer*
/// inherits a dependency's `includes`, but nothing supplies a target's own.
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
pub fn render_needs_attention_build_bazel() -> String {
    "filegroup(\n    name = \"all\",\n    srcs = glob(\n        [\"*.md\"],\n        \
     allow_empty = True,\n    ),\n    visibility = [\"//visibility:public\"],\n)\n"
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
            targets: vec![Target {
                name: "hello".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/main.cpp".to_string()],
                public_headers: vec![],
                dependencies: vec![],
                includes: vec![],
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
            targets: vec![
                Target {
                    name: "greet".to_string(),
                    kind: TargetKind::Library,
                    sources: vec!["src/greet.cpp".to_string()],
                    public_headers: vec!["include/greet.hpp".to_string()],
                    dependencies: vec![],
                    includes: vec!["include".to_string()],
                    artifacts: vec!["libgreet.a".to_string()],
                },
                Target {
                    name: "hello".to_string(),
                    kind: TargetKind::Executable,
                    sources: vec!["src/main.cpp".to_string()],
                    public_headers: vec![],
                    dependencies: vec!["greet".to_string()],
                    includes: vec![],
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
            targets: vec![Target {
                name: "app".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/main.cpp".to_string(), "inc/cfg.hpp".to_string()],
                public_headers: vec![],
                dependencies: vec![],
                includes: vec!["inc".to_string()],
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
                targets: vec![Target {
                    name: "t".to_string(),
                    kind: kind.clone(),
                    sources: vec!["src/t.cpp".to_string()],
                    public_headers: vec![],
                    dependencies: vec![],
                    includes: vec!["inc".to_string()],
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

    fn graph_with(kind: TargetKind, mutate: impl FnOnce(&mut Target)) -> BuildGraph {
        let mut target = Target {
            name: "t".to_string(),
            kind,
            sources: vec!["src/t.cpp".to_string()],
            public_headers: vec![],
            dependencies: vec![],
            includes: vec![],
            artifacts: vec![],
        };
        mutate(&mut target);
        BuildGraph {
            module: ModuleInfo {
                name: "m".to_string(),
                version: None,
            },
            targets: vec![target],
        }
    }

    // One case per path-valued attribute. `srcs`/`hdrs` are Bazel *label*
    // attributes, so an absolute path there is at least an analysis error
    // downstream; `includes` is a plain string list that Bazel accepts
    // silently (verified against Bazel 9.2.0), which is why codegen has to
    // be the thing that refuses it.
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
            targets: vec![
                Target {
                    name: "lib".to_string(),
                    kind: TargetKind::Library,
                    sources: vec!["src/lib.cpp".to_string()],
                    public_headers: vec!["include/lib.hpp".to_string()],
                    dependencies: vec![],
                    includes: vec!["include".to_string()],
                    artifacts: vec!["liblib.a".to_string()],
                },
                Target {
                    name: "app".to_string(),
                    kind: TargetKind::Executable,
                    sources: vec!["src/main.cpp".to_string()],
                    public_headers: vec![],
                    dependencies: vec!["lib".to_string()],
                    includes: vec!["inc".to_string()],
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
