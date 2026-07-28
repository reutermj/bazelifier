mod cmake_api;
mod codegen;
mod model;
mod needs_attention;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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

    /// Root of the source deliverable being converted — the tarball,
    /// checkout, or directory the project ships as its sources.
    ///
    /// The generated module may grow to cover anything the build
    /// references inside this directory, so a project that compiles a
    /// sibling directory's sources converts cleanly when the root is set
    /// wide enough to contain both. Anything referenced from outside it
    /// cannot be reproduced from what the project ships and is escalated
    /// instead of quietly packaged.
    ///
    /// Deliberately explicit rather than inferred: inference here fails
    /// silently, and in the direction of packaging too much. Defaults to
    /// `source_dir`, i.e. the project converts on its own.
    #[arg(long)]
    deliverable_root: Option<PathBuf>,
}

fn main() -> ExitCode {
    if let Err(error) = run() {
        eprintln!("{}", report(error.as_ref()));
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Formats a fatal error for the terminal, via `Display`.
///
/// Returning `Result<_, Box<dyn Error>>` from `main` looks like it does
/// this already, but Rust's termination path formats the error with
/// `Debug`. Every `Display` impl in the crate was therefore dead code, and
/// the messages written to be read by a human — `cmake_api::Error`'s, which
/// passes CMake's own multi-line stderr straight through — arrived as a
/// single escaped blob wrapped in struct syntax. That is the output a user
/// gets for the most common failure there is, a CMake project that doesn't
/// configure.
fn report(error: &dyn std::error::Error) -> String {
    format!("error: {error}")
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let deliverable_root = args.deliverable_root.as_ref().unwrap_or(&args.source_dir);
    let discovery = cmake_api::discover(&args.source_dir, &args.build_dir, deliverable_root)?;
    let graph = &discovery.graph;
    let generated = codegen::render(graph);

    copy_referenced_sources(&discovery.module_root, &args.out_module, graph)?;
    fs::write(args.out_module.join("MODULE.bazel"), generated.module_bazel)?;
    fs::write(args.out_module.join("BUILD.bazel"), generated.build_bazel)?;
    copy_ground_truth_artifacts(&args.build_dir, &args.out_module, graph)?;
    write_needs_attention(&args.out_module, &discovery.needs_attention)?;

    Ok(())
}

/// Writes one `needs_attention/<NNN>-<slug>.md` file per gap the
/// translator could not confidently resolve for this specific conversion:
/// actionable follow-up for whoever picks up the converted project. See
/// docs/architecture/needs-attention-interface.md.
///
/// The directory and its `BUILD.bazel` are always written, even with zero
/// items — see `codegen::render_needs_attention_build_bazel` for why the
/// empty case has to remain a valid Bazel package.
fn write_needs_attention(
    out_module: &Path,
    items: &[needs_attention::NeedsAttention],
) -> std::io::Result<()> {
    let dir = out_module.join("needs_attention");
    fs::create_dir_all(&dir)?;

    for (i, item) in items.iter().enumerate() {
        let filename = format!("{:03}-{}.md", i + 1, needs_attention::slugify(&item.title));
        fs::write(dir.join(filename), needs_attention::render(item))?;
    }

    fs::write(
        dir.join("BUILD.bazel"),
        codegen::render_needs_attention_build_bazel(),
    )?;

    Ok(())
}

/// Copies the real cmake+ninja-built artifacts (e.g. each target's built
/// binary) into `<out_module>/ground_truth/`, alongside a small
/// `BUILD.bazel` exporting them (`codegen::render_ground_truth_build_bazel`)
/// so they're referenceable (e.g. `@<module>//ground_truth:hello`) when
/// validating that the Bazel build is functionally equivalent — see
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
            copy_into(&build_dir.join(artifact), &ground_truth_dir.join(artifact))?;
            artifact_paths.push(artifact.clone());
        }
    }

    fs::write(
        ground_truth_dir.join("BUILD.bazel"),
        codegen::render_ground_truth_build_bazel(&artifact_paths),
    )?;

    Ok(())
}

/// Copies exactly the files the converted build graph references — every
/// target's `sources` and `public_headers` — from the module root, keeping
/// their layout relative to it.
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
    module_root: &Path,
    out_dir: &Path,
    graph: &model::BuildGraph,
) -> std::io::Result<()> {
    fs::create_dir_all(out_dir)?;

    // A file can be referenced by more than one target (e.g. the same
    // source compiled into both a static and a shared library).
    let mut copied = HashSet::new();
    for target in &graph.targets {
        for relative in target.sources.iter().chain(target.public_headers.iter()) {
            if copied.insert(relative.as_str()) {
                copy_into(&module_root.join(relative), &out_dir.join(relative))?;
            }
        }
    }

    Ok(())
}

/// Copies `src` to `dst`, creating `dst`'s parent directories. Every file
/// the translator places in the output tree keeps its layout relative to
/// some root, so the destination's directories generally don't exist yet.
fn copy_into(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The regression this guards: `{error:?}` here instead of `{error}`
    // silently reverts every Display impl in the crate to dead code, and
    // nothing else in the suite would notice.
    #[test]
    fn report_formats_with_display_not_debug() {
        let error = cmake_api::Error::CmakeConfigureFailed {
            stderr: "CMake Error at CMakeLists.txt:3 (add_executable):\n  Cannot find source \
                     file:\n\n    does_not_exist.cpp\n"
                .to_string(),
        };

        let reported = report(&error);

        assert!(reported.contains("cmake configure failed:\n"));
        assert!(reported.contains("Cannot find source file"));
        // Debug would name the variant and escape the newlines into one line.
        assert!(
            !reported.contains("CmakeConfigureFailed"),
            "Debug formatting leaked into the message:\n{reported}"
        );
        assert!(
            !reported.contains("\\n"),
            "newlines were escaped rather than printed:\n{reported}"
        );
    }

    #[test]
    fn report_covers_every_error_variant_readably() {
        let variants: Vec<Box<dyn std::error::Error>> = vec![
            Box::new(cmake_api::Error::NoProject),
            Box::new(cmake_api::Error::CmakeBuildFailed {
                stderr: "ninja: build stopped\n".to_string(),
            }),
            Box::new(cmake_api::Error::SourceDirOutsideDeliverableRoot {
                source_dir: "/a/proj".to_string(),
                deliverable_root: "/b".to_string(),
            }),
        ];

        for error in &variants {
            let reported = report(error.as_ref());
            assert!(reported.starts_with("error: "), "{reported}");
            assert!(
                !reported.contains('{'),
                "looks like Debug struct syntax, not a sentence:\n{reported}"
            );
        }
    }
}
