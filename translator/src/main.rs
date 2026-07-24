mod cmake_api;
mod codegen;
mod model;

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

    copy_sources(&args.source_dir, &args.out_module)?;
    fs::write(
        args.out_module.join("MODULE.bazel"),
        generated.module_bazel,
    )?;
    fs::write(args.out_module.join("BUILD.bazel"), generated.build_bazel)?;
    copy_ground_truth_artifacts(&args.build_dir, &args.out_module, &graph)?;

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

/// Copies the CMake project's source files into the output module
/// directory, so the generated BUILD.bazel has something to build against.
fn copy_sources(source_dir: &Path, out_dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(out_dir)?;
    copy_dir_recursive(source_dir, out_dir)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();

        let dst_path = dst.join(entry.file_name());
        if path.is_dir() {
            fs::create_dir_all(&dst_path)?;
            copy_dir_recursive(&path, &dst_path)?;
        } else {
            fs::copy(&path, &dst_path)?;
        }
    }
    Ok(())
}
