//! CMake File API frontend.
//!
//! Configures the target CMake project, requests the codemodel-v2 (targets,
//! sources, types) and cache-v2 (project version) File API queries, and
//! reads the replies into our internal `BuildGraph` model. See
//! docs/architecture/cmake-frontend.md for why the File API is the source
//! of truth rather than parsing CMakeLists.txt directly.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::model::{BuildGraph, ModuleInfo, Target, TargetKind};
use crate::needs_attention::{
    NeedsAttention, generated_sources_needs_attention, header_visibility_needs_attention,
    sources_outside_deliverable_needs_attention, unsupported_target_needs_attention,
};

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
    nodes: Vec<BacktraceNode>,
}

#[derive(Debug, Deserialize)]
struct BacktraceNode {
    command: Option<usize>,
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
    configure(source_dir, build_dir)?;
    build(build_dir)?;
    let reply_dir = build_dir.join(".cmake/api/v1/reply");
    let deliverable_root = absolutize(deliverable_root)?;
    let codemodel = read_codemodel_reply(&reply_dir, &deliverable_root)?;
    let version = read_project_version(&reply_dir)?;

    Ok(Discovery {
        graph: BuildGraph {
            module: ModuleInfo {
                name: codemodel.project_name,
                version,
            },
            targets: codemodel.targets,
        },
        needs_attention: codemodel.needs_attention,
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

fn configure(source_dir: &Path, build_dir: &Path) -> Result<(), Error> {
    let output = Command::new("cmake")
        .arg("-G")
        .arg("Ninja")
        .arg("-B")
        .arg(build_dir)
        .arg("-S")
        .arg(source_dir)
        .output()?;

    if !output.status.success() {
        return Err(Error::CmakeConfigureFailed {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
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

fn read_codemodel_reply(reply_dir: &Path, deliverable_root: &Path) -> Result<Codemodel, Error> {
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

    let mut targets = Vec::new();
    let mut needs_attention = Vec::new();

    for (id, reply) in &replies {
        // A target whose CMake type has no Bazel rule yet is escalated via
        // needs_attention/ rather than aborting the whole conversion — one
        // unrecognized target must not cost the project every other target
        // it defines. See docs/architecture/cmake-frontend.md. The edges it
        // cost are named in the escalation, since dropping them silently
        // would leave dependents quietly incomplete.
        let Some(kind) = target_kind(&reply.cmake_type) else {
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
        let (target, attention) = to_target(reply, kind, &translated_names, is_depended_on);

        targets.push(target);
        needs_attention.extend(attention);
    }

    let source_dir = normalize_lexically(Path::new(&index.paths.source));
    if !source_dir.starts_with(deliverable_root) {
        return Err(Error::SourceDirOutsideDeliverableRoot {
            source_dir: source_dir.to_string_lossy().into_owned(),
            deliverable_root: deliverable_root.to_string_lossy().into_owned(),
        });
    }

    let (module_root, rebase_escalations) =
        rebase_to_module_root(&mut targets, &source_dir, deliverable_root);
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
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn absolutize(path: &Path) -> std::io::Result<PathBuf> {
    Ok(normalize_lexically(&std::path::absolute(path)?))
}

/// Deepest directory containing `base` and every path in `others`.
fn common_ancestor(base: &Path, others: &[PathBuf]) -> PathBuf {
    let mut common: Vec<Component> = base.components().collect();
    for other in others {
        let shared = common
            .iter()
            .zip(other.components())
            .take_while(|(a, b)| **a == *b)
            .count();
        common.truncate(shared);
    }
    common.iter().map(|c| c.as_os_str()).collect()
}

/// Turns a File API path into an absolute one. CMake reports a source path
/// relative to the project's top-level source directory when the file is
/// inside it, and absolute otherwise.
fn resolve_against(path: &str, source_dir: &Path) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        normalize_lexically(path)
    } else {
        normalize_lexically(&source_dir.join(path))
    }
}

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
        let mut unreachable = Vec::new();

        for list in [&mut target.sources, &mut target.public_headers] {
            let mut kept = Vec::with_capacity(list.len());
            for path in list.iter() {
                let absolute = resolve_against(path, source_dir);
                match absolute.strip_prefix(&module_root) {
                    Ok(relative) => kept.push(relative.to_string_lossy().into_owned()),
                    Err(_) => unreachable.push(absolute.to_string_lossy().into_owned()),
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

        if !unreachable.is_empty() {
            escalations.push(sources_outside_deliverable_needs_attention(
                &target.name,
                &unreachable,
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

        let is_public_header = source
            .file_set_index
            .and_then(|i| reply.file_sets.get(i))
            .is_some_and(|fs| {
                fs.fileset_type == "HEADERS"
                    && (fs.visibility == "PUBLIC" || fs.visibility == "INTERFACE")
            });

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
            nodes: node_commands
                .into_iter()
                .map(|command| BacktraceNode { command })
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
            backtrace_graph: empty_backtrace_graph(),
        };

        assert_eq!(
            target_defines(&reply),
            vec!["OWN_DEF".to_string(), "PUB_FROM_DEP=1".to_string()],
            "target_defines must collect every effective define (including \
             inherited PUBLIC ones), deduped, preserving NAME=VALUE verbatim"
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

        let (target, needs_attention) =
            to_target(&reply, TargetKind::Library, &HashMap::new(), true);

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

        let (target, needs_attention) =
            to_target(&reply, TargetKind::Library, &HashMap::new(), true);

        // The plain header stays in srcs, NOT silently promoted to hdrs.
        assert_eq!(
            target.sources,
            vec!["src/greet.cpp".to_string(), "src/greet.hpp".to_string()]
        );
        assert!(target.public_headers.is_empty());

        assert_eq!(needs_attention.len(), 1);
        assert!(needs_attention[0].title.contains("greet"));
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
        let (_target, needs_attention) =
            to_target(&reply, TargetKind::Library, &HashMap::new(), false);

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

        let (_target, needs_attention) =
            to_target(&reply, TargetKind::Library, &HashMap::new(), true);

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
            backtrace_graph: empty_backtrace_graph(),
        };

        let (target, _) = to_target(&reply, TargetKind::Executable, &translated_names, false);
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
            backtrace_graph: empty_backtrace_graph(),
        };

        // "gen_docs" is deliberately absent: it is the UTILITY target that
        // was escalated rather than translated.
        let translated_names = HashMap::from([("greet::@abc123", "greet")]);

        let (target, _) = to_target(&reply, TargetKind::Executable, &translated_names, false);
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
            backtrace_graph: empty_backtrace_graph(),
        };

        let (target, needs_attention) =
            to_target(&reply, TargetKind::Executable, &HashMap::new(), false);

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
            backtrace_graph: empty_backtrace_graph(),
        };

        let (_target, needs_attention) =
            to_target(&reply, TargetKind::Executable, &HashMap::new(), false);

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

        let (_target, needs_attention) =
            to_target(&reply, TargetKind::Library, &HashMap::new(), false);

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

    #[test]
    fn normalize_lexically_resolves_dot_and_parent_components() {
        assert_eq!(
            normalize_lexically(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
    }

    #[test]
    fn common_ancestor_of_paths_under_the_base_is_the_base() {
        assert_eq!(
            common_ancestor(
                Path::new("/deliv/proj"),
                &[
                    PathBuf::from("/deliv/proj/src/main.cpp"),
                    PathBuf::from("/deliv/proj/inc/cfg.hpp"),
                ]
            ),
            PathBuf::from("/deliv/proj")
        );
    }

    #[test]
    fn common_ancestor_widens_to_cover_a_sibling_directory() {
        assert_eq!(
            common_ancestor(
                Path::new("/deliv/proj"),
                &[
                    PathBuf::from("/deliv/proj/src/main.cpp"),
                    PathBuf::from("/deliv/shared/helper.cpp"),
                ]
            ),
            PathBuf::from("/deliv")
        );
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

        let (module_root, escalations) =
            rebase_to_module_root(&mut targets, Path::new("/deliv/proj"), Path::new("/deliv"));

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

    // System include dirs land outside the module and have no `includes`
    // translation — dropping them is correct, and is not a gap.
    #[test]
    fn rebase_drops_include_dirs_outside_the_module_without_escalating() {
        let mut targets = vec![target_with(vec!["src/main.cpp"], vec!["/usr/include"])];

        let (_root, escalations) = rebase_to_module_root(
            &mut targets,
            Path::new("/deliv/proj"),
            Path::new("/deliv/proj"),
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

    fn reply_dir_from_real_capture() -> ScratchDir {
        let dir = ScratchDir::new("codemodel");
        dir.write("codemodel-v2-abc123.json", CODEMODEL_JSON);
        dir.write("target-greet.json", TARGET_GREET_JSON);
        dir.write("target-hello.json", TARGET_HELLO_JSON);
        dir
    }

    #[test]
    fn read_codemodel_reply_wires_real_capture_into_a_build_graph() {
        let dir = reply_dir_from_real_capture();

        let codemodel = read_codemodel_reply(&dir.path, Path::new("/abs/002-with-library"))
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

        match read_codemodel_reply(&dir.path, Path::new("/somewhere/else")) {
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
