//! CMake frontend.
//!
//! Configures the target CMake project and reads its resolved output into
//! our internal `BuildGraph` model: the `codemodel-v2` (targets, sources,
//! types, compile groups, install rules) and `cache-v2` (project version,
//! `CMAKE_ROOT`) File API queries, plus the CTest test model (`ctest
//! --show-only=json-v1`), which the File API does not expose. See
//! docs/architecture/cmake-frontend.md for why CMake's own output is the
//! source of truth rather than parsing CMakeLists.txt directly. Pure path
//! geometry this builds on lives in [`crate::paths`].

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::configure_file::{
    self, build_config_headers, is_config_header_output, parse_configure_files,
};
use crate::ctest;
use crate::model::{BuildGraph, ConfigHeader, ModuleInfo, Target, TargetKind};
use crate::needs_attention::{
    NeedsAttention, generated_config_header_needs_attention, generated_sources_needs_attention,
    header_visibility_needs_attention, inert_convenience_targets_needs_attention,
    sources_outside_deliverable_needs_attention, unmapped_config_macros_needs_attention,
    unsupported_target_needs_attention,
};
use crate::paths::{absolutize, common_ancestor, normalize_lexically, resolve_against};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    CmakeConfigureFailed {
        stderr: String,
    },
    CmakeBuildFailed {
        stderr: String,
    },
    NoProject,
    SourceDirOutsideDeliverableRoot {
        source_dir: String,
        deliverable_root: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Json(e) => write!(f, "failed to parse CMake File API JSON: {e}"),
            Error::CmakeConfigureFailed { stderr } => {
                write!(f, "cmake configure failed:\n{stderr}")
            }
            Error::CmakeBuildFailed { stderr } => {
                write!(f, "cmake build failed:\n{stderr}")
            }
            Error::NoProject => write!(f, "codemodel reply contains no project()"),
            Error::SourceDirOutsideDeliverableRoot {
                source_dir,
                deliverable_root,
            } => write!(
                f,
                "the CMake project directory ({source_dir}) is not inside the declared \
                 deliverable root ({deliverable_root}); the deliverable root must contain \
                 the project being converted"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

#[derive(Debug, Deserialize)]
struct CodemodelIndexReply {
    configurations: Vec<CodemodelConfiguration>,
    paths: CodemodelPaths,
}

#[derive(Debug, Deserialize)]
struct CodemodelPaths {
    source: String,
}

#[derive(Debug, Deserialize)]
struct CodemodelConfiguration {
    projects: Vec<CodemodelProject>,
    targets: Vec<CodemodelTargetRef>,
    // Per-directory replies, carrying install() rules among other things.
    // Read to recover install-declared public headers — see
    // `installed_public_headers`.
    #[serde(default)]
    directories: Vec<CodemodelDirectoryRef>,
}

#[derive(Debug, Deserialize)]
struct CodemodelProject {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CodemodelTargetRef {
    id: String,
    #[serde(rename = "jsonFile")]
    json_file: String,
}

#[derive(Debug, Deserialize)]
struct CodemodelDirectoryRef {
    #[serde(rename = "jsonFile")]
    json_file: String,
    // CMake's own flag for whether this directory has any install() rules —
    // lets us skip reading (and skip the whole installers pass for) the
    // common directory that installs nothing.
    #[serde(default)]
    #[serde(rename = "hasInstallRule")]
    has_install_rule: bool,
}

/// A `directory-*.json` reply. Only its `installers` matter here.
#[derive(Debug, Deserialize)]
struct DirectoryReply {
    #[serde(default)]
    installers: Vec<Installer>,
}

/// One `install()` rule as the File API reports it. `install(FILES ... TYPE
/// INCLUDE)` and `install(TARGETS ...)` both land here, distinguished by
/// `installer_type` (`"file"` vs `"target"` vs `"export"`, ...).
#[derive(Debug, Deserialize)]
struct Installer {
    #[serde(rename = "type")]
    installer_type: String,
    // Relative to CMAKE_INSTALL_PREFIX. `None` for installer types that
    // carry no destination. A header installed to `include`/`include/...`
    // is the project declaring it public.
    #[serde(default)]
    destination: Option<String>,
    // Files this installer copies. For a project header these are
    // project-relative; generated files can appear as absolute paths (which
    // are never project sources, so they can't match a target's headers).
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TargetReply {
    name: String,
    #[serde(rename = "type")]
    cmake_type: String,
    sources: Vec<TargetSource>,
    #[serde(default)]
    #[serde(rename = "fileSets")]
    file_sets: Vec<TargetFileSet>,
    #[serde(default)]
    dependencies: Vec<TargetDependency>,
    #[serde(default)]
    artifacts: Vec<TargetArtifact>,
    #[serde(default)]
    #[serde(rename = "compileGroups")]
    compile_groups: Vec<CompileGroup>,
    // The backtraceGraph node index of the command that *defined* this
    // target (e.g. the add_library/add_custom_target call), used to trace a
    // target back to the file it was declared in — which distinguishes a
    // project-authored target from one a CMake module injected via
    // include(). Absent only in replies that carry no backtrace graph.
    #[serde(default)]
    backtrace: Option<usize>,
    #[serde(default)]
    #[serde(rename = "backtraceGraph")]
    backtrace_graph: BacktraceGraph,
}

#[derive(Debug, Deserialize)]
struct CompileGroup {
    #[serde(default)]
    includes: Vec<CompileGroupInclude>,
    // The File API omits this key for a group with no definitions, hence
    // the default. Each entry is the effective define for this group's
    // compilation with its PUBLIC/PRIVATE/INTERFACE origin already erased —
    // see docs/lore/cmake-file-api-compile-definitions-shape.md.
    #[serde(default)]
    defines: Vec<CompileGroupDefine>,
}

#[derive(Debug, Deserialize)]
struct CompileGroupInclude {
    path: String,
    backtrace: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct CompileGroupDefine {
    /// The full macro as CMake would pass it to the compiler: `NAME` for a
    /// bare definition, `NAME=VALUE` when a value is given. Emitted verbatim
    /// into Bazel's `local_defines` — Bazel applies the same `NAME[=VALUE]`
    /// convention, so no reformatting is needed.
    define: String,
}

#[derive(Debug, Default, Deserialize)]
struct BacktraceGraph {
    commands: Vec<String>,
    // The files referenced by nodes below — CMakeLists.txt and any included
    // .cmake modules. A node's `file` indexes into this. Used to tell
    // whether a target was defined in the project's own sources or inside a
    // CMake-provided module (CMAKE_ROOT/Modules/...).
    #[serde(default)]
    files: Vec<String>,
    nodes: Vec<BacktraceNode>,
}

#[derive(Debug, Deserialize)]
struct BacktraceNode {
    command: Option<usize>,
    // Index into BacktraceGraph::files for the file this node is in. The
    // graph's root node can omit it.
    #[serde(default)]
    file: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TargetSource {
    path: String,
    #[serde(rename = "fileSetIndex")]
    file_set_index: Option<usize>,
    /// CMake's own flag for a source it produces during the build rather
    /// than one checked into the project — e.g. an `add_custom_command()`
    /// output, or the object files an `OBJECT_LIBRARY` splices into its
    /// consumers. Reported as an ABSOLUTE path into the CMake build
    /// directory, so it is never a valid Bazel source label.
    #[serde(default)]
    #[serde(rename = "isGenerated")]
    is_generated: bool,
}

#[derive(Debug, Deserialize)]
struct TargetFileSet {
    #[serde(rename = "type")]
    fileset_type: String,
    visibility: String,
}

#[derive(Debug, Deserialize)]
struct TargetDependency {
    id: String,
}

#[derive(Debug, Deserialize)]
struct TargetArtifact {
    path: String,
}

#[derive(Debug, Deserialize)]
struct CacheReply {
    entries: Vec<CacheEntry>,
}

#[derive(Debug, Deserialize)]
struct CacheEntry {
    name: String,
    value: String,
}

/// A completed discovery pass: the build graph, the gaps that kept parts of
/// the project out of it, and the directory on this machine that the
/// graph's (module-relative) paths are relative to.
pub struct Discovery {
    pub graph: BuildGraph,
    /// Gaps to escalate for this conversion — see
    /// docs/architecture/needs-attention-interface.md.
    pub needs_attention: Vec<NeedsAttention>,
    /// Absolute path to the converted module's root — where
    /// `copy_referenced_sources` reads the referenced files from. Equal to
    /// the CMake project directory unless the build referenced files above
    /// it that still ship with the project; see `rebase_to_module_root`.
    pub module_root: PathBuf,
}

/// What one codemodel reply yields. Named rather than returned as a tuple:
/// its two `Vec` members and two path-ish members are easy to transpose at
/// a call site, and the compiler would not notice.
struct Codemodel {
    project_name: String,
    targets: Vec<Target>,
    needs_attention: Vec<NeedsAttention>,
    module_root: PathBuf,
}

/// Takes a CMake project through configure and build, and reads the File
/// API replies into a `BuildGraph`.
///
/// The two orderings below are load-bearing, and neither is visible from
/// the calls themselves:
///
/// - Queries are written before `configure`, not after. CMake answers only
///   the queries already present in the build directory when it configures;
///   writing them afterwards produces no reply at all until something
///   configures again.
/// - The project is really built, not just configured. That step exists to
///   produce the ground-truth artifacts the equivalence check compares
///   against, which is why discovery owns it rather than the caller — see
///   docs/architecture/build-verification.md.
pub fn discover(
    source_dir: &Path,
    build_dir: &Path,
    deliverable_root: &Path,
) -> Result<Discovery, Error> {
    request_file_api_queries(build_dir)?;
    let trace = configure(source_dir, build_dir)?;
    build(build_dir)?;

    // configure_file calls (template -> output) come from the configure
    // trace, not the File API — see parse_configure_files.
    let abs_source_dir = absolutize(source_dir)?;
    let configure_files = parse_configure_files(&trace, &abs_source_dir);

    // The generated config headers appear in a target's sources as absolute
    // build-dir paths. read_codemodel_reply must know their names so it drops
    // them (a config_header rule reproduces them, wired in via a label) rather
    // than escalating them as unreachable build-dir sources.
    let config_header_outputs: HashSet<String> = configure_files
        .iter()
        .filter(|c| is_config_header_output(&c.output))
        .filter_map(|c| {
            c.output
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .collect();

    let reply_dir = build_dir.join(".cmake/api/v1/reply");
    let deliverable_root = absolutize(deliverable_root)?;
    // Absolute so it can be compared against the absolute paths the File API
    // reports for build-directory outputs (configure_file headers) — see
    // rebase_to_module_root's partitioning of unreachable sources.
    let abs_build_dir = absolutize(build_dir)?;
    let codemodel = read_codemodel_reply(
        &reply_dir,
        &deliverable_root,
        &abs_build_dir,
        &config_header_outputs,
    )?;
    let version = read_project_version(&reply_dir)?;

    // Tests come from ctest, not the File API (see read_tests), and their
    // working directories are rebased against the same module root the
    // targets' paths were.
    let mut tests = ctest::read_tests(build_dir)?;
    ctest::rebase_tests_to_module_root(&mut tests, &codemodel.module_root);
    let (tests, test_escalation) =
        ctest::partition_tests_by_buildable_command(tests, &codemodel.targets);

    let cache = read_cache_values(&reply_dir)?;
    let (config_headers, config_escalations) =
        build_config_headers(&configure_files, &codemodel.module_root, &cache);
    let mut needs_attention = codemodel.needs_attention;
    needs_attention.extend(config_escalations);
    needs_attention.extend(test_escalation);

    Ok(Discovery {
        graph: BuildGraph {
            module: ModuleInfo {
                name: codemodel.project_name,
                version,
            },
            targets: codemodel.targets,
            tests,
            config_headers,
        },
        needs_attention,
        module_root: codemodel.module_root,
    })
}

fn request_file_api_queries(build_dir: &Path) -> Result<(), Error> {
    let query_dir = build_dir.join(".cmake/api/v1/query");
    fs::create_dir_all(&query_dir)?;
    fs::write(query_dir.join("codemodel-v2"), "")?;
    fs::write(query_dir.join("cache-v2"), "")?;
    Ok(())
}

/// Configures the project, returning CMake's `--trace-expand` output (on
/// stderr). The trace is the only place `configure_file` calls are reported
/// — the File API models them not at all — so it's captured here rather than
/// running cmake a second time. See
/// docs/lore/cmake-configure-file-is-in-the-trace-not-the-file-api.md.
fn configure(source_dir: &Path, build_dir: &Path) -> Result<String, Error> {
    let output = Command::new("cmake")
        .arg("--trace-expand")
        .arg("-G")
        .arg("Ninja")
        .arg("-B")
        .arg(build_dir)
        .arg("-S")
        .arg(source_dir)
        .output()?;

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(Error::CmakeConfigureFailed { stderr });
    }
    Ok(stderr)
}

fn build(build_dir: &Path) -> Result<(), Error> {
    let output = Command::new("cmake")
        .arg("--build")
        .arg(build_dir)
        .output()?;

    if !output.status.success() {
        return Err(Error::CmakeBuildFailed {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

fn read_codemodel_reply(
    reply_dir: &Path,
    deliverable_root: &Path,
    build_dir: &Path,
    config_header_outputs: &HashSet<String>,
) -> Result<Codemodel, Error> {
    let index_path = find_reply_file(reply_dir, "codemodel-v2-")?;
    let index: CodemodelIndexReply = serde_json::from_str(&fs::read_to_string(index_path)?)?;

    let configuration = index.configurations.first().ok_or(Error::NoProject)?;
    let project = configuration.projects.first().ok_or(Error::NoProject)?;

    // Read every target's reply once, keeping the codemodel's own order so
    // the order recorded in an escalation is deterministic across runs. All
    // of them are read before any is translated, since a target can name one
    // that appears later as a dependency.
    let mut replies = Vec::with_capacity(configuration.targets.len());
    for target_ref in &configuration.targets {
        let target_path = reply_dir.join(&target_ref.json_file);
        let reply: TargetReply = serde_json::from_str(&fs::read_to_string(target_path)?)?;
        replies.push((target_ref.id.as_str(), reply));
    }

    // Directory replies carry install() rules; read only those that have any
    // (hasInstallRule) to recover install-declared public headers. A project
    // with no install rules skips this entirely.
    let mut directory_replies = Vec::new();
    for dir_ref in &configuration.directories {
        if !dir_ref.has_install_rule {
            continue;
        }
        let dir_path = reply_dir.join(&dir_ref.json_file);
        let dir_reply: DirectoryReply = serde_json::from_str(&fs::read_to_string(dir_path)?)?;
        directory_replies.push(dir_reply);
    }
    let installed_headers = installed_public_headers(&directory_replies);

    // Target dependencies are reported by opaque id (e.g.
    // "greet::@6890427a1f51a3e7e1df"), not name, so translating one needs a
    // lookup back to names. Only targets that actually get a Bazel rule are
    // in it: an untranslatable target's id then resolves to nothing, which
    // is what every lookup here wants anyway — a `deps` edge naming it is
    // dropped rather than emitted as a label pointing at a target that was
    // never generated (which would fail at Bazel *analysis* time, with an
    // error far removed from the real cause), and it is not listed as a
    // dependent whose edge was lost. Keying that decision on the id rather
    // than the name also means no amount of name collision can confuse two
    // targets for each other.
    let translated_names: HashMap<&str, &str> = replies
        .iter()
        .filter(|(_, reply)| target_kind(&reply.cmake_type).is_some())
        .map(|(id, reply)| (*id, reply.name.as_str()))
        .collect();

    // Reverse the dependency edges once: target id -> ids of the targets
    // naming it as a dependency. Two questions read off this, and both are
    // asked below per target:
    //
    //  - does anything depend on it at all? (a library nothing depends on
    //    has no consumer that could need a header it isn't exposing, so an
    //    unexposed header there isn't worth flagging — see
    //    docs/architecture/cmake-frontend.md)
    //  - which *translated* targets depend on it? (an escalation for a
    //    skipped target has to name the targets whose edge was dropped)
    let mut dependents_of: HashMap<&str, Vec<&str>> = HashMap::new();
    for (id, reply) in &replies {
        for dep in &reply.dependencies {
            dependents_of.entry(dep.id.as_str()).or_default().push(id);
        }
    }

    // Provenance for the injected-target filter: a target defined under this
    // path came from an include()d CMake module, not the project. Read once.
    let cmake_root = read_cmake_root(reply_dir)?;

    let mut targets = Vec::new();
    let mut needs_attention = Vec::new();
    // Project-authored convenience targets (UTILITY-ish, no artifacts, no
    // dependents) that aren't from a CMake module — aggregated into ONE
    // escalation at the end instead of one apiece, so a project with a
    // handful of docs/format targets doesn't bury the real gaps.
    let mut inert_convenience: Vec<String> = Vec::new();

    for (id, reply) in &replies {
        let Some(kind) = target_kind(&reply.cmake_type) else {
            // Two kinds of untranslatable target never warrant an individual
            // escalation, because there is nothing for an agent to translate:
            //
            //  - injected by a CMake module (CTest's dashboard targets, a
            //    Doxygen `doc` target): dropped silently — the project didn't
            //    author it and it has no place in the Bazel build. See
            //    docs/lore/cmake-include-ctest-injects-utility-targets.md.
            //  - a project's own convenience target with no artifact and no
            //    dependents: collected for one aggregated note below.
            //
            // Both require the target to be inert (no artifacts, no
            // dependents); an injected OR convenience-shaped target that is
            // actually load-bearing falls through to a normal escalation,
            // because dropping it would leave real dependents incomplete.
            if is_inert_target(reply, &dependents_of, id) {
                if is_cmake_provided(reply, cmake_root.as_deref()) {
                    continue;
                }
                inert_convenience.push(reply.name.clone());
                continue;
            }

            // A load-bearing target whose CMake type has no Bazel rule yet is
            // escalated rather than aborting the whole conversion — one
            // unrecognized target must not cost the project every other
            // target it defines. See docs/architecture/cmake-frontend.md. The
            // edges it cost are named, since dropping them silently would
            // leave dependents quietly incomplete.
            let dropped_edges: Vec<String> = dependents_of
                .get(id)
                .into_iter()
                .flatten()
                .filter_map(|dependent| translated_names.get(dependent).map(|n| n.to_string()))
                .collect();
            needs_attention.push(unsupported_target_needs_attention(
                &reply.name,
                &reply.cmake_type,
                &dropped_edges,
            ));
            continue;
        };

        let is_depended_on = dependents_of.contains_key(id);
        let (target, attention) = to_target(
            reply,
            kind,
            &translated_names,
            is_depended_on,
            &installed_headers,
        );

        targets.push(target);
        needs_attention.extend(attention);
    }

    if !inert_convenience.is_empty() {
        needs_attention.push(inert_convenience_targets_needs_attention(
            &inert_convenience,
        ));
    }

    let source_dir = normalize_lexically(Path::new(&index.paths.source));
    if !source_dir.starts_with(deliverable_root) {
        return Err(Error::SourceDirOutsideDeliverableRoot {
            source_dir: source_dir.to_string_lossy().into_owned(),
            deliverable_root: deliverable_root.to_string_lossy().into_owned(),
        });
    }

    inject_unenumerated_installed_headers(&mut targets, &installed_headers, &source_dir);
    // After the install()-declared pass, whose headers this one then sees as
    // already accounted for. Both run before rebasing, while sources are
    // project-relative and include dirs absolute.
    inject_headers_on_include_dirs(&mut targets, &source_dir);

    let (module_root, rebase_escalations) = rebase_to_module_root(
        &mut targets,
        &source_dir,
        deliverable_root,
        build_dir,
        config_header_outputs,
    );
    needs_attention.extend(rebase_escalations);

    Ok(Codemodel {
        project_name: project.name.clone(),
        targets,
        needs_attention,
        module_root,
    })
}

/// Resolves `.` and `..` textually, without touching the filesystem.
///
/// Deliberately not `fs::canonicalize`: under a Bazel sandbox the source
/// tree is a web of symlinks into the execroot and the output base, and
/// resolving them would yield paths that are correct on this machine but
/// meaningless as a description of the module's layout.
/// Chooses the converted module's root and rewrites every path in `targets`
/// to be relative to it.
///
/// The root is the deepest directory containing both the CMake project and
/// every referenced file that ships with the project — so it is simply the
/// project directory when nothing reaches outside it (the common case, and
/// what every fixture but one does), and widens only as far as it must
/// otherwise. `deliverable_root` caps that widening: a file outside it
/// cannot be reproduced from what the project ships, so the module is not
/// grown to swallow it and it is escalated instead. See
/// docs/architecture/cmake-frontend.md.
///
/// Include directories are treated differently from sources on the way
/// out: one that lands outside the module is a system include path
/// (`/usr/include`), which has no `includes` translation and is not a gap
/// worth reporting.
fn rebase_to_module_root(
    targets: &mut [Target],
    source_dir: &Path,
    deliverable_root: &Path,
    build_dir: &Path,
    config_header_outputs: &HashSet<String>,
) -> (PathBuf, Vec<NeedsAttention>) {
    let mut shipped = Vec::new();
    for target in targets.iter() {
        for path in target
            .sources
            .iter()
            .chain(&target.public_headers)
            .chain(&target.includes)
        {
            let absolute = resolve_against(path, source_dir);
            if absolute.starts_with(deliverable_root) {
                shipped.push(absolute);
            }
        }
    }
    let module_root = common_ancestor(source_dir, &shipped);

    let mut escalations = Vec::new();
    for target in targets.iter_mut() {
        // A source the module can't reach is one of two very different
        // things, distinguished by whether it lives under the CMake build
        // directory: a build-time-generated header (a configure_file output
        // — the build_dir case) versus a file under some sibling source
        // directory the deliverable root was drawn too narrowly to include.
        // They get different escalations because their resolutions are
        // different — see generated_config_header_needs_attention.
        let mut build_dir_outputs = Vec::new();
        let mut outside_deliverable = Vec::new();

        for list in [&mut target.sources, &mut target.public_headers] {
            let mut kept = Vec::with_capacity(list.len());
            for path in list.iter() {
                let absolute = resolve_against(path, source_dir);
                match absolute.strip_prefix(&module_root) {
                    Ok(relative) => kept.push(relative.to_string_lossy().into_owned()),
                    Err(_) => {
                        // A build-dir source that a config_header rule
                        // reproduces is neither kept nor escalated: it's
                        // supplied to this target by the config header's label
                        // (codegen folds :config_h into srcs). Dropping it here
                        // is what reconciles the two mechanisms — otherwise the
                        // same header is both escalated and regenerated.
                        let is_reproduced_config_header = absolute.starts_with(build_dir)
                            && absolute
                                .file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| config_header_outputs.contains(n));
                        if is_reproduced_config_header {
                            continue;
                        }
                        let display = absolute.to_string_lossy().into_owned();
                        if absolute.starts_with(build_dir) {
                            build_dir_outputs.push(display);
                        } else {
                            outside_deliverable.push(display);
                        }
                    }
                }
            }
            *list = kept;
        }

        target.includes = target
            .includes
            .iter()
            .filter_map(|path| {
                let relative = resolve_against(path, source_dir)
                    .strip_prefix(&module_root)
                    .ok()?
                    .to_string_lossy()
                    .into_owned();
                // The module root itself isn't expressible as an `includes`
                // entry, and adds nothing Bazel doesn't already do.
                (!relative.is_empty()).then_some(relative)
            })
            .collect();

        if !outside_deliverable.is_empty() {
            escalations.push(sources_outside_deliverable_needs_attention(
                &target.name,
                &outside_deliverable,
            ));
        }
        if !build_dir_outputs.is_empty() {
            escalations.push(generated_config_header_needs_attention(
                &target.name,
                &build_dir_outputs,
            ));
        }
    }

    (module_root, escalations)
}

/// Maps a CMake target type onto the internal model's kind. `None` means
/// the translator has no Bazel rule for it yet — the caller escalates via
/// `needs_attention/` rather than failing the conversion.
fn target_kind(cmake_type: &str) -> Option<TargetKind> {
    match cmake_type {
        "EXECUTABLE" => Some(TargetKind::Executable),
        "STATIC_LIBRARY" | "SHARED_LIBRARY" => Some(TargetKind::Library),
        _ => None,
    }
}

/// Reads `CMAKE_PROJECT_VERSION` from the cache-v2 reply, when CMake's
/// top-level `project()` call specified a `VERSION`. Returns `None`
/// otherwise (CMake never sets the cache entry in that case).
fn read_project_version(reply_dir: &Path) -> Result<Option<String>, Error> {
    let cache_path = find_reply_file(reply_dir, "cache-v2-")?;
    let cache: CacheReply = serde_json::from_str(&fs::read_to_string(cache_path)?)?;

    Ok(cache
        .entries
        .into_iter()
        .find(|e| e.name == "CMAKE_PROJECT_VERSION")
        .map(|e| e.value)
        .filter(|v| !v.is_empty()))
}

/// Reads `CMAKE_ROOT` from the cache-v2 reply — the root of the CMake
/// installation (e.g. `/usr/share/cmake-3.28`). Targets whose defining
/// command lives under this path were injected by an `include()` of a
/// CMake-provided module (CTest's dashboard targets, a Doxygen module's
/// `doc` target, ...), not authored by the project — see
/// `defining_command_file` and docs/lore/cmake-include-ctest-injects-utility-targets.md.
///
/// `CMAKE_ROOT` is always present in a normal cache; `None` only guards a
/// malformed reply, and every provenance check treats `None` as "cannot
/// prove it's CMake-provided," i.e. errs toward escalating rather than
/// silently dropping.
fn read_cmake_root(reply_dir: &Path) -> Result<Option<String>, Error> {
    let cache_path = find_reply_file(reply_dir, "cache-v2-")?;
    let cache: CacheReply = serde_json::from_str(&fs::read_to_string(cache_path)?)?;

    Ok(cache
        .entries
        .into_iter()
        .find(|e| e.name == "CMAKE_ROOT")
        .map(|e| e.value)
        .filter(|v| !v.is_empty()))
}

/// The whole cache as a name -> value map, for resolving a `configure_file`
/// template's `@VAR@` substitutions (which reference arbitrary CMake
/// variables). Unlike `read_project_version`/`read_cmake_root`, which pick a
/// single entry, this returns everything since a template can reference any of
/// it.
fn read_cache_values(reply_dir: &Path) -> Result<HashMap<String, String>, Error> {
    let cache_path = find_reply_file(reply_dir, "cache-v2-")?;
    let cache: CacheReply = serde_json::from_str(&fs::read_to_string(cache_path)?)?;
    Ok(cache
        .entries
        .into_iter()
        .map(|e| (e.name, e.value))
        .collect())
}

/// Finds a File API reply by filename prefix, since the replies cannot be
/// opened by name: CMake documents them as
/// `<kind>-v<major>-<unspecified>.json`, and the trailing part is CMake's
/// to choose. The prefix a query asked for (`codemodel-v2-`, `cache-v2-`)
/// is the whole of the stable part.
///
/// Taking the first match is safe only because the translator writes
/// exactly one query per kind, into a build directory it also created, so
/// one reply per prefix exists. Resolving names through the reply index
/// would be the general answer, for a reply directory this translator did
/// not produce.
fn find_reply_file(reply_dir: &Path, prefix: &str) -> Result<PathBuf, Error> {
    for entry in fs::read_dir(reply_dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str()
            && name.starts_with(prefix)
        {
            return Ok(entry.path());
        }
    }
    Err(Error::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("no reply file with prefix '{prefix}' in {reply_dir:?}"),
    )))
}

fn to_target(
    reply: &TargetReply,
    kind: TargetKind,
    translated_names: &HashMap<&str, &str>,
    is_depended_on: bool,
    installed_headers: &HashSet<String>,
) -> (Target, Vec<NeedsAttention>) {
    let mut sources = Vec::new();
    let mut public_headers = Vec::new();
    let mut generated_sources = Vec::new();
    let mut has_unclassified_headers = false;
    for source in &reply.sources {
        // The translator can't produce a generated file, and has no way to
        // know what does.
        if source.is_generated {
            // Every add_custom_command() output arrives with a phantom
            // "<output>.rule" sibling in the File API reply — Ninja/Make
            // build-graph bookkeeping that names no file on disk, not a
            // second missing input. Left in, it would read to an agent as
            // a second generated source to account for. See
            // docs/lore/cmake-file-api-generated-source-shape.md.
            if source.path.ends_with(".rule") {
                continue;
            }
            generated_sources.push(source.path.clone());
            continue;
        }

        // Two authoritative signals that a header is public, either
        // sufficient: a target_sources FILE_SET (CMake 3.23+), or an
        // install(FILES ... TYPE INCLUDE) rule (the pre-FILE_SET way, and
        // the only one many real projects use) — see installed_public_headers.
        let is_file_set_public = source
            .file_set_index
            .and_then(|i| reply.file_sets.get(i))
            .is_some_and(|fs| {
                fs.fileset_type == "HEADERS"
                    && (fs.visibility == "PUBLIC" || fs.visibility == "INTERFACE")
            });
        let is_public_header = is_file_set_public || installed_headers.contains(&source.path);

        if is_public_header {
            public_headers.push(source.path.clone());
        } else {
            if looks_like_header(&source.path) {
                has_unclassified_headers = true;
            }
            sources.push(source.path.clone());
        }
    }

    // An id missing from the map belongs to a target that was never
    // translated, so the edge is dropped here rather than emitted as a
    // dangling label — see `translated_names` in `read_codemodel_reply`.
    let dependencies: Vec<String> = reply
        .dependencies
        .iter()
        .filter_map(|d| translated_names.get(d.id.as_str()).map(|n| n.to_string()))
        .collect();

    let includes = own_include_dirs(reply);
    let local_defines = target_defines(reply);

    let mut needs_attention = Vec::new();
    if kind == TargetKind::Library
        && public_headers.is_empty()
        && has_unclassified_headers
        && is_depended_on
    {
        needs_attention.push(header_visibility_needs_attention(&reply.name));
    }
    if !generated_sources.is_empty() {
        needs_attention.push(generated_sources_needs_attention(
            &reply.name,
            &generated_sources,
        ));
    }

    let target = Target {
        name: reply.name.clone(),
        kind,
        sources,
        public_headers,
        dependencies,
        includes,
        local_defines,
        artifacts: reply.artifacts.iter().map(|a| a.path.clone()).collect(),
    };

    (target, needs_attention)
}

/// Whether a plain source looks like a header, by extension. Only ever used
/// to decide whether a library with no public `FILE_SET` is worth
/// escalating — nothing is classified or emitted differently on the
/// strength of it.
///
/// Extension is the only signal available: CMake reports these as ordinary
/// sources precisely because the project never declared them as headers.
/// The list is therefore conservative, and being wrong is one-directional —
/// an unlisted extension (`.inc`, `.ipp`, an extensionless C++ header)
/// means a gap goes unreported, never that a file is misplaced in the
/// generated output.
fn looks_like_header(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("h") | Some("hpp") | Some("hh") | Some("hxx")
    )
}

/// Extracts this target's OWN include directories from its compile groups,
/// excluding ones inherited from a dependency via `target_link_libraries`.
/// Returned exactly as the File API reported them — absolute;
/// `rebase_to_module_root` makes them module-relative once the module root
/// is known.
///
/// The File API doesn't separately expose "this target's own
/// target_include_directories() dirs" vs. "inherited from a linked
/// target" — both appear identically in `compileGroups[].includes`. The
/// distinguishing signal is each include's `backtrace`: CMake attributes
/// inherited includes to the `target_link_libraries` call that pulled
/// them in, while includes declared directly on this target (via
/// `target_include_directories`, or a `target_sources` FILE_SET's
/// `BASE_DIRS`) trace to some other command. Bazel's `includes` is
/// transitive, so only the target's own dirs need to be captured — a
/// consuming target already gets them via its `deps` edge.
fn own_include_dirs(reply: &TargetReply) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut includes = Vec::new();

    for group in &reply.compile_groups {
        for include in &group.includes {
            if is_inherited_via_link_libraries(include.backtrace, &reply.backtrace_graph) {
                continue;
            }

            if seen.insert(include.path.clone()) {
                includes.push(include.path.clone());
            }
        }
    }

    includes
}

/// Collects the preprocessor definitions effective on this target's own
/// compilation, deduplicated, in first-seen order.
///
/// Unlike `own_include_dirs`, this does NOT filter by backtrace to drop
/// inherited entries: everything here becomes Bazel `local_defines` (Layer
/// A), which are non-propagating, so a define that reached this target from
/// a dependency's PUBLIC visibility is *supposed* to be re-stated on this
/// target rather than inherited — that is exactly what makes the flattened
/// per-target set self-consistent without reconstructing propagation. The
/// backtrace-based own-vs-inherited split is Layer B (bzl-c54.3). See
/// `model::Target::local_defines` and
/// docs/lore/cmake-file-api-compile-definitions-shape.md.
fn target_defines(reply: &TargetReply) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut defines = Vec::new();

    for group in &reply.compile_groups {
        for define in &group.defines {
            if seen.insert(define.define.clone()) {
                defines.push(define.define.clone());
            }
        }
    }

    defines
}

/// Whether an `install(FILES ...)` destination is a public-header include
/// directory. Matches a leading `include` (`install(... DESTINATION include)`,
/// the relative form) AND an absolute one anywhere in the path
/// (`.../include/json-c`, which is what `CMAKE_INSTALL_FULL_INCLUDEDIR`
/// expands to — json-c installs there). The absolute form is common: a project
/// that uses the `GNUInstallDirs`/`CMAKE_INSTALL_FULL_*` variables gets a fully
/// resolved `<prefix>/include/...` rather than a bare `include`.
///
/// A nested `include` under `lib`/`lib64`/`share` is deliberately NOT matched:
/// `lib/include` is a build-private include tree, not the public install
/// location, and treating its contents as public headers would be wrong.
fn is_include_destination(destination: &str) -> bool {
    let components: Vec<&str> = Path::new(destination)
        .components()
        .filter_map(|c| match c {
            Component::Normal(c) => c.to_str(),
            _ => None,
        })
        .collect();
    components.iter().enumerate().any(|(i, &c)| {
        c == "include"
            && !matches!(
                i.checked_sub(1).map(|p| components[p]),
                Some("lib") | Some("lib64") | Some("share")
            )
    })
}

/// The set of header paths the project installs to an include destination,
/// gathered across every directory reply — the project's own authoritative
/// declaration of which headers are public, via `install(FILES ... TYPE
/// INCLUDE)`. This is the pre-FILE_SET way to declare a public header, and
/// the only signal for the many projects that never adopted `FILE_SET`.
///
/// Paths are returned exactly as the installer reported them (project-relative
/// for checked-in headers); a target's own source paths are matched against
/// this set by `to_target`. Absolute paths (generated files) can appear in an
/// installer and are kept as-is — they simply never match a project source.
fn installed_public_headers(directories: &[DirectoryReply]) -> HashSet<String> {
    let mut headers = HashSet::new();
    for dir in directories {
        for installer in &dir.installers {
            if installer.installer_type != "file" {
                continue;
            }
            let Some(destination) = &installer.destination else {
                continue;
            };
            if !is_include_destination(destination) {
                continue;
            }
            for path in &installer.paths {
                headers.insert(path.clone());
            }
        }
    }
    headers
}

/// Adds public headers the project `install()`s to an include destination but
/// that NO target's source list enumerated, attaching each to the library
/// whose include path it sits on.
///
/// Why this is needed at all: a huge class of C projects list only `.c` files
/// on a target and leave headers ambient on the include path (see
/// docs/architecture/cmake-frontend.md). CMake compiles each `.c` and finds
/// its headers via `-I`; the header is never a build input, so the File API
/// never reports it as a target source, so `copy_referenced_sources` never
/// copies it — and the converted library fails to compile the moment one of
/// its `.c` files `#include`s it. json-c is the live case: a CMakeLists
/// ordering bug drops `json_pointer.h`/`json_patch.h` from the library's
/// header list, yet `json_pointer.c` includes `json_pointer.h`.
///
/// The `install(FILES ... DESTINATION <include>)` rule is the project's own
/// authoritative statement that these headers are public, so an unenumerated
/// one is added to `public_headers` (→ a library's `hdrs`), not `sources`.
/// This is deliberately narrower than "copy every header under the include
/// dirs": only headers the project explicitly declared public are injected, so
/// a header with no such evidence still defaults to whatever the target already
/// said about it (private, or absent) rather than being guessed public.
///
/// Attribution is by include path: the header is added to every LIBRARY whose
/// own include directories contain the header's parent directory. That handles
/// json-c's two libraries (shared + static, same include dirs) getting the
/// same headers, and avoids attaching a header to a library that can't see it.
///
/// Paths here are still in the File API's frame (pre-rebase): target
/// `sources`/`public_headers` and the relative install paths are
/// project-relative; include dirs and the generated headers' install paths are
/// absolute. An install path that resolves outside `source_dir` (a generated
/// header in the build dir, like `json.h`) is skipped — those are reproduced
/// by the config_header machinery, not copied.
fn inject_unenumerated_installed_headers(
    targets: &mut [Target],
    installed_headers: &HashSet<String>,
    source_dir: &Path,
) {
    // Every header path any target already accounts for, so an install-declared
    // header a target DID enumerate isn't added a second time. Owned (not
    // borrowed from `targets`) so the mutable pass below is free to push.
    let enumerated: HashSet<String> = targets
        .iter()
        .flat_map(|t| t.sources.iter().chain(t.public_headers.iter()))
        .cloned()
        .collect();

    // Sorted: `installed_headers` is a HashSet, and pushing in iteration order
    // would make `public_headers` (hence the generated `hdrs`) nondeterministic
    // across runs — the reproducible-output invariant every other list holds.
    let mut candidates: Vec<&String> = installed_headers.iter().collect();
    candidates.sort();

    for header in candidates {
        if enumerated.contains(header.as_str()) {
            continue;
        }
        // Resolve to absolute to both bound it to the source tree and match it
        // against the (absolute) include dirs. An absolute install path that is
        // not under source_dir is a generated/out-of-tree file — skip it.
        let absolute = if Path::new(header).is_absolute() {
            normalize_lexically(Path::new(header))
        } else {
            normalize_lexically(&source_dir.join(header))
        };
        if !absolute.starts_with(source_dir) || !absolute.is_file() {
            continue;
        }
        let Some(parent) = absolute.parent() else {
            continue;
        };

        for target in targets.iter_mut() {
            if target.kind != TargetKind::Library {
                continue;
            }
            let on_include_path = target
                .includes
                .iter()
                .any(|dir| normalize_lexically(Path::new(dir)) == parent);
            if on_include_path && !target.public_headers.contains(header) {
                target.public_headers.push(header.clone());
            }
        }
    }
}

/// Attaches every header sitting in a target's own include directories that
/// no target enumerated, so Bazel stages it into the compile sandbox.
///
/// The two build systems disagree about what a header *is*. CMake compiles a
/// `.c` and lets the preprocessor find headers on disk via `-I`; a header is
/// therefore never a build input and the File API never reports it as a
/// target source. Bazel stages only DECLARED inputs, so the same header is
/// absent at compile time and the build fails with `file not found` while the
/// `-I` flag is present and correct.
///
/// **Everything on the include path is an input.** That is CMake's own
/// semantic, and it is the whole declaration available: CMake has no per-file
/// statement that a target uses one header from a directory and not another,
/// so there is nothing here to be more precise than.
///
/// An earlier version of this scanned each source for `#include "..."` and
/// staged only the named headers. That was rejected: it re-implemented a
/// preprocessor badly (no `#if` evaluation, no computed includes, quoted form
/// only), it made the frontend read source text when its source of truth is
/// the File API (docs/architecture/cmake-frontend.md), and it bought nothing
/// — on json-c the two approaches select the identical set of headers.
///
/// Distinct from `inject_unenumerated_installed_headers`, which acts on the
/// project's `install()` declarations — its authoritative statement that a
/// header is PUBLIC — and populates a library's `public_headers` (→ `hdrs`).
/// An include directory carries no such claim, so headers land in `sources`,
/// where a header contributes nothing but its presence in the sandbox.
///
/// Include dirs outside the source tree are skipped: those are the build
/// directory (whose headers the config_header machinery reproduces) and
/// toolchain paths, neither of which the module may carry. Non-recursive, so
/// a directory on the include path contributes the headers a compiler would
/// actually resolve from it rather than an entire nested tree.
///
/// Paths are in the File API's frame (pre-rebase): `sources` are
/// project-relative, include dirs absolute.
fn inject_headers_on_include_dirs(targets: &mut [Target], source_dir: &Path) {
    for target in targets.iter_mut() {
        // Per-target, NOT global: a header can be enumerated on one target and
        // reachable only via the include path on another, and asking "does any
        // target list it?" would suppress it exactly where it is needed.
        // json-c is the live case — `test1Formatted` lists parse_flags.h while
        // its sibling `test1` (same include dir, one source) does not.
        let enumerated: HashSet<&str> = target
            .sources
            .iter()
            .chain(target.public_headers.iter())
            .map(String::as_str)
            .collect();

        let mut discovered: Vec<String> = Vec::new();

        for dir in &target.includes {
            let absolute = normalize_lexically(Path::new(dir));
            // The `strip_prefix` below would also reject an outside directory,
            // so this looks redundant and is not: it skips the `read_dir` of a
            // toolchain include path entirely (/usr/include and friends are
            // large), rather than listing it and discarding every entry.
            if !absolute.starts_with(source_dir) {
                continue;
            }
            let Ok(entries) = fs::read_dir(&absolute) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() || !is_header_file(&path) {
                    continue;
                }
                let Ok(relative) = path.strip_prefix(source_dir) else {
                    continue;
                };
                let relative = relative.to_string_lossy().into_owned();
                if !enumerated.contains(relative.as_str()) && !discovered.contains(&relative) {
                    discovered.push(relative);
                }
            }
        }

        // Sorted: read_dir order is filesystem-dependent, and the generated
        // `srcs` must not vary between runs.
        discovered.sort();
        target.sources.extend(discovered);
    }
}

/// Whether a path names a C/C++ header by extension. Extension-based because
/// that is what a compiler's include resolution and CMake's own header
/// classification go on; a directory on the include path can also hold `.c`
/// files (json-c's `tests/`), which are sources of some other target and must
/// not be swept in as headers.
fn is_header_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "h" | "hpp" | "hh" | "hxx" | "inc" | "ipp"))
}

/// The file in which this target's defining command (add_library,
/// add_custom_target, ...) appears, per its backtrace, or `None` if the
/// reply carries no usable backtrace for it. Absolute for included modules
/// (`/usr/share/cmake-3.28/Modules/CTestTargets.cmake`) and repo-relative
/// for the project's own (`CMakeLists.txt`), exactly as CMake records them.
fn defining_command_file(reply: &TargetReply) -> Option<&str> {
    let node = reply.backtrace_graph.nodes.get(reply.backtrace?)?;
    let file_index = node.file?;
    reply
        .backtrace_graph
        .files
        .get(file_index)
        .map(String::as_str)
}

/// Whether this target has no build artifact and no other target depends on
/// it — the shape of a developer-convenience target (a named `add_custom_target`
/// build step: docs, formatting, a dashboard step), as opposed to something
/// load-bearing in the build graph. Both conditions matter: a UTILITY target
/// that produces a consumed file (has artifacts) or that something links/depends
/// on is real work and must not be swept up as convenience.
fn is_inert_target(
    reply: &TargetReply,
    dependents_of: &HashMap<&str, Vec<&str>>,
    id: &str,
) -> bool {
    reply.artifacts.is_empty() && !dependents_of.contains_key(id)
}

/// Whether this target was injected by an `include()` of a CMake-provided
/// module rather than authored by the project — proven by its defining
/// command living under `CMAKE_ROOT` (the CMake installation tree). This is
/// provenance, not a name match: it catches CTest's dashboard targets and a
/// Doxygen module's `doc` target alike, and never catches a target the
/// project wrote itself, whose defining command is in the project's own
/// `CMakeLists.txt`/`.cmake`. A `None` cmake_root (malformed cache) makes
/// this conservatively `false` — escalate rather than silently drop.
fn is_cmake_provided(reply: &TargetReply, cmake_root: Option<&str>) -> bool {
    let Some(cmake_root) = cmake_root else {
        return false;
    };
    defining_command_file(reply).is_some_and(|file| Path::new(file).starts_with(cmake_root))
}

/// Whether an include's backtrace resolves to a `target_link_libraries`
/// call, i.e. whether a dependency pulled it in rather than the target
/// declaring it — see `own_include_dirs` for why that is the distinguishing
/// signal.
///
/// The answer takes three hops through the reply's `backtraceGraph`, each
/// of which can come back empty: an include may carry no backtrace, a node
/// index may not resolve, and a node may name no command (the graph's root
/// node has `command: null`). Every one of those falls through to `false`,
/// i.e. "the target's own" — deliberately the safe direction. Guessing
/// "own" for something inherited emits a redundant `includes` entry, which
/// Bazel would have supplied transitively anyway; guessing "inherited" for
/// something the target actually declared drops the only `-I` path it has,
/// and it fails to compile with nothing pointing at the cause.
fn is_inherited_via_link_libraries(backtrace: Option<usize>, graph: &BacktraceGraph) -> bool {
    let Some(node_index) = backtrace else {
        return false;
    };
    let Some(node) = graph.nodes.get(node_index) else {
        return false;
    };
    let Some(command_index) = node.command else {
        return false;
    };
    graph
        .commands
        .get(command_index)
        .is_some_and(|c| c == "target_link_libraries")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_backtrace_graph() -> BacktraceGraph {
        BacktraceGraph {
            commands: Vec::new(),
            files: Vec::new(),
            nodes: Vec::new(),
        }
    }

    // Mirrors the shape actually observed from CMake's File API: a
    // target's own target_include_directories()/FILE_SET BASE_DIRS
    // include traces to some command other than target_link_libraries; an
    // include inherited from a dependency traces to the
    // target_link_libraries call that pulled it in.
    fn backtrace_graph_with_commands(
        commands: Vec<&str>,
        node_commands: Vec<Option<usize>>,
    ) -> BacktraceGraph {
        BacktraceGraph {
            commands: commands.into_iter().map(str::to_string).collect(),
            files: Vec::new(),
            nodes: node_commands
                .into_iter()
                .map(|command| BacktraceNode {
                    command,
                    file: None,
                })
                .collect(),
        }
    }

    #[test]
    fn is_inherited_via_link_libraries_true_when_backtrace_traces_to_it() {
        let graph = backtrace_graph_with_commands(
            vec!["add_executable", "target_link_libraries"],
            vec![None, Some(0), Some(1)],
        );
        assert!(is_inherited_via_link_libraries(Some(2), &graph));
    }

    #[test]
    fn is_inherited_via_link_libraries_false_for_own_include_directories() {
        let graph = backtrace_graph_with_commands(
            vec!["add_library", "target_include_directories"],
            vec![None, Some(0), Some(1)],
        );
        assert!(!is_inherited_via_link_libraries(Some(2), &graph));
    }

    #[test]
    fn is_inherited_via_link_libraries_false_when_no_backtrace() {
        assert!(!is_inherited_via_link_libraries(
            None,
            &empty_backtrace_graph()
        ));
    }

    #[test]
    fn own_include_dirs_excludes_inherited_and_dedupes() {
        let reply = TargetReply {
            name: "hello".to_string(),
            cmake_type: "EXECUTABLE".to_string(),
            sources: vec![],
            file_sets: vec![],
            dependencies: vec![],
            artifacts: vec![],
            compile_groups: vec![CompileGroup {
                includes: vec![
                    // Own: target_include_directories (command index 1).
                    CompileGroupInclude {
                        path: "/proj/include".to_string(),
                        backtrace: Some(1),
                    },
                    // Duplicate of the above — should be deduped.
                    CompileGroupInclude {
                        path: "/proj/include".to_string(),
                        backtrace: Some(1),
                    },
                    // Inherited via target_link_libraries (command index 2).
                    CompileGroupInclude {
                        path: "/proj/other_lib_include".to_string(),
                        backtrace: Some(2),
                    },
                ],
                defines: vec![],
            }],
            backtrace: None,
            backtrace_graph: backtrace_graph_with_commands(
                vec!["target_include_directories", "target_link_libraries"],
                vec![None, Some(0), Some(1), Some(1)],
            ),
        };

        // Absolute at this stage; rebase_to_module_root makes it
        // module-relative once the module root is known.
        assert_eq!(own_include_dirs(&reply), vec!["/proj/include".to_string()]);
    }

    // Layer A collects every effective define, deduped across groups, in
    // first-seen order — and, unlike own_include_dirs, does NOT drop
    // inherited (PUBLIC-propagated) ones: they belong in this target's
    // local_defines because local_defines don't propagate. A NAME=VALUE
    // define must survive verbatim. See target_defines and
    // docs/lore/cmake-file-api-compile-definitions-shape.md.
    #[test]
    fn target_defines_collects_deduped_in_order_including_inherited() {
        let reply = TargetReply {
            name: "app".to_string(),
            cmake_type: "EXECUTABLE".to_string(),
            sources: vec![],
            file_sets: vec![],
            dependencies: vec![],
            artifacts: vec![],
            compile_groups: vec![
                CompileGroup {
                    includes: vec![],
                    defines: vec![
                        CompileGroupDefine {
                            define: "OWN_DEF".to_string(),
                        },
                        // Inherited from a dependency's PUBLIC define — must
                        // be KEPT (contrast own_include_dirs), because
                        // local_defines are non-propagating.
                        CompileGroupDefine {
                            define: "PUB_FROM_DEP=1".to_string(),
                        },
                    ],
                },
                CompileGroup {
                    includes: vec![],
                    // Duplicate across groups — deduped.
                    defines: vec![CompileGroupDefine {
                        define: "OWN_DEF".to_string(),
                    }],
                },
            ],
            backtrace: None,
            backtrace_graph: empty_backtrace_graph(),
        };

        assert_eq!(
            target_defines(&reply),
            vec!["OWN_DEF".to_string(), "PUB_FROM_DEP=1".to_string()],
            "target_defines must collect every effective define (including \
             inherited PUBLIC ones), deduped, preserving NAME=VALUE verbatim"
        );
    }

    // A UTILITY target (add_custom_target) whose defining command sits in
    // `defining_file`, with the given artifacts. `defining_file` becomes the
    // one file in the backtrace graph, and the target's backtrace points at
    // a node in it whose command is add_custom_target — the exact shape the
    // File API produces (verified against a real reply; see
    // docs/lore/cmake-include-ctest-injects-utility-targets.md).
    fn utility_reply(name: &str, defining_file: &str, artifacts: Vec<&str>) -> TargetReply {
        TargetReply {
            name: name.to_string(),
            cmake_type: "UTILITY".to_string(),
            sources: vec![],
            file_sets: vec![],
            dependencies: vec![],
            artifacts: artifacts
                .into_iter()
                .map(|p| TargetArtifact {
                    path: p.to_string(),
                })
                .collect(),
            compile_groups: vec![],
            backtrace: Some(1),
            backtrace_graph: BacktraceGraph {
                commands: vec!["add_custom_target".to_string()],
                files: vec![defining_file.to_string()],
                nodes: vec![
                    BacktraceNode {
                        command: None,
                        file: Some(0),
                    },
                    BacktraceNode {
                        command: Some(0),
                        file: Some(0),
                    },
                ],
            },
        }
    }

    #[test]
    fn defining_command_file_reads_the_targets_own_backtrace_file() {
        let reply = utility_reply("doc", "CMakeLists.txt", vec![]);
        assert_eq!(defining_command_file(&reply), Some("CMakeLists.txt"));
        let injected = utility_reply(
            "Continuous",
            "/usr/share/cmake-3.28/Modules/CTestTargets.cmake",
            vec![],
        );
        assert_eq!(
            defining_command_file(&injected),
            Some("/usr/share/cmake-3.28/Modules/CTestTargets.cmake")
        );
    }

    // Provenance, not a name match: a target defined under CMAKE_ROOT is
    // CMake-injected (CTest, Doxygen, ...) regardless of its name, and one
    // defined in the project's own files never is, regardless of its name.
    #[test]
    fn is_cmake_provided_keys_on_the_defining_files_location() {
        let root = Some("/usr/share/cmake-3.28");
        let injected = utility_reply(
            "Continuous",
            "/usr/share/cmake-3.28/Modules/CTestTargets.cmake",
            vec![],
        );
        assert!(
            is_cmake_provided(&injected, root),
            "a target defined under CMAKE_ROOT must be recognized as CMake-provided"
        );

        // Same name, but authored in the project — must NOT be caught.
        let authored = utility_reply("Continuous", "CMakeLists.txt", vec![]);
        assert!(
            !is_cmake_provided(&authored, root),
            "a project-authored target must never be treated as CMake-provided, \
             even if it shares a name with a CTest dashboard target"
        );

        // Missing CMAKE_ROOT (malformed cache) errs toward NOT dropping.
        assert!(
            !is_cmake_provided(&injected, None),
            "without CMAKE_ROOT the provenance of a target can't be proven, so it \
             must not be treated as CMake-provided"
        );
    }

    // Inertness is BOTH conditions: no artifact AND no dependent. A UTILITY
    // target that produces a consumed file, or that something depends on, is
    // load-bearing and must fall through to a real escalation rather than be
    // swept up as convenience/injected.
    #[test]
    fn is_inert_target_requires_no_artifacts_and_no_dependents() {
        let inert = utility_reply("doc", "CMakeLists.txt", vec![]);
        let mut no_dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        assert!(is_inert_target(&inert, &no_dependents, "doc::@x"));

        // Has an artifact → not inert.
        let with_artifact = utility_reply("gen", "CMakeLists.txt", vec!["gen.h"]);
        assert!(
            !is_inert_target(&with_artifact, &no_dependents, "gen::@x"),
            "a UTILITY target with a declared artifact is load-bearing"
        );

        // Has a dependent → not inert.
        no_dependents.insert("doc::@x", vec!["app::@y"]);
        assert!(
            !is_inert_target(&inert, &no_dependents, "doc::@x"),
            "a UTILITY target something depends on is load-bearing"
        );
    }

    fn public_file_set() -> Vec<TargetFileSet> {
        vec![TargetFileSet {
            fileset_type: "HEADERS".to_string(),
            visibility: "PUBLIC".to_string(),
        }]
    }

    fn library_reply(sources: Vec<TargetSource>, file_sets: Vec<TargetFileSet>) -> TargetReply {
        TargetReply {
            name: "greet".to_string(),
            cmake_type: "STATIC_LIBRARY".to_string(),
            sources,
            file_sets,
            dependencies: vec![],
            artifacts: vec![TargetArtifact {
                path: "libgreet.a".to_string(),
            }],
            compile_groups: vec![],
            backtrace: None,
            backtrace_graph: empty_backtrace_graph(),
        }
    }

    #[test]
    fn to_target_classifies_file_set_header_as_public() {
        let reply = library_reply(
            vec![
                TargetSource {
                    path: "src/greet.cpp".to_string(),
                    file_set_index: None,
                    is_generated: false,
                },
                TargetSource {
                    path: "include/greet.hpp".to_string(),
                    file_set_index: Some(0),
                    is_generated: false,
                },
            ],
            public_file_set(),
        );

        let (target, needs_attention) = to_target(
            &reply,
            TargetKind::Library,
            &HashMap::new(),
            true,
            &HashSet::new(),
        );

        assert_eq!(target.sources, vec!["src/greet.cpp".to_string()]);
        assert_eq!(target.public_headers, vec!["include/greet.hpp".to_string()]);
        assert!(
            needs_attention.is_empty(),
            "a properly file-set-declared public header should not need attention"
        );
    }

    #[test]
    fn to_target_flags_needs_attention_for_plain_header_with_consumer() {
        let reply = library_reply(
            vec![
                TargetSource {
                    path: "src/greet.cpp".to_string(),
                    file_set_index: None,
                    is_generated: false,
                },
                TargetSource {
                    path: "src/greet.hpp".to_string(),
                    file_set_index: None,
                    is_generated: false,
                },
            ],
            vec![],
        );

        let (target, needs_attention) = to_target(
            &reply,
            TargetKind::Library,
            &HashMap::new(),
            true,
            &HashSet::new(),
        );

        // The plain header stays in srcs, NOT silently promoted to hdrs.
        assert_eq!(
            target.sources,
            vec!["src/greet.cpp".to_string(), "src/greet.hpp".to_string()]
        );
        assert!(target.public_headers.is_empty());

        assert_eq!(needs_attention.len(), 1);
        assert!(needs_attention[0].title.contains("greet"));
    }

    // The mirror of the test above: the SAME plain header (no FILE_SET, on a
    // depended-on library) is classified public and does NOT escalate once
    // the project declares it via install(FILES ... TYPE INCLUDE). This is
    // the whole point of reading install rules — the two tests differ only
    // in whether the install-header set names the header.
    #[test]
    fn to_target_classifies_install_declared_header_as_public() {
        let reply = library_reply(
            vec![
                TargetSource {
                    path: "src/greet.cpp".to_string(),
                    file_set_index: None,
                    is_generated: false,
                },
                TargetSource {
                    path: "greet.h".to_string(),
                    file_set_index: None,
                    is_generated: false,
                },
            ],
            vec![],
        );
        let installed: HashSet<String> = ["greet.h".to_string()].into_iter().collect();

        let (target, needs_attention) = to_target(
            &reply,
            TargetKind::Library,
            &HashMap::new(),
            true,
            &installed,
        );

        assert_eq!(
            target.public_headers,
            vec!["greet.h".to_string()],
            "a header the project installs to an include destination is public (hdrs)"
        );
        assert_eq!(target.sources, vec!["src/greet.cpp".to_string()]);
        assert!(
            needs_attention.is_empty(),
            "an install-declared public header resolves the ambiguity, so no escalation: {:?}",
            needs_attention.first().map(|n| &n.title)
        );
    }

    #[test]
    fn is_include_destination_matches_include_dirs_only() {
        assert!(is_include_destination("include"));
        assert!(is_include_destination("include/tinyxml2"));
        // The absolute form CMAKE_INSTALL_FULL_INCLUDEDIR expands to — what
        // json-c installs its public headers to (bzl-fxa.14).
        assert!(is_include_destination("/usr/local/include/json-c"));
        assert!(is_include_destination("/usr/include"));
        assert!(is_include_destination("/opt/thing/include/sub"));
        // Not a header destination — these are where a target install, a
        // pkgconfig file, and cmake package files land.
        assert!(!is_include_destination("lib"));
        assert!(!is_include_destination("lib/pkgconfig"));
        assert!(!is_include_destination("lib/cmake/tinyxml2"));
        assert!(!is_include_destination("share/doc"));
        assert!(!is_include_destination("/usr/local/lib/cmake/json-c"));
        // A build-private include tree nested under lib/lib64/share is not the
        // public install location — only a real include prefix counts.
        assert!(!is_include_destination("lib/include"));
        assert!(!is_include_destination("/usr/local/lib/include"));
        assert!(!is_include_destination("share/include"));
    }

    #[test]
    fn installed_public_headers_collects_only_file_installs_to_include() {
        let dir = DirectoryReply {
            installers: vec![
                // A target install (the .a) — not a header.
                Installer {
                    installer_type: "target".to_string(),
                    destination: Some("lib".to_string()),
                    paths: vec!["libgreet.a".to_string()],
                },
                // A file install to include — the public header. KEEP.
                Installer {
                    installer_type: "file".to_string(),
                    destination: Some("include".to_string()),
                    paths: vec!["greet.h".to_string()],
                },
                // A file install elsewhere (pkgconfig) — not a header.
                Installer {
                    installer_type: "file".to_string(),
                    destination: Some("lib/pkgconfig".to_string()),
                    paths: vec!["greet.pc".to_string()],
                },
            ],
        };
        let headers = installed_public_headers(std::slice::from_ref(&dir));
        assert_eq!(
            headers,
            ["greet.h".to_string()].into_iter().collect(),
            "only files installed to an include destination are public headers"
        );
    }

    // Helper: an empty library Target with the given name and include dirs.
    fn library_target(name: &str, includes: Vec<String>) -> Target {
        Target {
            name: name.to_string(),
            kind: TargetKind::Library,
            sources: Vec::new(),
            public_headers: Vec::new(),
            dependencies: Vec::new(),
            includes,
            local_defines: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    // bzl-fxa.18: json-c's `test1` lists one .c and reaches parse_flags.h
    // purely through its own include dir. CMake finds it on disk; Bazel stages
    // only declared inputs, so it must become a source.
    #[test]
    fn headers_on_a_targets_include_dirs_are_added_to_its_sources() {
        let dir =
            std::env::temp_dir().join(format!("bzlf_incdir_{}_{}", std::process::id(), line!()));
        let tests_dir = dir.join("tests");
        fs::create_dir_all(&tests_dir).unwrap();
        fs::write(tests_dir.join("test1.c"), b"int main(void){return 0;}\n").unwrap();
        fs::write(tests_dir.join("parse_flags.h"), b"#pragma once\n").unwrap();
        // A .c in the same directory: it is another target's source, and a
        // header injection must not sweep it in.
        fs::write(tests_dir.join("parse_flags.c"), b"int f(void){return 0;}\n").unwrap();
        let src = dir.to_string_lossy().into_owned();

        let mut targets = vec![{
            let mut t = library_target("test1", vec![tests_dir.to_string_lossy().into_owned()]);
            t.kind = TargetKind::Executable;
            t.sources.push("tests/test1.c".to_string());
            t
        }];

        inject_headers_on_include_dirs(&mut targets, Path::new(&src));

        assert_eq!(
            targets[0].sources,
            vec![
                "tests/test1.c".to_string(),
                "tests/parse_flags.h".to_string()
            ],
            "every header on the include path is an input (CMake's own semantic), but a \
             .c file there belongs to some other target and must not be swept in"
        );
    }

    // The dedup must be PER-TARGET. Asking "does any target list this header?"
    // suppresses it exactly where it's missing — json-c's test1Formatted lists
    // parse_flags.h while its sibling test1, same include dir, does not.
    #[test]
    fn a_header_enumerated_on_a_sibling_target_is_still_injected_here() {
        let dir =
            std::env::temp_dir().join(format!("bzlf_incdir_{}_{}", std::process::id(), line!()));
        let tests_dir = dir.join("tests");
        fs::create_dir_all(&tests_dir).unwrap();
        fs::write(tests_dir.join("test1.c"), b"int main(void){return 0;}\n").unwrap();
        fs::write(tests_dir.join("parse_flags.h"), b"#pragma once\n").unwrap();
        let src = dir.to_string_lossy().into_owned();
        let include = tests_dir.to_string_lossy().into_owned();

        let mut targets = vec![
            {
                let mut t = library_target("test1", vec![include.clone()]);
                t.kind = TargetKind::Executable;
                t.sources.push("tests/test1.c".to_string());
                t
            },
            {
                let mut t = library_target("test1Formatted", vec![include]);
                t.kind = TargetKind::Executable;
                t.sources.push("tests/test1.c".to_string());
                // CMake enumerated it here, and only here.
                t.sources.push("tests/parse_flags.h".to_string());
                t
            },
        ];

        inject_headers_on_include_dirs(&mut targets, Path::new(&src));

        let by_name = |n: &str| targets.iter().find(|t| t.name == n).unwrap();
        assert!(
            by_name("test1")
                .sources
                .contains(&"tests/parse_flags.h".to_string()),
            "the sibling listing it must not suppress it here:\n{:?}",
            by_name("test1").sources
        );
        assert_eq!(
            by_name("test1Formatted")
                .sources
                .iter()
                .filter(|s| *s == "tests/parse_flags.h")
                .count(),
            1,
            "and the target that DID enumerate it must not list it twice:\n{:?}",
            by_name("test1Formatted").sources
        );
    }

    #[test]
    fn an_include_dir_outside_the_source_tree_contributes_nothing() {
        let dir =
            std::env::temp_dir().join(format!("bzlf_incdir_{}_{}", std::process::id(), line!()));
        let src_root = dir.join("proj");
        let build_dir = dir.join("build");
        fs::create_dir_all(&src_root).unwrap();
        fs::create_dir_all(&build_dir).unwrap();
        fs::write(src_root.join("app.c"), b"int main(void){return 0;}\n").unwrap();
        // A generated header in the build tree: reproduced by the
        // config_header machinery, never copied.
        fs::write(build_dir.join("config.h"), b"#pragma once\n").unwrap();
        let src = src_root.to_string_lossy().into_owned();

        let mut targets = vec![{
            let mut t = library_target("app", vec![build_dir.to_string_lossy().into_owned()]);
            t.kind = TargetKind::Executable;
            t.sources.push("app.c".to_string());
            t
        }];

        inject_headers_on_include_dirs(&mut targets, Path::new(&src));

        assert_eq!(
            targets[0].sources,
            vec!["app.c".to_string()],
            "a build-dir include path must contribute nothing — its headers are \
             reproduced by config_header, and the module may not carry them"
        );
    }

    #[test]
    fn inject_unenumerated_installed_headers_adds_them_to_libraries_on_the_include_path() {
        // bzl-fxa.10: a public header the project install()s but that no target
        // enumerated (json-c's json_pointer.h — dropped from the library's
        // header list by a CMakeLists ordering bug, yet #included by its .c and
        // install()d as public). It must be injected as a public header on the
        // libraries whose include path it sits on, so it's copied and reachable.
        let dir =
            std::env::temp_dir().join(format!("bzlf_inject_{}_{}", std::process::id(), line!()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("json_pointer.h"), b"#pragma once\n").unwrap();
        fs::write(dir.join("json_object.h"), b"#pragma once\n").unwrap();
        let src = dir.to_string_lossy().into_owned();

        // Two libraries on the same include dir (shared + static, as json-c
        // has), plus an executable that must NOT receive the header, plus a
        // library whose include dir does not contain it.
        let mut targets = vec![
            {
                let mut t = library_target("json-c", vec![src.clone()]);
                // json_object.h is enumerated AND install-declared -> already
                // public via to_target; must not be injected a second time.
                t.public_headers.push("json_object.h".to_string());
                t.sources.push("json_object.c".to_string());
                t
            },
            library_target("json-c-static", vec![src.clone()]),
            {
                let mut t = library_target("elsewhere", vec!["/other/include".to_string()]);
                t.kind = TargetKind::Library;
                t
            },
            {
                let mut t = library_target("app", vec![src.clone()]);
                t.kind = TargetKind::Executable;
                t
            },
        ];

        let installed: HashSet<String> =
            ["json_pointer.h".to_string(), "json_object.h".to_string()]
                .into_iter()
                .collect();

        inject_unenumerated_installed_headers(&mut targets, &installed, Path::new(&src));

        let by_name = |n: &str| targets.iter().find(|t| t.name == n).unwrap();
        assert_eq!(
            by_name("json-c").public_headers,
            vec!["json_object.h".to_string(), "json_pointer.h".to_string()],
            "the unenumerated install-declared header is appended; the enumerated one is not duplicated"
        );
        assert_eq!(
            by_name("json-c-static").public_headers,
            vec!["json_pointer.h".to_string()],
            "the sibling library on the same include path also gets it"
        );
        assert!(
            by_name("elsewhere").public_headers.is_empty(),
            "a library whose include dirs don't contain the header is not given it"
        );
        assert!(
            by_name("app").public_headers.is_empty(),
            "an executable is never given injected headers (cc_binary has no hdrs)"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inject_unenumerated_installed_headers_skips_out_of_tree_and_missing() {
        // A generated header installed by absolute build-dir path (json-c's
        // json.h) resolves outside source_dir and must be skipped — it's
        // reproduced by the config_header machinery, not copied. A declared
        // header that doesn't exist on disk is skipped too (copying it would
        // hard-fail).
        let dir = std::env::temp_dir().join(format!(
            "bzlf_inject_skip_{}_{}",
            std::process::id(),
            line!()
        ));
        fs::create_dir_all(&dir).unwrap();
        let src = dir.to_string_lossy().into_owned();
        let mut targets = vec![library_target("lib", vec![src.clone()])];

        let installed: HashSet<String> = [
            "/some/build/json.h".to_string(), // absolute, outside source_dir
            "phantom.h".to_string(),          // under source_dir but not on disk
        ]
        .into_iter()
        .collect();

        inject_unenumerated_installed_headers(&mut targets, &installed, Path::new(&src));

        assert!(
            targets[0].public_headers.is_empty(),
            "neither an out-of-tree generated header nor a nonexistent one is injected"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn to_target_no_needs_attention_when_nothing_depends_on_it() {
        let reply = library_reply(
            vec![
                TargetSource {
                    path: "src/greet.cpp".to_string(),
                    file_set_index: None,
                    is_generated: false,
                },
                TargetSource {
                    path: "src/greet.hpp".to_string(),
                    file_set_index: None,
                    is_generated: false,
                },
            ],
            vec![],
        );

        // is_depended_on = false: nothing in the project links against
        // this library, so there's no consumer that could need a header
        // it isn't exposing — not worth flagging.
        let (_target, needs_attention) = to_target(
            &reply,
            TargetKind::Library,
            &HashMap::new(),
            false,
            &HashSet::new(),
        );

        assert!(needs_attention.is_empty());
    }

    #[test]
    fn to_target_no_needs_attention_when_no_header_like_sources() {
        // A library with only .cpp sources (no headers at all) and a
        // consumer: nothing to classify, so no gap to flag.
        let reply = library_reply(
            vec![TargetSource {
                path: "src/greet.cpp".to_string(),
                file_set_index: None,
                is_generated: false,
            }],
            vec![],
        );

        let (_target, needs_attention) = to_target(
            &reply,
            TargetKind::Library,
            &HashMap::new(),
            true,
            &HashSet::new(),
        );

        assert!(needs_attention.is_empty());
    }

    #[test]
    fn to_target_resolves_dependencies_by_id() {
        let translated_names = HashMap::from([("greet::@abc123", "greet")]);

        let reply = TargetReply {
            name: "hello".to_string(),
            cmake_type: "EXECUTABLE".to_string(),
            sources: vec![TargetSource {
                path: "src/main.cpp".to_string(),
                file_set_index: None,
                is_generated: false,
            }],
            file_sets: vec![],
            dependencies: vec![TargetDependency {
                id: "greet::@abc123".to_string(),
            }],
            artifacts: vec![TargetArtifact {
                path: "hello".to_string(),
            }],
            compile_groups: vec![],
            backtrace: None,
            backtrace_graph: empty_backtrace_graph(),
        };

        let (target, _) = to_target(
            &reply,
            TargetKind::Executable,
            &translated_names,
            false,
            &HashSet::new(),
        );
        assert_eq!(target.dependencies, vec!["greet".to_string()]);
    }

    // An edge to a target that never got a Bazel rule is dropped rather
    // than emitted as a dangling label — see `translated_names` in
    // `read_codemodel_reply` for why, and
    // `unsupported_target_needs_attention` for where the lost edge is
    // recorded instead.
    #[test]
    fn to_target_drops_dependencies_on_untranslated_targets() {
        let reply = TargetReply {
            name: "app".to_string(),
            cmake_type: "EXECUTABLE".to_string(),
            sources: vec![],
            file_sets: vec![],
            dependencies: vec![
                TargetDependency {
                    id: "greet::@abc123".to_string(),
                },
                TargetDependency {
                    id: "gen_docs::@def456".to_string(),
                },
            ],
            artifacts: vec![],
            compile_groups: vec![],
            backtrace: None,
            backtrace_graph: empty_backtrace_graph(),
        };

        // "gen_docs" is deliberately absent: it is the UTILITY target that
        // was escalated rather than translated.
        let translated_names = HashMap::from([("greet::@abc123", "greet")]);

        let (target, _) = to_target(
            &reply,
            TargetKind::Executable,
            &translated_names,
            false,
            &HashSet::new(),
        );
        assert_eq!(target.dependencies, vec!["greet".to_string()]);
    }

    // CMake splices an OBJECT_LIBRARY's output into its consumers as a
    // generated source, reported as an absolute path into the build
    // directory. Emitting that verbatim would put the build machine's
    // filesystem layout into the generated BUILD.bazel.
    #[test]
    fn to_target_excludes_generated_sources_and_escalates() {
        let reply = TargetReply {
            name: "app".to_string(),
            cmake_type: "EXECUTABLE".to_string(),
            sources: vec![
                TargetSource {
                    path: "src/main.cpp".to_string(),
                    file_set_index: None,
                    is_generated: false,
                },
                TargetSource {
                    path: "/abs/build/CMakeFiles/obj.dir/src/lib.cpp.o".to_string(),
                    file_set_index: None,
                    is_generated: true,
                },
            ],
            file_sets: vec![],
            dependencies: vec![],
            artifacts: vec![TargetArtifact {
                path: "app".to_string(),
            }],
            compile_groups: vec![],
            backtrace: None,
            backtrace_graph: empty_backtrace_graph(),
        };

        let (target, needs_attention) = to_target(
            &reply,
            TargetKind::Executable,
            &HashMap::new(),
            false,
            &HashSet::new(),
        );

        assert_eq!(
            target.sources,
            vec!["src/main.cpp".to_string()],
            "a generated source must never reach srcs"
        );
        assert_eq!(needs_attention.len(), 1);
        assert!(needs_attention[0].title.contains("app"));
        assert!(
            needs_attention[0]
                .gap
                .contains("/abs/build/CMakeFiles/obj.dir/src/lib.cpp.o"),
            "the escalation must name the file that was dropped:\n{}",
            needs_attention[0].gap
        );
    }

    // Every add_custom_command() output arrives with a phantom "<output>.rule"
    // sibling in the File API reply (Ninja/Make build-graph bookkeeping,
    // names no file on disk) — see
    // docs/lore/cmake-file-api-generated-source-shape.md. It must not read
    // to an agent as a second missing generated source.
    #[test]
    fn to_target_filters_phantom_rule_sibling_out_of_generated_sources() {
        let reply = TargetReply {
            name: "app".to_string(),
            cmake_type: "EXECUTABLE".to_string(),
            sources: vec![
                TargetSource {
                    path: "src/main.cpp".to_string(),
                    file_set_index: None,
                    is_generated: false,
                },
                TargetSource {
                    path: "/abs/build/gen.cpp".to_string(),
                    file_set_index: None,
                    is_generated: true,
                },
                TargetSource {
                    path: "/abs/build/gen.cpp.rule".to_string(),
                    file_set_index: None,
                    is_generated: true,
                },
            ],
            file_sets: vec![],
            dependencies: vec![],
            artifacts: vec![TargetArtifact {
                path: "app".to_string(),
            }],
            compile_groups: vec![],
            backtrace: None,
            backtrace_graph: empty_backtrace_graph(),
        };

        let (_target, needs_attention) = to_target(
            &reply,
            TargetKind::Executable,
            &HashMap::new(),
            false,
            &HashSet::new(),
        );

        assert_eq!(needs_attention.len(), 1);
        assert!(
            needs_attention[0].gap.contains("/abs/build/gen.cpp")
                && !needs_attention[0].gap.contains("gen.cpp.rule"),
            "escalation must name the real generated file but not its \
             phantom .rule sibling:\n{}",
            needs_attention[0].gap
        );
    }

    #[test]
    fn to_target_no_generated_source_escalation_for_ordinary_sources() {
        let reply = library_reply(
            vec![TargetSource {
                path: "src/greet.cpp".to_string(),
                file_set_index: None,
                is_generated: false,
            }],
            vec![],
        );

        let (_target, needs_attention) = to_target(
            &reply,
            TargetKind::Library,
            &HashMap::new(),
            false,
            &HashSet::new(),
        );

        assert!(needs_attention.is_empty());
    }

    fn target_with(sources: Vec<&str>, includes: Vec<&str>) -> Target {
        Target {
            name: "app".to_string(),
            kind: TargetKind::Executable,
            sources: sources.into_iter().map(str::to_string).collect(),
            public_headers: vec![],
            dependencies: vec![],
            includes: includes.into_iter().map(str::to_string).collect(),
            local_defines: vec![],
            artifacts: vec![],
        }
    }

    // The common case: nothing reaches outside the project, so the module
    // root is the project directory and paths are unchanged.
    #[test]
    fn rebase_keeps_module_at_project_when_nothing_reaches_outside() {
        let mut targets = vec![target_with(vec!["src/main.cpp"], vec!["/deliv/proj/inc"])];

        let (module_root, escalations) = rebase_to_module_root(
            &mut targets,
            Path::new("/deliv/proj"),
            Path::new("/deliv/proj"),
            Path::new("/deliv/proj/_build"),
            &HashSet::new(),
        );

        assert_eq!(module_root, PathBuf::from("/deliv/proj"));
        assert_eq!(targets[0].sources, vec!["src/main.cpp".to_string()]);
        assert_eq!(targets[0].includes, vec!["inc".to_string()]);
        assert!(escalations.is_empty());
    }

    // A sibling directory that ships with the project: the module widens to
    // cover it, every path stays relative, and nothing is escalated —
    // the file is reproducible, so there is no gap to report.
    #[test]
    fn rebase_widens_module_root_to_cover_shipped_sibling_sources() {
        let mut targets = vec![target_with(
            vec!["src/main.cpp", "/deliv/shared/helper.cpp"],
            vec![],
        )];

        let (module_root, escalations) = rebase_to_module_root(
            &mut targets,
            Path::new("/deliv/proj"),
            Path::new("/deliv"),
            Path::new("/deliv/proj/_build"),
            &HashSet::new(),
        );

        assert_eq!(module_root, PathBuf::from("/deliv"));
        assert_eq!(
            targets[0].sources,
            vec![
                "proj/src/main.cpp".to_string(),
                "shared/helper.cpp".to_string()
            ]
        );
        assert!(
            escalations.is_empty(),
            "a file that ships with the project is not a gap: {:?}",
            escalations.iter().map(|e| &e.title).collect::<Vec<_>>()
        );
    }

    // The cap doing its job: a file outside the deliverable must not drag
    // the module root out with it, however far up the tree it sits.
    #[test]
    fn rebase_refuses_to_widen_past_the_deliverable_root() {
        let mut targets = vec![target_with(
            vec!["src/main.cpp", "/elsewhere/vendor/blob.cpp"],
            vec![],
        )];

        let (module_root, escalations) = rebase_to_module_root(
            &mut targets,
            Path::new("/deliv/proj"),
            Path::new("/deliv/proj"),
            Path::new("/deliv/proj/_build"),
            &HashSet::new(),
        );

        assert_eq!(module_root, PathBuf::from("/deliv/proj"));
        assert_eq!(targets[0].sources, vec!["src/main.cpp".to_string()]);
        assert_eq!(escalations.len(), 1);
        assert!(
            escalations[0].gap.contains("/elsewhere/vendor/blob.cpp"),
            "{}",
            escalations[0].gap
        );
    }

    // Both directions of the unreachable-source split: a header under the
    // build directory (a configure_file output — json-c's json_config.h) gets
    // the build-generated-header escalation, while a source under an
    // unrelated directory gets the sources-outside-deliverable one. Same
    // target, one of each, so the routing itself is under test — not just
    // that some escalation fired.
    #[test]
    fn rebase_routes_build_dir_headers_and_outside_sources_to_different_escalations() {
        // The build dir is OUTSIDE the module root (as it is in practice — the
        // translator configures into a scratch dir separate from the sources),
        // so a header there is genuinely unreachable AND recognizable as a
        // build output.
        let mut targets = vec![target_with(
            vec![
                "src/main.cpp",
                "/build-out/json_config.h",
                "/elsewhere/vendor/blob.cpp",
            ],
            vec![],
        )];

        let (_root, escalations) = rebase_to_module_root(
            &mut targets,
            Path::new("/deliv/proj"),
            Path::new("/deliv/proj"),
            Path::new("/build-out"),
            &HashSet::new(),
        );

        let generated = escalations
            .iter()
            .find(|e| e.title.contains("build-generated headers"))
            .expect("a build-dir header must get the build-generated-header escalation");
        assert!(
            generated.gap.contains("json_config.h") && !generated.gap.contains("blob.cpp"),
            "the build-generated escalation names only the build-dir header:\n{}",
            generated.gap
        );

        let outside = escalations
            .iter()
            .find(|e| e.title.contains("sources the module cannot reach"))
            .expect("a non-build-dir outside source must get the sources-outside escalation");
        assert!(
            outside.gap.contains("blob.cpp") && !outside.gap.contains("json_config.h"),
            "the sources-outside escalation names only the outside-deliverable source:\n{}",
            outside.gap
        );
    }

    // The reconciliation: a build-dir source that a config_header rule
    // reproduces (its name is in config_header_outputs) is dropped from
    // sources and NOT escalated — it's supplied by the config header's label.
    // Otherwise the same header is both escalated here and regenerated,
    // which is exactly what json-c exposed.
    #[test]
    fn rebase_drops_a_reproduced_config_header_source_without_escalating() {
        let mut targets = vec![target_with(
            vec!["src/main.cpp", "/build-out/config.h"],
            vec![],
        )];
        let reproduced: HashSet<String> = ["config.h".to_string()].into_iter().collect();

        let (_root, escalations) = rebase_to_module_root(
            &mut targets,
            Path::new("/deliv/proj"),
            Path::new("/deliv/proj"),
            Path::new("/build-out"),
            &reproduced,
        );

        assert_eq!(
            targets[0].sources,
            vec!["src/main.cpp".to_string()],
            "the reproduced config header is dropped from srcs (the config_header label supplies it)"
        );
        assert!(
            escalations.is_empty(),
            "a reproduced config header must not be escalated: {:?}",
            escalations.iter().map(|e| &e.title).collect::<Vec<_>>()
        );
    }

    // System include dirs land outside the module and have no `includes`
    // translation — dropping them is correct, and is not a gap.
    #[test]
    fn rebase_drops_include_dirs_outside_the_module_without_escalating() {
        let mut targets = vec![target_with(vec!["src/main.cpp"], vec!["/usr/include"])];

        let (_root, escalations) = rebase_to_module_root(
            &mut targets,
            Path::new("/deliv/proj"),
            Path::new("/deliv/proj"),
            Path::new("/deliv/proj/_build"),
            &HashSet::new(),
        );

        assert!(targets[0].includes.is_empty());
        assert!(escalations.is_empty());
    }

    #[test]
    fn target_kind_maps_supported_cmake_types() {
        assert_eq!(target_kind("EXECUTABLE"), Some(TargetKind::Executable));
        assert_eq!(target_kind("STATIC_LIBRARY"), Some(TargetKind::Library));
        assert_eq!(target_kind("SHARED_LIBRARY"), Some(TargetKind::Library));
    }

    // An unmapped type must escalate, never abort the conversion — see
    // docs/architecture/cmake-frontend.md. `INTERFACE_LIBRARY` is covered
    // defensively only: verified against CMake 3.28 + Ninja, it never
    // appears in a codemodel reply at all, so the translator cannot in
    // practice escalate one — see that doc's "known hard cases".
    #[test]
    fn target_kind_rejects_types_with_no_bazel_rule_yet() {
        for cmake_type in [
            "UTILITY",
            "OBJECT_LIBRARY",
            "MODULE_LIBRARY",
            "INTERFACE_LIBRARY",
        ] {
            assert_eq!(
                target_kind(cmake_type),
                None,
                "{cmake_type} should escalate, not translate"
            );
        }
    }

    /// A directory that deletes itself (recursively) on drop, so a failing
    /// assertion doesn't leave the reply files behind in the OS temp dir.
    /// Not `tempfile`: this is the only place in the crate that needs a
    /// scratch directory, so a real dependency (and the `Cargo.lock` regen
    /// that comes with one — see
    /// docs/runbooks/001-regenerate-translator-cargo-lock.md) isn't worth
    /// it for one call site.
    struct ScratchDir {
        path: PathBuf,
    }

    impl ScratchDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "bazelifier-test-{name}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            ScratchDir { path }
        }

        fn write(&self, filename: &str, contents: &str) {
            fs::write(self.path.join(filename), contents).unwrap();
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    // Real CMake File API output, captured verbatim from a build of
    // 002-with-library (see translator/tests/fixtures/002-with-library) —
    // NOT hand-constructed, unlike every TargetReply/CodemodelIndexReply
    // literal elsewhere in this file. Those only prove read_codemodel_reply
    // agrees with our own idea of the schema; this pins it against what
    // CMake 3.28 + the codemodel-v2/cache-v2 File API actually emit, so a
    // wrong #[serde(rename)] (e.g. "isGenerated") would show up here as a
    // wrong result instead of deserializing to a silent default. Only the
    // top-level `paths.source`/`paths.build` (what read_codemodel_reply
    // actually reads back out, via `index.paths`) were edited, from the
    // capture-time sandbox path to a made-up but structurally identical
    // absolute path — CMake always reports this field absolute, so
    // rewriting it to "." would silently stop exercising the same
    // starts_with(deliverable_root) comparison discover() runs in
    // production (absolutize() always makes deliverable_root absolute
    // before calling read_codemodel_reply).
    const CODEMODEL_JSON: &str = r#"{
  "configurations": [
    {
      "directories": [
        {
          "build": ".",
          "jsonFile": "directory-.json",
          "minimumCMakeVersion": { "string": "3.23" },
          "projectIndex": 0,
          "source": ".",
          "targetIndexes": [0, 1]
        }
      ],
      "name": "",
      "projects": [
        {
          "directoryIndexes": [0],
          "name": "with_library",
          "targetIndexes": [0, 1]
        }
      ],
      "targets": [
        {
          "directoryIndex": 0,
          "id": "greet::@6890427a1f51a3e7e1df",
          "jsonFile": "target-greet.json",
          "name": "greet",
          "projectIndex": 0
        },
        {
          "directoryIndex": 0,
          "id": "hello::@6890427a1f51a3e7e1df",
          "jsonFile": "target-hello.json",
          "name": "hello",
          "projectIndex": 0
        }
      ]
    }
  ],
  "kind": "codemodel",
  "paths": {
    "build": "/abs/002-with-library/_build",
    "source": "/abs/002-with-library"
  },
  "version": { "major": 2, "minor": 6 }
}"#;

    const TARGET_GREET_JSON: &str = r#"{
  "archive": {},
  "artifacts": [{ "path": "libgreet.a" }],
  "backtrace": 1,
  "backtraceGraph": {
    "commands": ["add_library", "target_sources"],
    "files": ["CMakeLists.txt"],
    "nodes": [
      { "file": 0 },
      { "command": 0, "file": 0, "line": 4, "parent": 0 },
      { "command": 1, "file": 0, "line": 5, "parent": 0 }
    ]
  },
  "compileGroups": [
    {
      "includes": [{ "backtrace": 2, "path": "/abs/002-with-library/include" }],
      "language": "CXX",
      "sourceIndexes": [0]
    }
  ],
  "fileSets": [
    {
      "baseDirectories": ["include"],
      "name": "public_headers",
      "type": "HEADERS",
      "visibility": "PUBLIC"
    }
  ],
  "id": "greet::@6890427a1f51a3e7e1df",
  "name": "greet",
  "nameOnDisk": "libgreet.a",
  "paths": { "build": ".", "source": "." },
  "sourceGroups": [
    { "name": "Source Files", "sourceIndexes": [0] },
    { "name": "Header Files", "sourceIndexes": [1] }
  ],
  "sources": [
    { "backtrace": 1, "compileGroupIndex": 0, "path": "src/greet.cpp", "sourceGroupIndex": 0 },
    {
      "backtrace": 2,
      "fileSetIndex": 0,
      "path": "include/greet.hpp",
      "sourceGroupIndex": 1
    }
  ],
  "type": "STATIC_LIBRARY"
}"#;

    const TARGET_HELLO_JSON: &str = r#"{
  "artifacts": [{ "path": "hello" }],
  "backtrace": 1,
  "backtraceGraph": {
    "commands": ["add_executable", "target_link_libraries"],
    "files": ["CMakeLists.txt"],
    "nodes": [
      { "file": 0 },
      { "command": 0, "file": 0, "line": 13, "parent": 0 },
      { "command": 1, "file": 0, "line": 14, "parent": 0 }
    ]
  },
  "compileGroups": [
    {
      "includes": [{ "backtrace": 2, "path": "/abs/002-with-library/include" }],
      "language": "CXX",
      "sourceIndexes": [0]
    }
  ],
  "dependencies": [{ "backtrace": 2, "id": "greet::@6890427a1f51a3e7e1df" }],
  "id": "hello::@6890427a1f51a3e7e1df",
  "link": {
    "commandFragments": [
      { "fragment": "", "role": "flags" },
      { "backtrace": 2, "fragment": "libgreet.a", "role": "libraries" }
    ],
    "language": "CXX"
  },
  "name": "hello",
  "nameOnDisk": "hello",
  "paths": { "build": ".", "source": "." },
  "sourceGroups": [{ "name": "Source Files", "sourceIndexes": [0] }],
  "sources": [
    { "backtrace": 1, "compileGroupIndex": 0, "path": "src/main.cpp", "sourceGroupIndex": 0 }
  ],
  "type": "EXECUTABLE"
}"#;

    // A real cache-v2 reply is ~83 entries; this keeps the two `read_codemodel_reply`
    // reads exercise (CMAKE_ROOT for the provenance filter) plus CMAKE_PROJECT_VERSION,
    // in the real envelope shape (CacheReply only reads name/value, ignoring the
    // properties/type each real entry also carries). CMAKE_ROOT is the real value
    // this CMake install reports; the reply directory always contains this file in
    // production because the translator writes a cache-v2 query alongside codemodel-v2.
    const CACHE_JSON: &str = r#"{
  "entries": [
    { "name": "CMAKE_PROJECT_VERSION", "type": "STATIC", "value": "" },
    { "name": "CMAKE_ROOT", "type": "INTERNAL", "value": "/usr/share/cmake-3.28" }
  ],
  "kind": "cache",
  "version": { "major": 2, "minor": 0 }
}"#;

    fn reply_dir_from_real_capture() -> ScratchDir {
        let dir = ScratchDir::new("codemodel");
        dir.write("codemodel-v2-abc123.json", CODEMODEL_JSON);
        dir.write("cache-v2-abc123.json", CACHE_JSON);
        dir.write("target-greet.json", TARGET_GREET_JSON);
        dir.write("target-hello.json", TARGET_HELLO_JSON);
        dir
    }

    #[test]
    fn read_codemodel_reply_wires_real_capture_into_a_build_graph() {
        let dir = reply_dir_from_real_capture();

        let codemodel = read_codemodel_reply(
            &dir.path,
            Path::new("/abs/002-with-library"),
            Path::new("/abs/002-with-library/_build"),
            &HashSet::new(),
        )
        .expect("real File API capture should parse and translate cleanly");

        assert_eq!(codemodel.project_name, "with_library");
        assert!(codemodel.needs_attention.is_empty());

        let names: Vec<&str> = codemodel.targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["greet", "hello"]);

        let greet = &codemodel.targets[0];
        assert_eq!(greet.kind, TargetKind::Library);
        assert_eq!(greet.sources, vec!["src/greet.cpp".to_string()]);
        // is_depended_on for greet is computed from hello's real
        // dependencies edge in the capture, not passed in by a test — this
        // is exactly the wiring to_target's own unit tests take as a given
        // parameter instead of proving.
        assert_eq!(
            greet.public_headers,
            vec!["include/greet.hpp".to_string()],
            "greet's FILE_SET PUBLIC header should be classified as public, \
             not escalated, since it's real is_depended_on is true and the \
             header IS file-set-declared"
        );

        let hello = &codemodel.targets[1];
        assert_eq!(hello.kind, TargetKind::Executable);
        assert_eq!(hello.sources, vec!["src/main.cpp".to_string()]);
        // Resolved from the opaque "greet::@..." id back to a name via
        // translated_names — the id never appears in the output.
        assert_eq!(hello.dependencies, vec!["greet".to_string()]);
    }

    #[test]
    fn read_codemodel_reply_rejects_source_dir_outside_deliverable_root() {
        let dir = reply_dir_from_real_capture();

        match read_codemodel_reply(
            &dir.path,
            Path::new("/somewhere/else"),
            Path::new("/somewhere/else/_build"),
            &HashSet::new(),
        ) {
            Err(Error::SourceDirOutsideDeliverableRoot { .. }) => {}
            other => panic!(
                "source dir (\".\", i.e. cwd) can never be inside an unrelated root, \
                 expected SourceDirOutsideDeliverableRoot, got: {}",
                match &other {
                    Ok(_) => "Ok(_)".to_string(),
                    Err(e) => e.to_string(),
                }
            ),
        }
    }

    // A codemodel with a translatable library (greet, reusing TARGET_GREET_JSON)
    // plus a project-authored inert UTILITY target (docs), so the full filtering
    // composition in read_codemodel_reply is exercised end-to-end — not just the
    // is_cmake_provided / is_inert_target predicates in isolation. The two source
    // paths stay "/abs/002-with-library" so the deliverable-root check passes.
    const CODEMODEL_WITH_PROJECT_UTILITY_JSON: &str = r#"{
  "configurations": [
    {
      "directories": [
        { "build": ".", "jsonFile": "directory-.json", "projectIndex": 0, "source": ".", "targetIndexes": [0, 1] }
      ],
      "name": "",
      "projects": [
        { "directoryIndexes": [0], "name": "with_library", "targetIndexes": [0, 1] }
      ],
      "targets": [
        { "directoryIndex": 0, "id": "greet::@6890427a1f51a3e7e1df", "jsonFile": "target-greet.json", "name": "greet", "projectIndex": 0 },
        { "directoryIndex": 0, "id": "docs::@6890427a1f51a3e7e1df", "jsonFile": "target-docs.json", "name": "docs", "projectIndex": 0 }
      ]
    }
  ],
  "kind": "codemodel",
  "paths": { "build": "/abs/002-with-library/_build", "source": "/abs/002-with-library" },
  "version": { "major": 2, "minor": 6 }
}"#;

    // A project-authored UTILITY target: defining command in CMakeLists.txt
    // (NOT under CMAKE_ROOT, so is_cmake_provided is false), no artifacts and no
    // dependents (so is_inert_target is true). The intended outcome is a single
    // AGGREGATED convenience escalation — neither a silent drop (that is only for
    // CMake-module-injected targets) nor a per-target unsupported-type item.
    const TARGET_DOCS_JSON: &str = r#"{
  "name": "docs",
  "type": "UTILITY",
  "sources": [],
  "backtrace": 1,
  "backtraceGraph": {
    "commands": ["add_custom_target"],
    "files": ["CMakeLists.txt"],
    "nodes": [
      { "file": 0 },
      { "command": 0, "file": 0, "line": 40, "parent": 0 }
    ]
  }
}"#;

    fn reply_dir_with_project_utility() -> ScratchDir {
        let dir = ScratchDir::new("codemodel_util");
        dir.write(
            "codemodel-v2-abc123.json",
            CODEMODEL_WITH_PROJECT_UTILITY_JSON,
        );
        dir.write("cache-v2-abc123.json", CACHE_JSON);
        dir.write("target-greet.json", TARGET_GREET_JSON);
        dir.write("target-docs.json", TARGET_DOCS_JSON);
        dir
    }

    // Guards the FILTERING COMPOSITION, not the predicates: a project-authored
    // inert UTILITY target must be AGGREGATED into one convenience escalation,
    // while a translatable target beside it still translates. is_cmake_provided
    // and is_inert_target are each unit-tested in isolation; this pins the branch
    // that wires them together (read_codemodel_reply), which otherwise only the
    // Bazel-tier fixture 010 covers — and fixture 010's targets are all
    // CMake-provided, so they never reach the project-authored (aggregated)
    // branch this exercises. Inverting the is_cmake_provided test in that branch
    // passes every other unit test.
    #[test]
    fn read_codemodel_reply_aggregates_a_project_authored_inert_utility() {
        let dir = reply_dir_with_project_utility();

        let codemodel = read_codemodel_reply(
            &dir.path,
            Path::new("/abs/002-with-library"),
            Path::new("/abs/002-with-library/_build"),
            &HashSet::new(),
        )
        .expect("codemodel with a project utility target should parse and translate");

        // The library still translates; the UTILITY target is not a build target.
        let names: Vec<&str> = codemodel.targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["greet"],
            "the translatable library must survive; the UTILITY target is not a cc_ target"
        );

        // Exactly one escalation, and it is the AGGREGATED convenience item that
        // names `docs` — not a silent drop (would be zero) and not a per-target
        // unsupported-type item. Asserting on the count and the named target, not
        // the escalation wording (which is the agent-facing interface).
        assert_eq!(
            codemodel.needs_attention.len(),
            1,
            "a project-authored inert UTILITY target must produce exactly one aggregated \
             escalation, got: {:?}",
            codemodel
                .needs_attention
                .iter()
                .map(|n| &n.title)
                .collect::<Vec<_>>()
        );
        // The aggregated-convenience escalation, distinguished from a per-target
        // unsupported-type item by its title (both name `docs`, so a gap-only
        // check could not tell them apart). Asserting which escalation KIND
        // fired is structure, not the agent-facing wording.
        let item = &codemodel.needs_attention[0];
        assert!(
            item.title.contains("convenience target"),
            "the inert UTILITY must yield the AGGREGATED convenience escalation, not a \
             per-target unsupported-type item; title was: {}",
            item.title
        );
        assert!(
            item.gap.contains("docs"),
            "the escalation must name the project-authored target `docs`, so its drop is a \
             decision and not a silent omission; gap was: {}",
            item.gap
        );
    }

    // Real CMake File API output for the `defs` target, captured verbatim
    // from a build of 009-compile-definitions — NOT hand-constructed. The
    // hand-built target_defines_* test above proves our dedup/ordering
    // logic; this pins the serde contract (`compileGroups[].defines[].define`)
    // against what CMake actually emits, so a dropped or wrong
    // #[serde(rename)] on CompileGroup::defines / CompileGroupDefine::define
    // shows up as an empty result here instead of deserializing to a silent
    // default. This is the "capture the reply that proves us wrong" tier from
    // CLAUDE.md; it supplements the 009 fixture, which is the only thing that
    // catches CMake itself changing. See
    // docs/lore/cmake-file-api-compile-definitions-shape.md.
    const TARGET_DEFS_JSON: &str = r#"{
  "artifacts": [{ "path": "libdefs.a" }],
  "backtrace": 1,
  "backtraceGraph": {
    "commands": ["add_library", "target_compile_definitions", "target_sources"],
    "files": ["CMakeLists.txt"],
    "nodes": [
      { "file": 0 },
      { "command": 0, "file": 0, "line": 22, "parent": 0 },
      { "command": 1, "file": 0, "line": 31, "parent": 0 },
      { "command": 2, "file": 0, "line": 23, "parent": 0 }
    ]
  },
  "compileGroups": [
    {
      "defines": [
        { "backtrace": 2, "define": "PRIVATE_VALUE=7" },
        { "backtrace": 2, "define": "PUBLIC_VALUE=42" }
      ],
      "includes": [
        { "backtrace": 3, "path": "/abs/009-compile-definitions/include" }
      ],
      "language": "CXX",
      "sourceIndexes": [0]
    }
  ],
  "fileSets": [
    {
      "baseDirectories": ["include"],
      "name": "public_headers",
      "type": "HEADERS",
      "visibility": "PUBLIC"
    }
  ],
  "id": "defs::@6890427a1f51a3e7e1df",
  "name": "defs",
  "nameOnDisk": "libdefs.a",
  "paths": { "build": ".", "source": "." },
  "sourceGroups": [
    { "name": "Source Files", "sourceIndexes": [0] },
    { "name": "Header Files", "sourceIndexes": [1] }
  ],
  "sources": [
    { "backtrace": 1, "compileGroupIndex": 0, "path": "src/defs.cpp", "sourceGroupIndex": 0 },
    { "backtrace": 3, "fileSetIndex": 0, "path": "include/defs.hpp", "sourceGroupIndex": 1 }
  ],
  "type": "STATIC_LIBRARY"
}"#;

    // Real CMake File API output for the CTest-injected `Continuous` target,
    // captured verbatim (only sources/artifacts/etc. trimmed away). Pins the
    // serde contract for the three fields the provenance filter added —
    // TargetReply::backtrace, BacktraceGraph::files, BacktraceNode::file — so
    // a dropped/wrong rename shows up as a wrong provenance verdict here
    // rather than deserializing to a default (which would make every injected
    // target look project-authored and re-flood needs_attention). See
    // docs/lore/cmake-include-ctest-injects-utility-targets.md.
    const TARGET_CONTINUOUS_JSON: &str = r#"{
  "name": "Continuous",
  "type": "UTILITY",
  "sources": [],
  "backtrace": 5,
  "backtraceGraph": {
    "commands": ["add_custom_target", "include"],
    "files": [
      "/usr/share/cmake-3.28/Modules/CTestTargets.cmake",
      "/usr/share/cmake-3.28/Modules/CTest.cmake",
      "CMakeLists.txt"
    ],
    "nodes": [
      { "file": 2 },
      { "command": 1, "file": 2, "line": 3, "parent": 0 },
      { "file": 1, "parent": 1 },
      { "command": 1, "file": 1, "line": 264, "parent": 2 },
      { "file": 0, "parent": 3 },
      { "command": 0, "file": 0, "line": 59, "parent": 4 }
    ]
  }
}"#;

    #[test]
    fn injected_target_provenance_deserializes_from_real_capture() {
        let reply: TargetReply = serde_json::from_str(TARGET_CONTINUOUS_JSON)
            .expect("real File API capture should parse");
        assert_eq!(
            defining_command_file(&reply),
            Some("/usr/share/cmake-3.28/Modules/CTestTargets.cmake"),
            "the target's backtrace must resolve to the CTest module it was injected from; \
             a wrong result means a dropped serde rename on backtrace/files/file"
        );
        assert!(
            is_cmake_provided(&reply, Some("/usr/share/cmake-3.28")),
            "a real CTest dashboard target must be recognized as CMake-provided"
        );
    }

    // Real tinyxml2 directory reply installers, captured verbatim. Exercises
    // all the installer kinds present (a target install to lib, an export, a
    // file install to lib/cmake, a file install to include, a file install
    // to lib/pkgconfig) so the include-header extraction is proven to pick
    // out tinyxml2.h and ONLY tinyxml2.h from real bytes — guarding the serde
    // contract on Installer::{type,destination,paths}. See bzl-c54.7.
    const DIRECTORY_TINYXML2_JSON: &str = r#"{
  "installers": [
    { "component": "tinyxml2_development", "destination": "lib",
      "paths": ["libtinyxml2.a"], "type": "target" },
    { "component": "tinyxml2_development", "destination": "lib/cmake/tinyxml2",
      "paths": ["CMakeFiles/Export/x/tinyxml2-static-targets.cmake"], "type": "export" },
    { "component": "tinyxml2_development", "destination": "lib/cmake/tinyxml2",
      "paths": ["cmake/tinyxml2-config.cmake", "/abs/tinyxml2-config-version.cmake"],
      "type": "file" },
    { "component": "tinyxml2_development", "destination": "include",
      "paths": ["tinyxml2.h"], "type": "file" },
    { "component": "tinyxml2_development", "destination": "lib/pkgconfig",
      "paths": ["/abs/tinyxml2.pc"], "type": "file" }
  ],
  "paths": { "build": ".", "source": "." }
}"#;

    #[test]
    fn installed_public_headers_deserializes_from_real_capture() {
        let dir: DirectoryReply = serde_json::from_str(DIRECTORY_TINYXML2_JSON)
            .expect("real File API directory capture should parse");
        assert_eq!(
            installed_public_headers(std::slice::from_ref(&dir)),
            ["tinyxml2.h".to_string()].into_iter().collect(),
            "tinyxml2.h is installed to `include`; the pkgconfig/cmake files and the .a are \
             not headers — an empty or larger result means a dropped serde rename on \
             Installer::type/destination/paths"
        );
    }


    #[test]
    fn target_defines_deserializes_real_capture() {
        let reply: TargetReply =
            serde_json::from_str(TARGET_DEFS_JSON).expect("real File API capture should parse");
        assert_eq!(
            target_defines(&reply),
            vec!["PRIVATE_VALUE=7".to_string(), "PUBLIC_VALUE=42".to_string()],
            "defines must deserialize from real compileGroups[].defines[].define bytes; \
             an empty result here means a dropped/wrong serde rename on \
             CompileGroup::defines or CompileGroupDefine::define"
        );
    }
}
