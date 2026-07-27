//! Bazel codegen: renders a standalone Bazel module (its own `MODULE.bazel`
//! and `BUILD.bazel`) for a CMake project's `BuildGraph`. See
//! docs/architecture/bazel-codegen.md — the whole point is that this output
//! must build on its own, with no reference back to bazelifier's own
//! MODULE.bazel/toolchains.

use crate::model::{BuildGraph, TargetKind};

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
    let mut out = String::new();

    out.push_str("module(\n");
    out.push_str(&format!("    name = \"{}\",\n", graph.module.name));
    if let Some(version) = &graph.module.version {
        out.push_str(&format!("    version = \"{version}\",\n"));
    }
    out.push_str(")\n\n");

    out.push_str(&format!(
        "bazel_dep(name = \"rules_cc\", version = \"{RULES_CC_VERSION}\")\n"
    ));
    out.push_str(&format!(
        "bazel_dep(name = \"llvm\", version = \"{LLVM_VERSION}\")\n\n"
    ));
    out.push_str("register_toolchains(\"@llvm//toolchain:all\")\n");

    out
}

fn render_build_bazel(graph: &BuildGraph) -> String {
    let mut out = String::new();

    let has_binary = graph
        .targets
        .iter()
        .any(|t| t.kind == TargetKind::Executable);
    let has_library = graph.targets.iter().any(|t| t.kind == TargetKind::Library);

    if has_binary {
        out.push_str("load(\"@rules_cc//cc:cc_binary.bzl\", \"cc_binary\")\n");
    }
    if has_library {
        out.push_str("load(\"@rules_cc//cc:cc_library.bzl\", \"cc_library\")\n");
    }
    if has_binary || has_library {
        out.push('\n');
    }

    for (i, target) in graph.targets.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match target.kind {
            TargetKind::Executable => render_cc_binary(
                &mut out,
                &target.name,
                &target.sources,
                &target.includes,
                &target.dependencies,
            ),
            TargetKind::Library => render_cc_library(
                &mut out,
                &target.name,
                &target.sources,
                &target.public_headers,
                &target.dependencies,
                &target.includes,
            ),
        }
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

fn render_deps(out: &mut String, deps: &[String]) {
    if deps.is_empty() {
        return;
    }
    out.push_str("    deps = [\n");
    for dep in deps {
        out.push_str(&format!("        \":{dep}\",\n"));
    }
    out.push_str("    ],\n");
}

// Public by default: a converted module is meant to be depended on, both
// by bazelifier's own validation tooling and, as more projects get
// converted, by other converted modules (matching how CMake targets are
// typically visible project-wide unless the project opts into something
// narrower — CMake has no per-target visibility concept of its own to
// translate).
const PUBLIC_VISIBILITY: &str = "    visibility = [\"//visibility:public\"],\n";

// `includes` matters just as much for an executable as for a library: an
// `add_executable` target with its own `target_include_directories()` needs
// the `-I` path to compile at all. It isn't only relevant to targets that
// get depended on — Bazel's transitivity means a *consumer* inherits a
// dependency's `includes`, but nothing supplies a target's own.
fn render_cc_binary(
    out: &mut String,
    name: &str,
    sources: &[String],
    includes: &[String],
    deps: &[String],
) {
    out.push_str("cc_binary(\n");
    out.push_str(&format!("    name = \"{name}\",\n"));
    render_string_list(out, "srcs", sources);
    render_string_list(out, "includes", includes);
    render_deps(out, deps);
    out.push_str(PUBLIC_VISIBILITY);
    out.push_str(")\n");
}

fn render_cc_library(
    out: &mut String,
    name: &str,
    sources: &[String],
    public_headers: &[String],
    deps: &[String],
    includes: &[String],
) {
    out.push_str("cc_library(\n");
    out.push_str(&format!("    name = \"{name}\",\n"));
    render_string_list(out, "srcs", sources);
    render_string_list(out, "hdrs", public_headers);
    render_string_list(out, "includes", includes);
    render_deps(out, deps);
    out.push_str(PUBLIC_VISIBILITY);
    out.push_str(")\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ModuleInfo, Target};

    fn graph(version: Option<&str>) -> BuildGraph {
        BuildGraph::new(
            ModuleInfo {
                name: "hello_world".to_string(),
                version: version.map(str::to_string),
            },
            vec![Target {
                name: "hello".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/main.cpp".to_string()],
                public_headers: vec![],
                dependencies: vec![],
                includes: vec![],
                artifacts: vec!["hello".to_string()],
            }],
        )
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
        let graph = BuildGraph::new(
            ModuleInfo {
                name: "lib_example".to_string(),
                version: None,
            },
            vec![
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
        );

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
        let graph = BuildGraph::new(
            ModuleInfo {
                name: "binary_private_include".to_string(),
                version: None,
            },
            vec![Target {
                name: "app".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/main.cpp".to_string(), "inc/cfg.hpp".to_string()],
                public_headers: vec![],
                dependencies: vec![],
                includes: vec!["inc".to_string()],
                artifacts: vec!["app".to_string()],
            }],
        );

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
            let graph = BuildGraph::new(
                ModuleInfo {
                    name: "m".to_string(),
                    version: None,
                },
                vec![Target {
                    name: "t".to_string(),
                    kind: kind.clone(),
                    sources: vec!["src/t.cpp".to_string()],
                    public_headers: vec![],
                    dependencies: vec![],
                    includes: vec!["inc".to_string()],
                    artifacts: vec![],
                }],
            );

            let rendered = render(&graph).build_bazel;
            assert!(
                rendered.contains("    includes = [\n        \"inc\",\n    ],\n"),
                "{kind:?} dropped its own include dirs:\n{rendered}"
            );
        }
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
