mod cmake_api;
mod codegen;
mod model;
mod needs_attention;

use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;

/// Convert a CMake project into a standalone Bazel module.
///
/// The output directory becomes an independent Bazel module: it gets its
/// own MODULE.bazel and BUILD.bazel, plus a copy of the project's source
/// files, so it can be built with no reference back to bazelifier's own
/// MODULE.bazel/toolchains.
#[derive(Parser)]
struct Args {
    /// Path to the CMake project (directory containing CMakeLists.txt).
    source_dir: PathBuf,

    /// Directory to configure the CMake project in (a scratch build dir,
    /// must be outside source_dir).
    #[arg(long)]
    build_dir: PathBuf,

    /// Directory to write the generated standalone Bazel module into.
    #[arg(long)]
    out_module: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let graph = cmake_api::discover(&args.source_dir, &args.build_dir)?;
    let generated = codegen::render(&graph);

    copy_referenced_sources(&args.source_dir, &args.out_module, &graph)?;
    fs::write(
        args.out_module.join("MODULE.bazel"),
        generated.module_bazel,
    )?;
    fs::write(args.out_module.join("BUILD.bazel"), generated.build_bazel)?;
    copy_ground_truth_artifacts(&args.build_dir, &args.out_module, &graph)?;
    write_needs_attention(&args.out_module, &graph)?;

    Ok(())
}

/// Writes one `needs_attention/<NNN>-<slug>.md` file per gap the
/// translator could not confidently resolve for this specific conversion
/// — distinct from bazelifier's own docs/runbooks/ interface docs, this
/// is actionable follow-up for whoever picks up the converted project.
/// See docs/architecture/runbook-interface.md.
///
/// The directory (with a glob-based `exports_files`-equivalent
/// `filegroup`) is always created, even with zero items: validation
/// tooling (see docs/architecture/build-verification.md) checks this
/// directory for any files at test-runtime to decide whether to gate on
/// triage before running the ground-truth comparison, which only works if
/// `@<module>//needs_attention:all` is always a valid, buildable label
/// regardless of whether there's anything in it.
fn write_needs_attention(out_module: &Path, graph: &model::BuildGraph) -> std::io::Result<()> {
    let dir = out_module.join("needs_attention");
    fs::create_dir_all(&dir)?;

    for (i, item) in graph.needs_attention.iter().enumerate() {
        let filename = format!("{:03}-{}.md", i + 1, needs_attention::slugify(&item.title));
        fs::write(dir.join(filename), needs_attention::render(item))?;
    }

    fs::write(
        dir.join("BUILD.bazel"),
        "filegroup(\n    name = \"all\",\n    srcs = glob(\n        [\"*.md\"],\n        allow_empty = True,\n    ),\n    visibility = [\"//visibility:public\"],\n)\n",
    )?;

    Ok(())
}

/// Copies the real cmake+ninja-built artifacts (e.g. each target's built
/// binary) into `<out_module>/ground_truth/`, alongside a small
/// `BUILD.bazel` exporting them (`exports_files`), so they're
/// referenceable (e.g. `@<module>//ground_truth:hello`) for validating
/// that the Bazel build is functionally equivalent, without exposing
/// validation-only targets in the user-facing top-level BUILD.bazel — see
/// docs/architecture/build-verification.md.
fn copy_ground_truth_artifacts(
    build_dir: &Path,
    out_module: &Path,
    graph: &model::BuildGraph,
) -> std::io::Result<()> {
    let ground_truth_dir = out_module.join("ground_truth");
    fs::create_dir_all(&ground_truth_dir)?;

    let mut artifact_paths = Vec::new();
    for target in &graph.targets {
        for artifact in &target.artifacts {
            let src = build_dir.join(artifact);
            let dst = ground_truth_dir.join(artifact);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src, &dst)?;
            artifact_paths.push(artifact.clone());
        }
    }

    let exports = artifact_paths
        .iter()
        .map(|p| format!("\"{p}\""))
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        ground_truth_dir.join("BUILD.bazel"),
        format!("exports_files([{exports}])\n"),
    )?;

    Ok(())
}

/// Copies exactly the files the converted build graph references — every
/// target's `sources` and `public_headers` — preserving their layout
/// relative to the module root.
///
/// Deliberately NOT a recursive copy of the source directory. The output is
/// a Bazel module, not a mirror of the CMake project, and a file belongs in
/// it because something in the build graph named it. Copying a directory
/// wholesale instead pulls in whatever else happens to be sitting there —
/// `.git/`, stale build outputs, editor scratch files — and, on a real
/// project, a great deal of it.
///
/// This also makes the module reproducible by construction: everything in
/// it traces to a build-graph reference, so a file that is present in the
/// source tree but not part of the build (a gitignored leftover, an
/// artifact from an earlier in-source build) cannot silently become part of
/// the deliverable. That property holds without the translator knowing
/// anything about version control — see docs/architecture/cmake-frontend.md.
///
/// Note the CMake project's own `CMakeLists.txt` is therefore not copied:
/// nothing in the generated module builds from it.
fn copy_referenced_sources(
    source_dir: &Path,
    out_dir: &Path,
    graph: &model::BuildGraph,
) -> std::io::Result<()> {
    fs::create_dir_all(out_dir)?;

    // A file can be referenced by more than one target (e.g. the same
    // source compiled into both a static and a shared library).
    let mut copied = std::collections::HashSet::new();
    for target in &graph.targets {
        for relative in target.sources.iter().chain(target.public_headers.iter()) {
            if !copied.insert(relative.as_str()) {
                continue;
            }
            let dst = out_dir.join(relative);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source_dir.join(relative), &dst)?;
        }
    }

    Ok(())
}
