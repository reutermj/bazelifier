//! Bazel BUILD file codegen. See docs/architecture/bazel-codegen.md.

use crate::model::{BuildGraph, TargetKind};

/// Renders a `BUILD.bazel` file for the given build graph. Output is meant
/// to be passed through buildifier afterward (see
/// docs/architecture/bazel-codegen.md) rather than hand-formatted here.
pub fn render(graph: &BuildGraph) -> String {
    let mut out = String::new();

    if graph
        .targets
        .iter()
        .any(|t| t.kind == TargetKind::Executable)
    {
        out.push_str("load(\"@rules_cc//cc:cc_binary.bzl\", \"cc_binary\")\n\n");
    }

    for (i, target) in graph.targets.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match target.kind {
            TargetKind::Executable => render_cc_binary(&mut out, &target.name, &target.sources),
        }
    }

    out
}

fn render_cc_binary(out: &mut String, name: &str, sources: &[String]) {
    out.push_str("cc_binary(\n");
    out.push_str(&format!("    name = \"{name}\",\n"));
    out.push_str("    srcs = [\n");
    for src in sources {
        out.push_str(&format!("        \"{src}\",\n"));
    }
    out.push_str("    ],\n");
    out.push_str(")\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Target;

    #[test]
    fn renders_single_cc_binary() {
        let graph = BuildGraph {
            targets: vec![Target {
                name: "hello".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/main.cpp".to_string()],
            }],
        };

        let rendered = render(&graph);
        assert_eq!(
            rendered,
            "load(\"@rules_cc//cc:cc_binary.bzl\", \"cc_binary\")\n\n\
             cc_binary(\n    name = \"hello\",\n    srcs = [\n        \"src/main.cpp\",\n    ],\n)\n"
        );
    }
}
