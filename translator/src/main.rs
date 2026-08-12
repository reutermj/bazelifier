mod autotools;
mod cmake_api;
mod codegen;
mod config_header;
mod configure_file;
mod ctest;
mod error;
mod headers;
mod libtool;
mod model;
mod needs_attention;
mod ninja_deps;
mod paths;
mod project_notes;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

/// Which build system a project uses, and so which frontend reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Frontend {
    Cmake,
    Autotools,
}

/// Detects a project's build system from what it ships.
///
/// Detection rather than a required flag because the answer is unambiguous in
/// practice — a project ships `CMakeLists.txt` or `configure.ac`, and one file
/// decides it — and requiring the flag would push that decision onto every
/// caller including every fixture's BUILD.bazel. `--frontend` overrides it for
/// the case detection cannot settle: a project shipping BOTH, where which one
/// to convert is a real choice rather than something to infer.
///
/// Deliberately does NOT fall back to a default. A directory with neither
/// marker is not a project this tool can convert, and guessing CMake would
/// produce a confusing failure deep in the frontend rather than here.
fn detect_frontend(source_dir: &Path) -> Option<Frontend> {
    let cmake = source_dir.join("CMakeLists.txt").is_file();
    // `configure.ac` is the modern name and `configure.in` the historical one;
    // a shipped tarball may have neither, having been bootstrapped already, so
    // `configure` counts too.
    let autotools = ["configure.ac", "configure.in", "configure"]
        .iter()
        .any(|f| source_dir.join(f).is_file());

    match (cmake, autotools) {
        (true, _) => Some(Frontend::Cmake),
        (false, true) => Some(Frontend::Autotools),
        (false, false) => None,
    }
}

/// Convert a CMake or Autotools project into a standalone Bazel module.
///
/// The output directory becomes an independent Bazel module: it gets its
/// own MODULE.bazel and BUILD.bazel, plus a copy of the project's source
/// files, so it can be built with no reference back to bazelifier's own
/// MODULE.bazel/toolchains.
#[derive(Parser)]
struct Args {
    /// Path to the project (the directory containing CMakeLists.txt or
    /// configure.ac).
    source_dir: PathBuf,

    /// Which build system to read the project with. Detected from the
    /// project's own files when omitted; pass this only when a project ships
    /// more than one and the choice is genuinely yours.
    #[arg(long, value_enum)]
    frontend: Option<Frontend>,

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
/// turns `error::Error`'s human-readable text — CMake's own multi-line
/// stderr, passed straight through — into one escaped blob in struct
/// syntax. That is the output a user gets for the most common failure
/// there is, a CMake project that doesn't configure.
fn report(error: &dyn std::error::Error) -> String {
    format!("error: {error}")
}

/// The frontend to convert with: an explicit `--frontend` if given, else
/// whatever the project's own files say.
///
/// Its own function so the PRECEDENCE is testable. The override is not a
/// convenience — xz ships both `CMakeLists.txt` and `configure.ac`, detection
/// picks CMake, and converting xz as CMake fails deep inside that frontend on
/// a File API reply that was never written.
fn choose_frontend(explicit: Option<Frontend>, source_dir: &Path) -> Option<Frontend> {
    explicit.or_else(|| detect_frontend(source_dir))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let deliverable_root = args.deliverable_root.as_ref().unwrap_or(&args.source_dir);
    let frontend = match choose_frontend(args.frontend, &args.source_dir) {
        Some(frontend) => frontend,
        None => {
            return Err(Box::new(error::Error::NoFrontendDetected {
                source_dir: args.source_dir.display().to_string(),
            }));
        }
    };
    let discovery = match frontend {
        Frontend::Cmake => {
            cmake_api::discover(&args.source_dir, &args.build_dir, deliverable_root)?
        }
        Frontend::Autotools => {
            autotools::discover(&args.source_dir, &args.build_dir, deliverable_root)?
        }
    };
    let graph = &discovery.graph;
    let generated = codegen::render(graph);

    copy_referenced_sources(&discovery.module_root, &args.out_module, graph)?;
    copy_test_runtime_data(&discovery.module_root, &args.out_module, graph)?;
    fs::write(args.out_module.join("MODULE.bazel"), generated.module_bazel)?;
    fs::write(args.out_module.join("BUILD.bazel"), generated.build_bazel)?;
    // The wrapper the generated sh_tests run — only when there are tests.
    if !graph.tests.is_empty() {
        let script_path = args.out_module.join("run_registered_test.sh");
        fs::write(&script_path, codegen::render_run_registered_test_sh())?;
        make_executable(&script_path)?;
    }
    copy_ground_truth_artifacts(&args.build_dir, &args.out_module, graph)?;
    write_needs_attention(&args.out_module, &discovery.needs_attention)?;
    write_project_notes(&args.out_module, &graph.module.name)?;
    write_targets_manifest(&args.out_module, graph)?;
    write_conversion_summary(&args.out_module, graph, &discovery.needs_attention)?;

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

/// Writes `CONVERSION.json`: what this conversion produced and what it could
/// not, in one machine-readable record.
///
/// For the pipeline sweep (bzl-ccv), which needs to compare a project against
/// its previous run and answer "did this change make some other project
/// escalate more". Reading that off the generated `BUILD.bazel` means
/// regexing Starlark, and reading it off the markdown means parsing prose.
///
/// Deliberately does NOT supersede `TARGETS` or `needs_attention/MANIFEST`,
/// which look like partial versions of this and are not. Both have consumers
/// with semantics this file must not quietly take over: `TARGETS` is the
/// contract `validation_workspace.bzl` reads to generate comparison tests,
/// and `MANIFEST`'s presence is how `compare_runtime_output.sh` tells "zero
/// escalations" from "the runfiles path does not resolve" — a distinction a
/// JSON file with a count in it cannot make.
///
/// Escalations are keyed by `kind`, never by title or filename: see
/// docs/architecture/needs-attention-interface.md on why those two are not
/// stable keys.
fn write_conversion_summary(
    out_module: &Path,
    graph: &model::BuildGraph,
    needs_attention: &[needs_attention::NeedsAttention],
) -> std::io::Result<()> {
    let mut escalations: Vec<serde_json::Value> = needs_attention
        .iter()
        .map(|item| {
            serde_json::json!({
                "kind": item.kind,
                "subject": item.subject,
                // What the item SAYS, not just that it exists — see
                // `needs_attention::digest`. Without this the sweep cannot
                // tell 70 escalated macros from 77.
                "digest": needs_attention::digest(item),
            })
        })
        .collect();
    // Sorted so two runs of the same input produce the same bytes; the sweep
    // diffs these, and an ordering difference would read as a change.
    escalations.sort_by_key(|e| {
        (
            e["kind"].as_str().unwrap_or("").to_string(),
            e["subject"].as_str().unwrap_or("").to_string(),
        )
    });

    let mut by_kind: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for item in needs_attention {
        *by_kind.entry(item.kind).or_default() += 1;
    }

    let mut targets: Vec<serde_json::Value> = graph
        .targets
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "kind": match t.kind {
                    model::TargetKind::Executable => "executable",
                    model::TargetKind::Library if t.is_shared => "shared_library",
                    model::TargetKind::Library => "static_library",
                },
                "sources": t.sources.len(),
            })
        })
        .collect();
    targets.sort_by_key(|t| t["name"].as_str().unwrap_or("").to_string());

    let summary = serde_json::json!({
        "module": graph.module.name,
        "version": graph.module.version,
        "targets": targets,
        "tests": graph.tests.len(),
        "config_headers": graph.config_headers.len(),
        "escalations": escalations,
        "escalations_by_kind": by_kind,
    });

    fs::write(
        out_module.join("CONVERSION.json"),
        format!("{}\n", serde_json::to_string_pretty(&summary)?),
    )
}

/// Writes `project_notes/`, the notes for THIS project, when there are any.
///
/// No directory at all when the project has none, rather than an empty one:
/// an empty directory reads as "someone looked and found nothing", which is
/// a claim nobody has made.
fn write_project_notes(out_module: &Path, module_name: &str) -> std::io::Result<()> {
    let notes = project_notes::for_project(module_name);
    if notes.is_empty() {
        return Ok(());
    }
    let dir = out_module.join("project_notes");
    fs::create_dir_all(&dir)?;
    for note in notes {
        fs::write(dir.join(note.filename), note.body)?;
    }
    Ok(())
}

/// Writes `TARGETS`, a machine-readable list of what this module emitted, for
/// the validation harness to read instead of regexing `BUILD.bazel`.
///
/// The harness needs three facts per module — which binaries exist, which
/// tests wrap them, and which config-header assertions to run — and used to
/// recover all three with `sed` over the generated Starlark. That coupled it
/// to codegen's exact whitespace and line breaking: a formatting change (the
/// intended `buildifier`-on-output pass is the obvious one) would silently
/// match nothing, and a `sed` that matches nothing yields no tests rather
/// than an error. See bzl-dmf.
///
/// One `<kind> <name>` per line, sorted within each kind so the file does not
/// churn on target order. Deliberately not JSON: the reader is a shell script
/// in a genrule, and `while read kind name` needs no parser.
fn write_targets_manifest(out_module: &Path, graph: &model::BuildGraph) -> std::io::Result<()> {
    let mut lines = Vec::new();

    let mut binaries: Vec<&str> = graph
        .targets
        .iter()
        .filter(|t| t.kind == model::TargetKind::Executable)
        .map(|t| t.name.as_str())
        .collect();
    binaries.sort_unstable();
    lines.extend(binaries.iter().map(|n| format!("binary {n}")));

    // The binary each generated sh_test wraps, so the harness can skip its
    // naive ground-truth comparison — a data-driven test run with no data
    // would fail identically on both sides and false-pass.
    let mut tests: Vec<(&str, &str)> = graph
        .tests
        .iter()
        .map(|t| (t.name.as_str(), t.target.as_str()))
        .collect();
    tests.sort_unstable();
    lines.extend(
        tests
            .iter()
            .map(|(name, target)| format!("test {name}_test {target}")),
    );

    let mut assertions: Vec<String> = graph
        .config_headers
        .iter()
        .map(|h| format!("assertion {}_test", codegen::config_header_name(h)))
        .collect();
    assertions.sort_unstable();
    lines.extend(assertions);

    fs::write(
        out_module.join("TARGETS"),
        format!("{}\n", lines.join("\n")),
    )
}

/// Copies the artifacts the project's OWN build system produced (each
/// target's built binary) into `<out_module>/ground_truth/`, alongside a small
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
            copy_into(
                &libtool::ground_truth_source(build_dir, artifact),
                &ground_truth_dir.join(artifact),
            )?;
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
            // A libtool `.la` names its real shared library in its own
            // `dlname=`; stage that chain under the .la's directory, so a
            // ground-truth binary's DT_NEEDED (`liblzma.so.5`) resolves the
            // same way it would for a non-libtool build.
            let (real, staged) = libtool::libtool_shared_library(build_dir, artifact)
                .unwrap_or_else(|| (artifact.to_string(), artifact.to_string()));
            if is_shared_library(&real) {
                for name in
                    stage_shared_library_chain(build_dir, &real, &staged, &ground_truth_dir)?
                {
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
    staged_as: &str,
    ground_truth_dir: &Path,
) -> std::io::Result<Vec<String>> {
    let artifact_path = build_dir.join(artifact);
    let real = fs::canonicalize(&artifact_path)?;
    let artifact_dir = artifact_path.parent().unwrap_or(build_dir);
    // Where the chain is WRITTEN, which is not always where it was found —
    // libtool keeps the real library in a private `.libs/` that no binary's
    // search path reaches. Equal to `artifact`'s own directory otherwise.
    let subdir = Path::new(staged_as).parent();

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
        // `textual_sources` too: a file in `textual_hdrs` is an input Bazel
        // demands on disk exactly like one in `srcs`. Omitting it fails at
        // ANALYSIS with "missing input file", not at compile — a different
        // and more confusing error than the "file not found" you get when the
        // rule never mentioned it at all.
        for relative in target
            .sources
            .iter()
            .chain(target.public_headers.iter())
            .chain(target.textual_sources.iter())
        {
            if copied.insert(relative.as_str()) {
                copy_into(&module_root.join(relative), &out_dir.join(relative))?;
            }
        }
    }

    // configure_file templates are referenced too — the config_header rule
    // expands them (their generated outputs are NOT copied; they're produced
    // at the consumer's build). Usually the template is a source-tree file
    // relative to the module root, like a source; when the project generated
    // it at configure time, `template_source` says where to copy it from
    // instead.
    for config_header in &graph.config_headers {
        if copied.insert(config_header.template.as_str()) {
            // `template_source` is set when the template is not in the source
            // tree — a project that generates it at configure time and then
            // expands it. See `model::ConfigHeader::template_source` for why
            // staging a TEMPLATE is not vendoring a generated HEADER.
            let from = match &config_header.template_source {
                Some(absolute) => absolute.clone(),
                None => module_root.join(&config_header.template),
            };
            copy_into(&from, &out_dir.join(&config_header.template))?;
        }
    }

    // An UNEXPRESSED test's command, when it is a checked-in file. These
    // reach no target — that is what makes them unexpressed — so the loop
    // above never sees them, and the module shipped without them: json-c's
    // 29 `.test` wrappers and xz's 4 `test_*.sh` are all checked in
    // upstream and none arrived. The escalation then told an agent to point
    // an `sh_test` at a file that was not there.
    //
    // Copying is not the same as expressing it: no rule runs these, and the
    // escalation stays open. It makes the item's instruction followable
    // rather than resolving it.
    for test in &graph.unexpressed_tests {
        if test.command.is_empty() || !copied.insert(test.command.as_str()) {
            continue;
        }
        let from = module_root.join(&test.command);
        // Only what the project ships. An absolute command is a system tool
        // (zlib's `/usr/bin/cmake`), and a relative one that does not exist
        // was generated into a build tree this module does not have.
        if Path::new(&test.command).is_relative() && from.is_file() {
            copy_into(&from, &out_dir.join(&test.command))?;
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
    // Escalated tests too. Their scripts, helpers and expected-output files
    // are exactly what a resolution needs, and skipping them left json-c's
    // module missing 74 of the 103 files in its tests/ directory — the
    // escalation named the scripts and nothing carried them.
    for test in graph.tests.iter().chain(&graph.unexpressed_tests) {
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
        // A symlink whose target does not resolve. A project's `configure`
        // can leave one in the BUILD directory — libidn2 does, for the
        // GNUmakefile maintainer wrapper — pointing at a path that stops
        // existing when the action ends. Copying it makes the whole tree
        // artifact invalid ("child GNUmakefile is a dangling symbolic
        // link"), which fails the conversion rather than losing one file.
        //
        // The test is whether it RESOLVES, not whether it is a link, and
        // that distinction is the whole point: Bazel stages an action's
        // inputs AS symlinks, so skipping every link made this function a
        // silent no-op under Bazel for every corpus project — json-c lost
        // 38 `.expected` files and `test-defs.sh`, while direct runs over a
        // real checkout staged them and looked correct.
        //
        // `exists()` follows the link, which is exactly what is wanted here;
        // `symlink_metadata` below is what distinguishes the two cases.
        if entry.file_type().is_ok_and(|t| t.is_symlink()) && !child_src.exists() {
            continue;
        }
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
    // The path is added to the error deliberately. A bare io::Error here says
    // only "No such file or directory", with nothing to say WHICH file — and
    // every caller is copying something the frontend claimed exists, so the
    // interesting information is exactly the path that turned out not to.
    fs::copy(src, dst).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("copying {} to {}: {e}", src.display(), dst.display()),
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    // Bazel stages an action's inputs as symlinks, so under Bazel EVERY
    // entry is one and the dangling-link guard discarded all of them:
    // `copy_test_runtime_data` was a silent no-op for every corpus project,
    // while direct runs over a real checkout staged everything and looked
    // correct. json-c lost 38 `.expected` files and `test-defs.sh` that way.
    //
    // The distinction that matters is whether the target RESOLVES, not
    // whether the entry is a link.
    #[test]
    fn copy_runtime_tree_follows_a_live_symlink() {
        let dir =
            std::env::temp_dir().join(format!("bzlf_livelink_{}_{}", std::process::id(), line!()));
        let real = dir.join("real");
        let src = dir.join("src");
        let dst = dir.join("dst");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(real.join("data.expected"), "expected output\n").unwrap();
        // Exactly Bazel's shape: the input is a link to a file that exists.
        std::os::unix::fs::symlink(real.join("data.expected"), src.join("data.expected")).unwrap();
        std::os::unix::fs::symlink(real.join("data.expected"), src.join("sub/nested.expected"))
            .unwrap();

        super::copy_runtime_tree(&src, &dst).unwrap();

        assert_eq!(
            std::fs::read_to_string(dst.join("data.expected")).unwrap(),
            "expected output\n",
            "a live symlink's CONTENT must be copied — under Bazel this is \
             every input file there is"
        );
        assert!(
            dst.join("sub/nested.expected").is_file(),
            "and through a subdirectory too"
        );
        assert!(
            !std::fs::symlink_metadata(dst.join("data.expected"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "copied as a regular file, not reproduced as a link: the target \
             is a sandbox path that stops existing when the action ends"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_runtime_tree_skips_a_symlink() {
        // A project's configure can create a symlink in the BUILD directory
        // pointing back into the source tree — libidn2 does, for the
        // GNUmakefile maintainer wrapper. Copying it produces a link to a
        // sandbox path that stops existing when the action ends, and Bazel
        // rejects the tree artifact outright: "child GNUmakefile is a
        // dangling symbolic link".
        let dir =
            std::env::temp_dir().join(format!("bzlf_symlink_{}_{}", std::process::id(), line!()));
        let src = dir.join("src");
        let dst = dir.join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("real.txt"), "data").unwrap();
        std::os::unix::fs::symlink("/nonexistent/target", src.join("link")).unwrap();

        super::copy_runtime_tree(&src, &dst).unwrap();

        assert!(
            dst.join("real.txt").is_file(),
            "an ordinary file is still copied"
        );
        assert!(
            !dst.join("link").exists() && std::fs::symlink_metadata(dst.join("link")).is_err(),
            "a symlink must not be reproduced: its target is a path that does \
             not exist outside this conversion"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    use super::*;

    fn summary_of(
        graph: &model::BuildGraph,
        items: &[needs_attention::NeedsAttention],
        tag: &str,
    ) -> serde_json::Value {
        let dir = unique_temp_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        write_conversion_summary(&dir, graph, items).unwrap();
        let text = fs::read_to_string(dir.join("CONVERSION.json")).unwrap();
        fs::remove_dir_all(&dir).ok();
        serde_json::from_str(&text).unwrap()
    }

    fn two_target_graph() -> model::BuildGraph {
        model::BuildGraph {
            module: model::ModuleInfo {
                name: "proj".to_string(),
                version: Some("1.0".to_string()),
            },
            targets: vec![
                model::Target {
                    name: "zzz_app".to_string(),
                    kind: model::TargetKind::Executable,
                    sources: vec!["main.c".to_string()],
                    ..Default::default()
                },
                model::Target {
                    name: "aaa_lib".to_string(),
                    kind: model::TargetKind::Library,
                    is_shared: true,
                    sources: vec!["a.c".to_string(), "b.c".to_string()],
                    ..Default::default()
                },
            ],
            tests: vec![],
            unexpressed_tests: Vec::new(),
            config_headers: vec![],
        }
    }

    // The sweep DIFFS these records, so anything order-dependent reads as a
    // change that never happened. Both lists are sorted; the graph deliberately
    // lists its targets in the wrong order to prove the sort is real.
    #[test]
    fn the_conversion_summary_is_ordered_independently_of_the_graph() {
        let items = vec![
            needs_attention::header_visibility_needs_attention("zzz"),
            needs_attention::header_visibility_needs_attention("aaa"),
        ];
        let summary = summary_of(&two_target_graph(), &items, "summary_order");

        let names: Vec<&str> = summary["targets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["aaa_lib", "zzz_app"], "{summary:#}");

        let subjects: Vec<&str> = summary["escalations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["subject"].as_str().unwrap())
            .collect();
        assert_eq!(subjects, vec!["aaa", "zzz"], "{summary:#}");
    }

    // Escalations are keyed by KIND, never by title or filename: both are
    // derived from prose that gets reworded, so a metric keyed on either
    // silently re-partitions. Two items of the same kind must count as two of
    // that kind rather than collapsing.
    #[test]
    fn the_conversion_summary_counts_escalations_by_kind() {
        let items = vec![
            needs_attention::header_visibility_needs_attention("a"),
            needs_attention::header_visibility_needs_attention("b"),
            needs_attention::unsupported_target_needs_attention("c", "OBJECT_LIBRARY", &[]),
        ];
        let summary = summary_of(&two_target_graph(), &items, "summary_kinds");
        assert_eq!(
            summary["escalations_by_kind"],
            serde_json::json!({ "header_visibility": 2, "unsupported_target": 1 }),
            "{summary:#}"
        );
    }

    // A library's LINKAGE is part of what the conversion produced — a project
    // that stops emitting a shared library has changed, and `kind: Library`
    // alone cannot say so.
    #[test]
    fn the_conversion_summary_distinguishes_shared_from_static_libraries() {
        let mut graph = two_target_graph();
        let summary = summary_of(&graph, &[], "summary_shared");
        assert_eq!(
            summary["targets"][0]["kind"], "shared_library",
            "{summary:#}"
        );

        graph.targets[1].is_shared = false;
        let summary = summary_of(&graph, &[], "summary_static");
        assert_eq!(
            summary["targets"][0]["kind"], "static_library",
            "{summary:#}"
        );
    }

    // A clean conversion must still write the file, with an EMPTY escalation
    // list rather than an absent key. A sweep reading `.escalations` on a
    // green project would otherwise have to treat "missing" and "none" alike,
    // which is the same conflation `needs_attention/MANIFEST` exists to avoid.
    #[test]
    fn a_clean_conversion_still_writes_a_summary() {
        let summary = summary_of(&two_target_graph(), &[], "summary_clean");
        assert_eq!(summary["escalations"], serde_json::json!([]), "{summary:#}");
        assert_eq!(
            summary["escalations_by_kind"],
            serde_json::json!({}),
            "{summary:#}"
        );
    }

    /// A directory no other test or run will pick, created fresh.
    ///
    /// Two simpler schemes were tried and both failed intermittently.
    /// `line!()` is not an identity — editing a comment above it shifts the
    /// number, so a run inherits a DIFFERENT test's leftover tree, and
    /// `create_dir_all` reuses it happily. Adding `process::id()` is not
    /// enough either: the suite runs its tests as THREADS of one process, so
    /// a sibling's `create_dir_all` races this one's cleanup. The counter is
    /// what makes it per-invocation.
    fn unique_temp_dir(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bzlf_{name}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::remove_dir_all(&dir).ok();
        dir
    }

    // The regression this guards: `{error:?}` here instead of `{error}`
    // silently reverts every Display impl in the crate to dead code, and
    // nothing else in the suite would notice.
    #[test]
    fn report_formats_with_display_not_debug() {
        let error = error::Error::ConfigureFailed {
            stderr: "CMake Error at CMakeLists.txt:3 (add_executable):\n  Cannot find source \
                     file:\n\n    does_not_exist.cpp\n"
                .to_string(),
        };

        let reported = report(&error);

        assert!(
            reported.contains("configure step failed:\n"),
            "named for the step, not the build system — both frontends reach \
             this variant:\n{reported}"
        );
        assert!(reported.contains("Cannot find source file"));
        // Debug would name the variant and escape the newlines into one line.
        assert!(
            !reported.contains("ConfigureFailed"),
            "Debug formatting leaked into the message:\n{reported}"
        );
        assert!(
            !reported.contains("\\n"),
            "newlines were escaped rather than printed:\n{reported}"
        );
    }

    // Detection decides which frontend runs, so a wrong answer converts the
    // project with the wrong reader entirely. Both directions and the
    // ambiguous case are pinned.
    #[test]
    fn detect_frontend_reads_the_project_not_a_default() {
        let root = std::env::temp_dir().join(format!("bzlf_det_{}", std::process::id()));
        let make = |name: &str, files: &[&str]| {
            let dir = root.join(name);
            fs::create_dir_all(&dir).unwrap();
            for f in files {
                fs::write(dir.join(f), "").unwrap();
            }
            dir
        };

        assert_eq!(
            detect_frontend(&make("cm", &["CMakeLists.txt"])),
            Some(Frontend::Cmake)
        );
        assert_eq!(
            detect_frontend(&make("ac", &["configure.ac"])),
            Some(Frontend::Autotools)
        );
        // A shipped tarball is already bootstrapped: configure.ac may be
        // absent while `configure` is present.
        assert_eq!(
            detect_frontend(&make("tarball", &["configure"])),
            Some(Frontend::Autotools)
        );
        // A project shipping BOTH is a real choice, not something to infer.
        // CMake wins as the default, and --frontend exists to override it.
        assert_eq!(
            detect_frontend(&make("both", &["CMakeLists.txt", "configure.ac"])),
            Some(Frontend::Cmake)
        );
        // Neither marker is not a project this tool converts. Returning None
        // rather than defaulting means the failure names the real problem
        // instead of surfacing deep inside a frontend.
        assert_eq!(detect_frontend(&make("empty", &[])), None);

        // The override, whose whole reason for existing is the "both" case
        // above: detection says CMake, and xz has to convert as Autotools.
        let both = make("both", &["CMakeLists.txt", "configure.ac"]);
        assert_eq!(
            choose_frontend(Some(Frontend::Autotools), &both),
            Some(Frontend::Autotools),
            "--frontend must beat detection, or a dual-build-system project \
             cannot be converted as anything but CMake"
        );
        assert_eq!(
            choose_frontend(None, &both),
            Some(Frontend::Cmake),
            "and without it, detection still decides"
        );
    }

    #[test]
    fn report_covers_every_error_variant_readably() {
        let variants: Vec<Box<dyn std::error::Error>> = vec![
            Box::new(error::Error::NoProject),
            Box::new(error::Error::BuildFailed {
                stderr: "ninja: build stopped\n".to_string(),
            }),
            Box::new(error::Error::SourceDirOutsideDeliverableRoot {
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
