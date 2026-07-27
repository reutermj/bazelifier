//! CMake File API frontend.
//!
//! Configures the target CMake project, requests the codemodel-v2 (targets,
//! sources, types) and cache-v2 (project version) File API queries, and
//! reads the replies into our internal `BuildGraph` model. See
//! docs/architecture/cmake-frontend.md for why the File API is the source
//! of truth rather than parsing CMakeLists.txt directly.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::model::{self, BuildGraph, ModuleInfo, NeedsAttention, Target, TargetKind};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    CmakeConfigureFailed { stderr: String },
    CmakeBuildFailed { stderr: String },
    NoProject,
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
}

#[derive(Debug, Deserialize)]
struct CompileGroupInclude {
    path: String,
    backtrace: Option<usize>,
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

/// Configures `source_dir` in `build_dir` via `cmake -G Ninja`, requesting
/// the codemodel-v2 and cache-v2 File API queries, actually builds the
/// project (so ground-truth artifacts exist in `build_dir` for validation
/// — see docs/architecture/build-verification.md), and reads the File API
/// replies into a `BuildGraph` (including the module name/version the
/// generated `MODULE.bazel` should use — see
/// docs/architecture/bazel-codegen.md).
pub fn discover(source_dir: &Path, build_dir: &Path) -> Result<BuildGraph, Error> {
    request_file_api_queries(build_dir)?;
    configure(source_dir, build_dir)?;
    build(build_dir)?;
    let reply_dir = build_dir.join(".cmake/api/v1/reply");
    let (module, targets, needs_attention) = read_codemodel_reply(&reply_dir)?;
    let version = read_project_version(&reply_dir)?;

    let mut graph = BuildGraph::new(
        ModuleInfo {
            name: module,
            version,
        },
        targets,
    );
    graph.needs_attention = needs_attention;
    Ok(graph)
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

fn read_codemodel_reply(
    reply_dir: &Path,
) -> Result<(String, Vec<Target>, Vec<NeedsAttention>), Error> {
    let index_path = find_reply_file(reply_dir, "codemodel-v2-")?;
    let index: CodemodelIndexReply = serde_json::from_str(&fs::read_to_string(index_path)?)?;

    let configuration = index.configurations.first().ok_or(Error::NoProject)?;
    let project = configuration.projects.first().ok_or(Error::NoProject)?;

    // Target dependencies are reported by opaque id (e.g.
    // "greet::@6890427a1f51a3e7e1df"), not name — build the lookup before
    // resolving any target's dependencies, since a target can depend on
    // one that appears later in configuration.targets.
    let mut replies_by_id = std::collections::HashMap::new();
    let mut id_to_name = std::collections::HashMap::new();
    for target_ref in &configuration.targets {
        let target_path = reply_dir.join(&target_ref.json_file);
        let target_reply: TargetReply = serde_json::from_str(&fs::read_to_string(target_path)?)?;
        id_to_name.insert(target_ref.id.clone(), target_reply.name.clone());
        replies_by_id.insert(target_ref.id.clone(), target_reply);
    }

    let mut targets = Vec::new();
    let mut needs_attention = Vec::new();

    // Collect which target ids have at least one dependent, to decide
    // whether a library with no public headers is worth flagging — a
    // library nothing depends on has no consumer that could need a header
    // it isn't exposing. See docs/architecture/cmake-frontend.md.
    let mut has_dependents: std::collections::HashSet<String> = std::collections::HashSet::new();
    for reply in replies_by_id.values() {
        for dep in &reply.dependencies {
            has_dependents.insert(dep.id.clone());
        }
    }

    // Targets whose CMake type has no Bazel rule yet. These are escalated
    // via needs_attention/ rather than aborting the whole conversion — one
    // unrecognized target must not cost the project every other target it
    // defines. See docs/architecture/cmake-frontend.md.
    let untranslatable: std::collections::HashMap<String, String> = configuration
        .targets
        .iter()
        .filter_map(|target_ref| {
            let reply = replies_by_id.get(&target_ref.id)?;
            target_kind(&reply.cmake_type)
                .is_none()
                .then(|| (target_ref.id.clone(), reply.cmake_type.clone()))
        })
        .collect();

    // Which surviving targets name each untranslatable one. Built by
    // walking configuration.targets (not the HashMap) so the order recorded
    // in the escalation is deterministic across runs.
    let mut dependents_of: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for target_ref in &configuration.targets {
        let Some(reply) = replies_by_id.get(&target_ref.id) else {
            continue;
        };
        if untranslatable.contains_key(&target_ref.id) {
            continue;
        }
        for dep in &reply.dependencies {
            if untranslatable.contains_key(&dep.id) {
                dependents_of
                    .entry(dep.id.clone())
                    .or_default()
                    .push(reply.name.clone());
            }
        }
    }

    // Names, not ids — a surviving target's `dependencies` have already been
    // resolved from opaque ids back to names by the time they're filtered.
    let untranslatable_names: std::collections::HashSet<String> = untranslatable
        .keys()
        .filter_map(|id| id_to_name.get(id).cloned())
        .collect();

    for target_ref in &configuration.targets {
        let Some(reply) = replies_by_id.remove(&target_ref.id) else {
            continue;
        };

        if let Some(cmake_type) = untranslatable.get(&target_ref.id) {
            needs_attention.push(unsupported_target_needs_attention(
                &reply.name,
                cmake_type,
                dependents_of
                    .get(&target_ref.id)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            ));
            continue;
        }

        let kind = target_kind(&reply.cmake_type)
            .expect("untranslatable targets were filtered out above");
        let is_depended_on = has_dependents.contains(&target_ref.id);
        let (mut target, attention) =
            to_target(reply, kind, &id_to_name, is_depended_on, &index.paths.source);

        // Drop edges to targets that were never emitted. Leaving them would
        // produce a BUILD.bazel referencing a label that doesn't exist,
        // failing at Bazel *analysis* time with an error far removed from
        // the real cause — and leaving the agent no workspace to resolve the
        // escalation in. The lost edges are recorded in the escalation above.
        target
            .dependencies
            .retain(|name| !untranslatable_names.contains(name));

        targets.push(target);
        needs_attention.extend(attention);
    }

    Ok((project.name.clone(), targets, needs_attention))
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

fn find_reply_file(reply_dir: &Path, prefix: &str) -> Result<std::path::PathBuf, Error> {
    for entry in fs::read_dir(reply_dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with(prefix) {
                return Ok(entry.path());
            }
        }
    }
    Err(Error::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("no reply file with prefix '{prefix}' in {reply_dir:?}"),
    )))
}

fn to_target(
    reply: TargetReply,
    kind: TargetKind,
    id_to_name: &std::collections::HashMap<String, String>,
    is_depended_on: bool,
    project_source_dir: &str,
) -> (Target, Vec<NeedsAttention>) {
    let mut sources = Vec::new();
    let mut public_headers = Vec::new();
    let mut generated_sources = Vec::new();
    let mut sources_outside_deliverable = Vec::new();
    let mut has_unclassified_headers = false;
    for source in &reply.sources {
        // The translator can't produce a generated file, and has no way to
        // know what does.
        if source.is_generated {
            generated_sources.push(source.path.clone());
            continue;
        }

        // CMake reports a source path relative to the top-level source
        // directory ONLY when the file is inside it; anything else comes
        // through as an absolute path. Emitting one verbatim would bake this
        // machine's filesystem layout into the generated BUILD.bazel and
        // produce something Bazel can't resolve as a label — and the file
        // isn't in the copied module either, since only the source dir is
        // copied. Same invariant `strip_project_prefix` enforces for include
        // directories.
        if !model::is_module_relative(&source.path) {
            sources_outside_deliverable.push(source.path.clone());
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

    let dependencies: Vec<String> = reply
        .dependencies
        .iter()
        .filter_map(|d| id_to_name.get(&d.id).cloned())
        .collect();

    let includes = own_include_dirs(&reply, project_source_dir);

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
    if !sources_outside_deliverable.is_empty() {
        needs_attention.push(sources_outside_deliverable_needs_attention(
            &reply.name,
            &sources_outside_deliverable,
        ));
    }

    let target = Target {
        name: reply.name,
        kind,
        sources,
        public_headers,
        dependencies,
        includes,
        artifacts: reply.artifacts.into_iter().map(|a| a.path).collect(),
    };

    (target, needs_attention)
}

fn looks_like_header(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("h") | Some("hpp") | Some("hh") | Some("hxx")
    )
}

/// Extracts this target's OWN include directories (as paths relative to
/// the CMake project root, for `cc_library`'s `includes` attribute) from
/// its compile groups — excluding ones inherited from a dependency via
/// `target_link_libraries`.
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
fn own_include_dirs(reply: &TargetReply, project_source_dir: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut includes = Vec::new();

    for group in &reply.compile_groups {
        for include in &group.includes {
            if is_inherited_via_link_libraries(include.backtrace, &reply.backtrace_graph) {
                continue;
            }

            let Some(relative) = strip_project_prefix(&include.path, project_source_dir) else {
                continue;
            };
            if seen.insert(relative.clone()) {
                includes.push(relative);
            }
        }
    }

    includes
}

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

/// Strips `project_source_dir` (an absolute path) as a prefix from
/// `absolute_path`, returning the remainder relative to the project root.
/// Returns `None` for paths outside the project (e.g. system include
/// dirs), which are not translatable to a Bazel `includes` entry.
fn strip_project_prefix(absolute_path: &str, project_source_dir: &str) -> Option<String> {
    let relative = Path::new(absolute_path)
        .strip_prefix(project_source_dir)
        .ok()?;
    if relative.as_os_str().is_empty() {
        return None;
    }
    Some(relative.to_string_lossy().into_owned())
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
    fn backtrace_graph_with_commands(commands: Vec<&str>, node_commands: Vec<Option<usize>>) -> BacktraceGraph {
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
    fn strip_project_prefix_strips_absolute_project_path() {
        assert_eq!(
            strip_project_prefix("/tmp/lib-test/include", "/tmp/lib-test"),
            Some("include".to_string())
        );
    }

    #[test]
    fn strip_project_prefix_none_for_project_root_itself() {
        assert_eq!(strip_project_prefix("/tmp/lib-test", "/tmp/lib-test"), None);
    }

    #[test]
    fn strip_project_prefix_none_for_paths_outside_project() {
        assert_eq!(
            strip_project_prefix("/usr/include", "/tmp/lib-test"),
            None
        );
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
            }],
            backtrace_graph: backtrace_graph_with_commands(
                vec!["target_include_directories", "target_link_libraries"],
                vec![None, Some(0), Some(1), Some(1)],
            ),
        };

        assert_eq!(
            own_include_dirs(&reply, "/proj"),
            vec!["include".to_string()]
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
            to_target(reply, TargetKind::Library, &std::collections::HashMap::new(), true, "/proj");

        assert_eq!(target.sources, vec!["src/greet.cpp".to_string()]);
        assert_eq!(
            target.public_headers,
            vec!["include/greet.hpp".to_string()]
        );
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
            to_target(reply, TargetKind::Library, &std::collections::HashMap::new(), true, "/proj");

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
            to_target(reply, TargetKind::Library, &std::collections::HashMap::new(), false, "/proj");

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
            to_target(reply, TargetKind::Library, &std::collections::HashMap::new(), true, "/proj");

        assert!(needs_attention.is_empty());
    }

    #[test]
    fn to_target_resolves_dependencies_by_id() {
        let mut id_to_name = std::collections::HashMap::new();
        id_to_name.insert("greet::@abc123".to_string(), "greet".to_string());

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

        let (target, _) = to_target(reply, TargetKind::Executable, &id_to_name, false, "/proj");
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

        let (target, needs_attention) = to_target(
            reply,
            TargetKind::Executable,
            &std::collections::HashMap::new(),
            false,
            "/proj",
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
            reply,
            TargetKind::Library,
            &std::collections::HashMap::new(),
            false,
            "/proj",
        );

        assert!(needs_attention.is_empty());
    }

    #[test]
    fn is_module_relative_accepts_paths_inside_the_project() {
        assert!(model::is_module_relative("src/main.cpp"));
        assert!(model::is_module_relative("include/greet.hpp"));
    }

    // CMake only reports a project-relative path when the file is inside
    // the top-level source dir — an absolute path means it isn't, and a
    // `..` component would escape the module root the same way.
    #[test]
    fn is_module_relative_rejects_paths_outside_the_project() {
        assert!(!model::is_module_relative("/abs/shared/helper.cpp"));
        assert!(!model::is_module_relative("../shared/helper.cpp"));
        assert!(!model::is_module_relative("src/../../escape.cpp"));
    }

    // An ordinary (non-generated) source outside the source tree —
    // `add_executable(app ../shared/helper.cpp)`. isGenerated is false here,
    // so classifying on that flag alone would let the absolute path through.
    #[test]
    fn to_target_excludes_sources_outside_deliverable_and_escalates() {
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
                    path: "/abs/shared/helper.cpp".to_string(),
                    file_set_index: None,
                    is_generated: false,
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

        let (target, needs_attention) = to_target(
            reply,
            TargetKind::Executable,
            &std::collections::HashMap::new(),
            false,
            "/proj",
        );

        assert_eq!(
            target.sources,
            vec!["src/main.cpp".to_string()],
            "an absolute path must never reach srcs"
        );
        assert_eq!(needs_attention.len(), 1);
        assert!(
            needs_attention[0].gap.contains("/abs/shared/helper.cpp"),
            "{}",
            needs_attention[0].gap
        );
        assert!(needs_attention[0].title.contains("cannot reach"));
        // The resolution turns on whether the file ships with the project,
        // not on where it happens to sit on this machine.
        assert!(
            needs_attention[0].context.contains("source deliverable"),
            "{}",
            needs_attention[0].context
        );
    }

    #[test]
    fn target_kind_maps_supported_cmake_types() {
        assert_eq!(target_kind("EXECUTABLE"), Some(TargetKind::Executable));
        assert_eq!(target_kind("STATIC_LIBRARY"), Some(TargetKind::Library));
        assert_eq!(target_kind("SHARED_LIBRARY"), Some(TargetKind::Library));
    }

    // These are the types actually reachable from a real codemodel reply
    // (verified against CMake 3.28 with the Ninja generator). An unmapped
    // type must escalate, never abort the conversion — see
    // docs/architecture/cmake-frontend.md.
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

    #[test]
    fn unsupported_target_escalation_names_type_and_target() {
        let item = unsupported_target_needs_attention("gen_docs", "UTILITY", &[]);

        assert!(item.title.contains("gen_docs"));
        assert!(item.title.contains("UTILITY"));
        // Type-specific guidance, not a generic "unsupported" message.
        assert!(item.context.contains("add_custom_target"));
        assert!(
            item.context.contains("No other target"),
            "an unreferenced target should say so explicitly:\n{}",
            item.context
        );
        assert!(item.expected_output.contains("do NOT edit"));
    }

    #[test]
    fn unsupported_target_escalation_records_dropped_dependency_edges() {
        let item = unsupported_target_needs_attention(
            "obj",
            "OBJECT_LIBRARY",
            &["app".to_string(), "app2".to_string()],
        );

        // The agent has to know which targets were left incomplete.
        assert!(item.context.contains("app, app2"), "{}", item.context);
        assert!(item.context.contains("DROPPED"), "{}", item.context);
        assert!(item.context.contains("cc_library"), "{}", item.context);
    }

    #[test]
    fn unsupported_type_guidance_falls_back_for_unknown_types() {
        let guidance = unsupported_type_guidance("SOMETHING_NEW");
        assert!(guidance.contains("no mapping in the translator yet"));
    }
}

/// Escalates sources the translator could not place inside the generated
/// module. The question that decides the resolution is whether the file is
/// part of the source deliverable — see the tier discussion in
/// docs/architecture/cmake-frontend.md.
fn sources_outside_deliverable_needs_attention(
    target_name: &str,
    outside_deliverable: &[String],
) -> NeedsAttention {
    let title = format!("Target '{target_name}' has sources the module cannot reach");
    NeedsAttention {
        gap: format!(
            "Target '{target_name}' compiles {} source file(s) that the translator could not \
             place inside the generated module:\n\n{}\n\nThey were left out of the generated \
             rule's `srcs`. The translator roots a converted module at the CMake project's \
             top-level source directory, and a Bazel label cannot refer to anything above its \
             own module root, so these files have nowhere to live in the output as it is \
             currently laid out.",
            outside_deliverable.len(),
            outside_deliverable
                .iter()
                .map(|p| format!("- `{p}`"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        context: format!(
            "The question that decides what to do here is NOT where the file sits on this \
             machine — it is whether the file is part of the source deliverable being \
             converted (the tarball, checkout, or directory the project ships as its \
             sources).\n\n\
             If it IS part of the deliverable — typically a sibling directory like \
             `../shared/` that ships alongside the project — then nothing is wrong with the \
             project. The file is reproducible, and this is a translator limitation: module \
             roots are not yet derived from the referenced file set, so the translator cannot \
             widen the module to include it. Resolve it by vendoring the file into this \
             module, or by converting the directory that owns it into its own Bazel module \
             and depending on that, which is what the validation workspace's cross-module \
             `bazel_dep` wiring exists to support (see \
             docs/architecture/build-verification.md).\n\n\
             If it is NOT part of the deliverable — an absolute path into a system location, \
             a checkout that only exists on the machine that ran the conversion, a prebuilt \
             artifact — then the gap is real: this build has an input that cannot be \
             reproduced from what the project ships, and no conversion can be faithful while \
             that is true. Vendoring the file is then the only honest fix.\n\n\
             Either way '{target_name}' is missing whatever those files contribute, so it \
             will fail to link if anything references their symbols."
        ),
        expected_output: format!(
            "State which of the two cases above applies for each file, then make it reachable \
             from '{target_name}' by a relative Bazel label — vendored into this module, or \
             supplied by a `deps` edge on another module — and wire it into the generated \
             `BUILD.bazel`. Resolve this in the GENERATED output only — do NOT edit the \
             project's CMakeLists.txt to move or inline the files."
        ),
        title,
    }
}

/// Escalates sources CMake produces during the build rather than reading
/// from the project tree. Kept out of the generated `srcs` — see the
/// `is_generated` handling in `to_target`.
fn generated_sources_needs_attention(target_name: &str, generated: &[String]) -> NeedsAttention {
    let title = format!("Target '{target_name}' consumes generated sources");
    NeedsAttention {
        gap: format!(
            "CMake reports {} source(s) for target '{target_name}' that it generates during \
             the build rather than reading from the project tree:\n\n{}\n\nThey were left out \
             of the generated `cc_*` rule's `srcs`. The File API reports them as absolute \
             paths into the CMake build directory and does not say what produces them, so the \
             translator has nothing it could point `srcs` at.",
            generated.len(),
            generated
                .iter()
                .map(|p| format!("- `{p}`"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        context: format!(
            "This is a translator capability gap, not a problem with the project. A generated \
             file is a perfectly legitimate build input: it is reproducible, because the \
             recipe that produces it ships with the sources. What the translator cannot yet do \
             is translate that recipe, so it has nothing to point `srcs` at. Nothing here \
             needs to be removed or worked around — the recipe needs expressing in \
             Bazel.\n\n\
             Two common causes: an `add_custom_command()` that produces a source, which maps \
             to a `genrule` whose output feeds this target's `srcs`; or a \
             `$<TARGET_OBJECTS:...>` expansion from an `OBJECT_LIBRARY`, in which case the \
             paths above are `.o` files and the real fix is translating that library — look \
             for a separate needs_attention item naming it, and resolving that one likely \
             resolves this too.\n\n\
             '{target_name}' is missing whatever these files contribute. Note the build may \
             still LINK if nothing references the missing symbols, so a green build does not \
             by itself mean this was handled."
        ),
        expected_output: format!(
            "Identify what produces each file above and express it in the generated \
             `BUILD.bazel` — typically a `genrule` (or a `cc_library` replacing the \
             `OBJECT_LIBRARY`) whose output is wired into '{target_name}'. Resolve this in \
             the GENERATED output only — do NOT edit the project's CMakeLists.txt."
        ),
        title,
    }
}

/// Type-specific guidance for an untranslatable target. A generic "this
/// type isn't supported" tells an agent nothing it couldn't read off the
/// title; what's actually useful is the shape of the Bazel answer for that
/// particular CMake construct.
fn unsupported_type_guidance(cmake_type: &str) -> &'static str {
    match cmake_type {
        "UTILITY" => {
            "`UTILITY` is what `add_custom_target()` produces — a named build step, not a \
             compiled artifact. Decide first whether it affects the converted output at all: \
             many utility targets are developer conveniences (docs, formatting, linting) with \
             no place in a Bazel build, in which case the correct resolution is to confirm \
             that and emit nothing. If it does produce a file something else consumes, it \
             maps to a `genrule` (or a custom rule) declaring that file as an output."
        }
        "OBJECT_LIBRARY" => {
            "`OBJECT_LIBRARY` has no direct Bazel equivalent: it exists in CMake to compile a \
             set of sources once and splice the resulting objects into several targets, \
             which is a job Bazel's `cc_library` already does. The usual resolution is a \
             plain `cc_library` with the same `srcs`, depended on normally — Bazel decides \
             object reuse and static/dynamic linking itself."
        }
        "MODULE_LIBRARY" => {
            "`MODULE_LIBRARY` is a plugin loaded at runtime via `dlopen()`, never linked \
             against. The closest Bazel equivalent is `cc_binary(linkshared = True)` with the \
             expected filename, rather than a `cc_library`."
        }
        "INTERFACE_LIBRARY" => {
            "`INTERFACE_LIBRARY` carries no compiled sources — only usage requirements \
             (include dirs, defines, link flags) for its consumers. It maps to a `cc_library` \
             with `hdrs`/`includes` and no `srcs`."
        }
        _ => {
            "This CMake target type has no mapping in the translator yet. Determine what the \
             target contributes to the build and express that with the closest native Bazel \
             rule."
        }
    }
}

/// Escalates a target whose CMake type the translator has no Bazel rule for.
/// The conversion continues without it — see the `untranslatable` handling in
/// `read_codemodel_reply` and docs/architecture/cmake-frontend.md.
fn unsupported_target_needs_attention(
    target_name: &str,
    cmake_type: &str,
    dependents: &[String],
) -> NeedsAttention {
    let title = format!("Target '{target_name}' has unsupported CMake type '{cmake_type}'");

    let dependents_context = if dependents.is_empty() {
        "No other target in this project depends on it, so no dependency edges were lost."
            .to_string()
    } else {
        format!(
            "These targets declared a dependency on '{target_name}': {}. That edge was \
             DROPPED from their generated `deps` — keeping it would emit a label pointing at \
             a target that was never generated, which fails at Bazel analysis time with an \
             error far removed from this cause. If '{target_name}' turns out to contribute \
             symbols or generated files, those targets are incomplete until the edge is \
             restored alongside whatever rule replaces it.",
            dependents.join(", ")
        )
    };

    NeedsAttention {
        gap: format!(
            "Target '{target_name}' has CMake type '{cmake_type}', which the translator has \
             no Bazel rule for — only `EXECUTABLE`, `STATIC_LIBRARY`, and `SHARED_LIBRARY` \
             are mapped today. No rule was generated for it. The rest of the project WAS \
             converted: an unrecognized target is escalated here rather than failing the \
             whole conversion, so the remaining targets are still usable and this gap stays \
             scoped to the one construct that caused it."
        ),
        context: format!(
            "{}\n\n{dependents_context}",
            unsupported_type_guidance(cmake_type)
        ),
        expected_output: format!(
            "Decide what '{target_name}' should become in Bazel and add it to the generated \
             `BUILD.bazel` — including restoring any dependency edge listed above, if the \
             replacement rule warrants one. If the correct answer is that it has no Bazel \
             equivalent, say so explicitly in the resolution rather than silently dropping \
             it; a deliberate omission and an overlooked one are indistinguishable in the \
             output otherwise. Resolve this in the GENERATED output only — do NOT edit the \
             project's CMakeLists.txt."
        ),
        title,
    }
}

fn header_visibility_needs_attention(target_name: &str) -> NeedsAttention {
    let title = format!("Library '{target_name}' has headers but no public FILE_SET");
    NeedsAttention {
        gap: format!(
            "Target '{target_name}' is a library with at least one other target depending \
             on it, and has header-like files among its sources, but none of them are \
             declared as a public `FILE_SET` (`target_sources({target_name} PUBLIC FILE_SET \
             ... TYPE HEADERS ...)`). The CMake File API does not report which plain-source \
             headers are meant for consumers vs. internal-only use, so the translator cannot \
             confidently populate `hdrs` for this target's generated `cc_library` — see \
             docs/architecture/cmake-frontend.md."
        ),
        context: format!(
            "'{target_name}' has at least one dependent target, meaning some other target \
             likely needs to #include one or more of this library's headers. Its generated \
             `cc_library` currently has an empty `hdrs` (all header-like sources were placed \
             in `srcs`). Note this conversion very likely still BUILDS: Bazel does not enforce \
             the hdrs/srcs split by default — a header listed in a dependency's `srcs` is \
             still propagated as an input to dependents' compile actions, so consumers can \
             #include it regardless. (`includes` only supplies the -I search path that \
             determines how the #include is spelled; it is not what exposes the file.) This \
             matches CMake's own looser semantics, where a consumer can #include any header \
             in an include directory whether or not it's the library's 'real' public \
             interface. So the gap here is weaker encapsulation and an unclear public/private \
             boundary, not necessarily a build failure — which is exactly why it needs \
             explicit triage rather than being inferred from a green build. See \
             docs/architecture/build-verification.md's 'Header visibility is not enforced by \
             default'."
        ),
        expected_output: format!(
            "Determine which of '{target_name}''s header files are actually part of its \
             public interface (consumed by dependents via #include) and move those from \
             `srcs` to `hdrs` in the generated `BUILD.bazel`. Resolve this in the GENERATED \
             output only — do NOT edit the project's CMakeLists.txt. The source build files \
             are the input being translated: adding a `FILE_SET` upstream would make this \
             particular project convert cleanly while leaving the translator just as unable \
             to handle the next project that has the same shape."
        ),
        title,
    }
}
