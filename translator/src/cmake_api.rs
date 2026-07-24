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

use crate::model::{BuildGraph, ModuleInfo, NeedsAttention, Target, TargetKind};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    CmakeConfigureFailed { stderr: String },
    CmakeBuildFailed { stderr: String },
    UnsupportedTargetType { target: String, cmake_type: String },
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
            Error::UnsupportedTargetType { target, cmake_type } => write!(
                f,
                "target '{target}' has unsupported CMake type '{cmake_type}'"
            ),
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

    for target_ref in &configuration.targets {
        let reply = replies_by_id.remove(&target_ref.id).unwrap();
        let is_depended_on = has_dependents.contains(&target_ref.id);
        let (target, attention) = to_target(reply, &id_to_name, is_depended_on, &index.paths.source)?;
        targets.push(target);
        needs_attention.extend(attention);
    }

    Ok((project.name.clone(), targets, needs_attention))
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
    id_to_name: &std::collections::HashMap<String, String>,
    is_depended_on: bool,
    project_source_dir: &str,
) -> Result<(Target, Vec<NeedsAttention>), Error> {
    let kind = match reply.cmake_type.as_str() {
        "EXECUTABLE" => TargetKind::Executable,
        "STATIC_LIBRARY" | "SHARED_LIBRARY" => TargetKind::Library,
        other => {
            return Err(Error::UnsupportedTargetType {
                target: reply.name,
                cmake_type: other.to_string(),
            });
        }
    };

    let mut sources = Vec::new();
    let mut public_headers = Vec::new();
    let mut has_unclassified_headers = false;
    for source in &reply.sources {
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

    let target = Target {
        name: reply.name,
        kind,
        sources,
        public_headers,
        dependencies,
        includes,
        artifacts: reply.artifacts.into_iter().map(|a| a.path).collect(),
    };

    Ok((target, needs_attention))
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
                },
                TargetSource {
                    path: "include/greet.hpp".to_string(),
                    file_set_index: Some(0),
                },
            ],
            public_file_set(),
        );

        let (target, needs_attention) =
            to_target(reply, &std::collections::HashMap::new(), true, "/proj").unwrap();

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
                },
                TargetSource {
                    path: "src/greet.hpp".to_string(),
                    file_set_index: None,
                },
            ],
            vec![],
        );

        let (target, needs_attention) =
            to_target(reply, &std::collections::HashMap::new(), true, "/proj").unwrap();

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
                },
                TargetSource {
                    path: "src/greet.hpp".to_string(),
                    file_set_index: None,
                },
            ],
            vec![],
        );

        // is_depended_on = false: nothing in the project links against
        // this library, so there's no consumer that could need a header
        // it isn't exposing — not worth flagging.
        let (_target, needs_attention) =
            to_target(reply, &std::collections::HashMap::new(), false, "/proj").unwrap();

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
            }],
            vec![],
        );

        let (_target, needs_attention) =
            to_target(reply, &std::collections::HashMap::new(), true, "/proj").unwrap();

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

        let (target, _) = to_target(reply, &id_to_name, false, "/proj").unwrap();
        assert_eq!(target.dependencies, vec!["greet".to_string()]);
    }

    #[test]
    fn to_target_rejects_unsupported_target_type() {
        let reply = TargetReply {
            name: "weird".to_string(),
            cmake_type: "OBJECT_LIBRARY".to_string(),
            sources: vec![],
            file_sets: vec![],
            dependencies: vec![],
            artifacts: vec![],
            compile_groups: vec![],
            backtrace_graph: empty_backtrace_graph(),
        };

        let err = to_target(reply, &std::collections::HashMap::new(), false, "/proj").unwrap_err();
        match err {
            Error::UnsupportedTargetType { target, cmake_type } => {
                assert_eq!(target, "weird");
                assert_eq!(cmake_type, "OBJECT_LIBRARY");
            }
            other => panic!("expected UnsupportedTargetType, got {other:?}"),
        }
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
             in `srcs`). Note this conversion likely still BUILDS: if '{target_name}' also has \
             a `target_include_directories()`-derived `includes` entry, Bazel's `includes` \
             exposes every file under that directory to consumers (a -I-style search path), \
             not just declared `hdrs` — matching CMake's own looser semantics, where a \
             consumer can #include any header in an include directory whether or not it's the \
             library's 'real' public interface. So the gap here is weaker encapsulation and an \
             unclear public/private boundary, not necessarily a build failure."
        ),
        expected_output: format!(
            "Determine which of '{target_name}''s header files are actually part of its \
             public interface (consumed by dependents via #include), move those from `srcs` \
             to `hdrs` in the generated `BUILD.bazel`, and consider adding a `target_sources \
             ... FILE_SET ... TYPE HEADERS` declaration to the original CMakeLists.txt so \
             future conversions of this project resolve this automatically with proper \
             hdrs/srcs separation instead of relying on `includes`' looser exposure."
        ),
        title,
    }
}
