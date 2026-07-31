mod cmake_api;
mod ctest;
mod codegen;
mod model;
mod needs_attention;
mod paths;

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
/// `Debug`, which makes every `Display` impl in the crate dead code and
/// turns `cmake_api::Error`'s human-readable text — CMake's own multi-line
/// stderr, passed straight through — into one escaped blob in struct
/// syntax. That is the output a user gets for the most common failure
/// there is, a CMake project that doesn't configure.
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
    copy_test_runtime_data(&discovery.module_root, &args.out_module, graph)?;
    fs::write(args.out_module.join("MODULE.bazel"), generated.module_bazel)?;
    fs::write(args.out_module.join("BUILD.bazel"), generated.build_bazel)?;
    // The wrapper the generated sh_tests run — only when there are tests.
    if !graph.tests.is_empty() {
        let script_path = args.out_module.join("run_cmake_test.sh");
        fs::write(&script_path, codegen::render_run_cmake_test_sh())?;
        make_executable(&script_path)?;
    }
    copy_ground_truth_artifacts(&args.build_dir, &args.out_module, graph)?;
    write_needs_attention(&args.out_module, &discovery.needs_attention)?;

    Ok(())
}

/// Marks a file executable (0o755). The generated test wrapper is an
/// `sh_test` `srcs` entry, which Bazel expects to be runnable.
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)
}

/// Writes one `needs_attention/<NNN>-<slug>.md` file per gap the
/// translator could not confidently resolve for this specific conversion:
/// actionable follow-up for whoever picks up the converted project. See
/// docs/architecture/needs-attention-interface.md.
///
/// The directory, `BUILD.bazel`, and `MANIFEST` are always written, even
/// with zero items. `BUILD.bazel` keeps the empty case a valid Bazel
/// package (see `codegen::render_needs_attention_build_bazel`). `MANIFEST`
/// exists for a different reason: an empty `*.md` glob is legal Starlark
/// but can vanish entirely from a consuming test's runfiles rather than
/// leaving an empty directory behind, so "wiring is broken" and "zero
/// items" would otherwise look identical to anything gating on directory
/// presence. `MANIFEST` is never itself the product of a glob, so its
/// presence is a reliable signal that this directory really is the one the
/// translator wrote, regardless of how many `.md` files it holds.
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

    fs::write(dir.join("MANIFEST"), format!("{}\n", items.len()))?;

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
    let mut shared_lib_names = Vec::new();
    // (target name, artifact path) per executable, so the ground_truth
    // BUILD.bazel can expose each under its TARGET name even when CMake built
    // it under a subdirectory (apps/json_parse). See bzl-fxa.13.
    let mut executables = Vec::new();
    for target in &graph.targets {
        if target.kind == model::TargetKind::Executable
            && let Some(artifact) = target.artifacts.first()
        {
            executables.push((target.name.clone(), artifact.clone()));
        }
        for artifact in &target.artifacts {
            copy_into(&build_dir.join(artifact), &ground_truth_dir.join(artifact))?;
            artifact_paths.push(artifact.clone());

            // A shared library the build produced (libfoo.so): a dynamically
            // linked ground-truth binary loads it by its SONAME at run time,
            // and the absolute RUNPATH CMake baked in points at this build dir,
            // which is gone by then. So stage the whole versioned symlink chain
            // (libfoo.so -> libfoo.so.5 -> libfoo.so.5.2.0) at the artifact's
            // own path under ground_truth/ and let the comparison test search
            // for it; the binary then finds whichever name it needs. That path
            // is NOT necessarily the binary's own directory — a subdirectory
            // binary sits below the libs — which is why the search walks up to
            // the ground_truth/ root (bzl-0x7). A static lib (.a) is linked
            // into the binary, so nothing to stage. See bzl-fxa.11 and
            // docs/architecture/build-verification.md.
            if is_shared_library(artifact) {
                for name in stage_shared_library_chain(build_dir, artifact, &ground_truth_dir)? {
                    if !artifact_paths.contains(&name) {
                        artifact_paths.push(name.clone());
                    }
                    if !shared_lib_names.contains(&name) {
                        shared_lib_names.push(name);
                    }
                }
            }
        }
    }

    fs::write(
        ground_truth_dir.join("BUILD.bazel"),
        codegen::render_ground_truth_build_bazel(&artifact_paths, &executables, &shared_lib_names),
    )?;

    Ok(())
}

/// Whether a build artifact is a shared library — i.e. something a dynamically
/// linked binary loads at run time, as opposed to a static archive (`.a`) that
/// is linked in. Matches both the plain `libfoo.so` and a versioned
/// `libfoo.so.5` / `libfoo.so.5.2.0`. (macOS `.dylib` / Windows `.dll` would
/// go here too when those platforms are supported; only ELF `.so` is exercised
/// today.)
fn is_shared_library(artifact: &str) -> bool {
    let name = Path::new(artifact)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    name.ends_with(".so") || name.contains(".so.")
}

/// Stages every name in a shared library's versioned symlink chain into
/// `ground_truth/`, copying the real file's bytes under each name, and returns
/// those names. The build produces `libfoo.so -> libfoo.so.5 ->
/// libfoo.so.5.2.0`; a binary's `DT_NEEDED` is the SONAME (`libfoo.so.5`), not
/// the `artifact` path the File API reports (`libfoo.so`), so copying only the
/// artifact would stage the wrong name. Copying the real bytes under every name
/// in the chain means whichever one the loader asks for is present — without
/// this translator having to read the binary's dynamic section.
///
/// The names are discovered by realpath, not by chasing links: every entry in
/// the build dir that resolves to the same real file as `artifact` is part of
/// the chain. Symlinks are deliberately flattened to real files because the
/// ground_truth tree ships through a Bazel tree artifact and a tarball, where a
/// dangling intra-build symlink would not survive.
fn stage_shared_library_chain(
    build_dir: &Path,
    artifact: &str,
    ground_truth_dir: &Path,
) -> std::io::Result<Vec<String>> {
    let artifact_path = build_dir.join(artifact);
    let real = fs::canonicalize(&artifact_path)?;
    let artifact_dir = artifact_path.parent().unwrap_or(build_dir);
    let subdir = Path::new(artifact).parent();

    let mut names = Vec::new();
    for entry in fs::read_dir(artifact_dir)? {
        let entry = entry?;
        let path = entry.path();
        // Same real file (the chain), following symlinks. canonicalize on a
        // non-symlink returns itself, so this also matches the real file.
        if fs::canonicalize(&path).ok().as_deref() != Some(real.as_path()) {
            continue;
        }
        let file_name = entry.file_name();
        // Preserve the artifact's own subdirectory (rare, but the File API can
        // report an artifact under a subdir) so staged names line up with it.
        let rel = match subdir {
            Some(dir) if !dir.as_os_str().is_empty() => dir.join(&file_name),
            _ => PathBuf::from(&file_name),
        };
        copy_into(&path, &ground_truth_dir.join(&rel))?;
        names.push(rel.to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

/// Copies exactly the files the converted build graph references — every
/// target's `sources` and `public_headers` — from the module root, keeping
/// their layout relative to it.
///
/// Deliberately NOT a recursive copy of the source directory: the output is
/// a Bazel module, not a mirror of the CMake project, and a file belongs in
/// it because something in the build graph named it. That keeps `.git/`,
/// stale build outputs and editor scratch files out, and makes the module
/// reproducible by construction without the translator knowing anything
/// about version control. The project's own `CMakeLists.txt` is therefore
/// not copied either — nothing in the generated module builds from it.
///
/// Full rationale: docs/architecture/cmake-frontend.md's "only referenced
/// files enter the module".
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

    // configure_file templates are referenced too — the config_header rule
    // expands them (their generated outputs are NOT copied; they're produced
    // at the consumer's build). The template is a source-tree file relative to
    // the module root, like a source.
    for config_header in &graph.config_headers {
        if copied.insert(config_header.template.as_str()) {
            copy_into(
                &module_root.join(&config_header.template),
                &out_dir.join(&config_header.template),
            )?;
        }
    }

    Ok(())
}

/// Copies the runtime data a CTest test reads/writes under its working
/// directory into the module, so the generated `sh_test` can stage and run
/// it (tinyxml2's xmltest reads `resources/*.xml` and writes `resources/out/`).
///
/// Unlike `copy_referenced_sources`, the translator cannot know *which*
/// files under the working directory a test touches — that would need to
/// run it. So this copies the working directory's subtree wholesale, minus
/// files that are definitely not runtime data: the build's own metadata
/// (CMakeLists.txt/Makefile/meson) which the module deliberately omits, and
/// the Bazel files a checkout may carry (`BUILD.bazel`/`MODULE.bazel`/
/// `REPO.bazel`), which would collide with the ones the translator emits.
/// This is the tinyxml2-shaped scope (bzl-c54.8); a precise
/// referenced-data model is future work — a working directory equal to the
/// module root copies the whole (clean) source tree, which over-includes
/// docs and the like but is correct.
fn copy_test_runtime_data(
    module_root: &Path,
    out_dir: &Path,
    graph: &model::BuildGraph,
) -> std::io::Result<()> {
    let mut copied_roots = HashSet::new();
    for test in &graph.tests {
        let work_rel = Path::new(&test.working_directory);
        if !copied_roots.insert(test.working_directory.clone()) {
            continue;
        }
        copy_runtime_tree(&module_root.join(work_rel), &out_dir.join(work_rel))?;
    }
    Ok(())
}

/// Names that must never be copied as runtime data: build metadata the
/// module omits, Bazel files that would collide with generated output, and
/// VCS bookkeeping that is never a test's runtime input. Matched on the
/// file/dir name at any depth.
const NON_RUNTIME_NAMES: &[&str] = &[
    "CMakeLists.txt",
    "Makefile",
    "meson.build",
    "meson_options.txt",
    "BUILD.bazel",
    "MODULE.bazel",
    "REPO.bazel",
    ".git",
    ".github",
    ".gitignore",
];

/// Recursively copies `src` into `dst`, skipping `NON_RUNTIME_NAMES`.
fn copy_runtime_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if NON_RUNTIME_NAMES
            .iter()
            .any(|skip| name.to_str() == Some(skip))
        {
            continue;
        }
        let child_src = entry.path();
        let child_dst = dst.join(&name);
        if child_src.is_dir() {
            copy_runtime_tree(&child_src, &child_dst)?;
        } else {
            copy_into(&child_src, &child_dst)?;
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

    #[test]
    fn is_shared_library_matches_so_and_versioned_so_only() {
        // What decides staging (bzl-fxa.11): a shared library is loaded at run
        // time, a static archive is linked in. Both the plain and the versioned
        // SONAME forms must count — the binary's DT_NEEDED is the versioned one.
        assert!(is_shared_library("libgreet.so"));
        assert!(is_shared_library("libgreet.so.5"));
        assert!(is_shared_library("libgreet.so.5.2.0"));
        assert!(is_shared_library("sub/dir/libjson-c.so.5"));
        // Not shared: a static archive, an executable, or a name that merely
        // contains "so" without the extension.
        assert!(!is_shared_library("libgreet.a"));
        assert!(!is_shared_library("app"));
        assert!(!is_shared_library("libsonic")); // no ".so"
        assert!(!is_shared_library("json_parse"));
    }
}
