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

use crate::model::{BuildGraph, ModuleInfo, Target, TargetKind, Test};
use crate::needs_attention::{
    NeedsAttention, generated_sources_needs_attention, header_visibility_needs_attention,
    inert_convenience_targets_needs_attention, sources_outside_deliverable_needs_attention,
    unsupported_target_needs_attention,
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

/// `ctest --show-only=json-v1` output. The File API has no test model, so
/// registered tests (add_test) and their properties come from here instead
/// — see docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md.
#[derive(Debug, Deserialize)]
struct CtestReply {
    tests: Vec<CtestTest>,
}

#[derive(Debug, Deserialize)]
struct CtestTest {
    name: String,
    // The resolved command line: [executable, args...]. Absent until the
    // test's executable is actually built (ctest can't resolve the path
    // before then); the translator builds first, so it is present.
    #[serde(default)]
    command: Vec<String>,
    #[serde(default)]
    properties: Vec<CtestProperty>,
}

#[derive(Debug, Deserialize)]
struct CtestProperty {
    name: String,
    // Property values are polymorphic in the ctest schema (a string for
    // WORKING_DIRECTORY, a list for PASS_REGULAR_EXPRESSION); kept as raw
    // JSON and interpreted per-property by `read_tests`.
    value: serde_json::Value,
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
    configure(source_dir, build_dir)?;
    build(build_dir)?;
    let reply_dir = build_dir.join(".cmake/api/v1/reply");
    let deliverable_root = absolutize(deliverable_root)?;
    let codemodel = read_codemodel_reply(&reply_dir, &deliverable_root)?;
    let version = read_project_version(&reply_dir)?;

    // Tests come from ctest, not the File API (see read_tests), and their
    // working directories are rebased against the same module root the
    // targets' paths were.
    let mut tests = read_tests(build_dir)?;
    rebase_tests_to_module_root(&mut tests, &codemodel.module_root);

    Ok(Discovery {
        graph: BuildGraph {
            module: ModuleInfo {
                name: codemodel.project_name,
                version,
            },
            targets: codemodel.targets,
            tests,
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

/// Runs `ctest --show-only=json-v1` in the build directory and translates
/// each registered test into a `Test`. The File API has no test
/// model, so this is the only structured source for `add_test` and its
/// properties — see docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md.
///
/// Working directories are returned ABSOLUTE (as CTest reports them);
/// `rebase_tests_to_module_root` makes them module-relative once the module
/// root is known, matching how target source paths are handled.
///
/// A ctest invocation that fails (no CTest, no tests configured) yields an
/// empty test list rather than an error: a project with no registered tests
/// is the common case, not a failure. Tests whose command never resolved
/// (executable not built) are skipped — there is no binary to run.
fn read_tests(build_dir: &Path) -> Result<Vec<Test>, Error> {
    let output = Command::new("ctest")
        .arg("--show-only=json-v1")
        .current_dir(build_dir)
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let reply: CtestReply = serde_json::from_slice(&output.stdout)?;
    Ok(ctest_reply_to_tests(reply))
}

/// Translates a parsed `ctest --show-only=json-v1` reply into the model's
/// tests. Split from `read_tests` so a frozen real capture can drive it
/// without shelling out. Working directories are still absolute here — see
/// `rebase_tests_to_module_root`.
fn ctest_reply_to_tests(reply: CtestReply) -> Vec<Test> {
    let mut tests = Vec::new();
    for test in reply.tests {
        // The first command element is the executable; its basename is the
        // generated cc_binary this test runs. No command means CTest could
        // not resolve the test's binary, so there is nothing to wrap.
        let Some(executable) = test.command.first() else {
            continue;
        };
        let target = Path::new(executable)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| executable.clone());

        let mut working_directory = String::new();
        let mut pass_regex = None;
        for property in &test.properties {
            match property.name.as_str() {
                "WORKING_DIRECTORY" => {
                    if let Some(dir) = property.value.as_str() {
                        working_directory = dir.to_string();
                    }
                }
                // CMake stores this as a list; a test rarely declares more
                // than one, and the tinyxml2-shaped scope takes the first.
                "PASS_REGULAR_EXPRESSION" => {
                    pass_regex = property
                        .value
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                }
                _ => {}
            }
        }

        tests.push(Test {
            name: test.name,
            target,
            working_directory,
            pass_regex,
        });
    }
    tests
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

/// Rewrites each test's `working_directory` from the absolute path CTest
/// reported to one relative to `module_root`, in place — the same rebasing
/// target paths get, but for tests. A working directory at the module root
/// becomes empty. One that resolves outside the module root is left as the
/// empty string (run at the module root): the tinyxml2-shaped scope does not
/// yet escalate a test whose data lives outside the deliverable, and running
/// at the root is the safe default rather than baking in an absolute path.
fn rebase_tests_to_module_root(tests: &mut [Test], module_root: &Path) {
    for test in tests {
        let absolute = normalize_lexically(Path::new(&test.working_directory));
        test.working_directory = absolute
            .strip_prefix(module_root)
            .map(|rel| rel.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
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

/// Whether an install destination names an include directory — i.e. a file
/// installed there is being declared a public header. Matches `include` and
/// anything under it (`include/tinyxml2`, ...), which is what
/// `install(FILES ... TYPE INCLUDE)` and `CMAKE_INSTALL_INCLUDEDIR` produce.
/// The destination is relative to `CMAKE_INSTALL_PREFIX`, so a leading
/// component of `include` is the whole signal — a file installed to `lib`,
/// `bin`, or `share` is not a header.
fn is_include_destination(destination: &str) -> bool {
    let first = Path::new(destination).components().next();
    matches!(first, Some(Component::Normal(c)) if c == "include")
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
        // Not a header destination — these are where a target install, a
        // pkgconfig file, and cmake package files land.
        assert!(!is_include_destination("lib"));
        assert!(!is_include_destination("lib/pkgconfig"));
        assert!(!is_include_destination("lib/cmake/tinyxml2"));
        assert!(!is_include_destination("share/doc"));
        // A path that merely CONTAINS "include" deeper down is not an include
        // destination — only a leading `include` component counts.
        assert!(!is_include_destination("lib/include"));
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

        let codemodel = read_codemodel_reply(&dir.path, Path::new("/abs/002-with-library"))
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

    // Real `ctest --show-only=json-v1` output for tinyxml2's xmltest,
    // captured verbatim. Pins the parse of the test model — command,
    // WORKING_DIRECTORY, PASS_REGULAR_EXPRESSION — against what CTest
    // actually emits, since the File API has no test model to cross-check
    // against. See docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md.
    const CTEST_XMLTEST_JSON: &str = r#"{
  "kind": "ctestInfo",
  "version": { "major": 1, "minor": 0 },
  "tests": [
    {
      "name": "xmltest",
      "command": [ "/abs/build/xmltest" ],
      "properties": [
        { "name": "PASS_REGULAR_EXPRESSION", "value": [", Fail 0"] },
        { "name": "WORKING_DIRECTORY", "value": "/abs/src" }
      ]
    }
  ]
}"#;

    #[test]
    fn ctest_reply_parses_real_capture_into_a_test() {
        let reply: CtestReply =
            serde_json::from_str(CTEST_XMLTEST_JSON).expect("real ctest capture should parse");
        let tests = ctest_reply_to_tests(reply);
        assert_eq!(tests.len(), 1);
        let t = &tests[0];
        assert_eq!(t.name, "xmltest");
        assert_eq!(
            t.target, "xmltest",
            "the test target is the basename of the command's executable"
        );
        assert_eq!(
            t.working_directory, "/abs/src",
            "WORKING_DIRECTORY (a string property) must be read verbatim"
        );
        assert_eq!(
            t.pass_regex.as_deref(),
            Some(", Fail 0"),
            "PASS_REGULAR_EXPRESSION (a list property) must yield its first entry"
        );
    }

    // A test whose command never resolved (executable not built) has no
    // binary to wrap and is skipped, rather than emitting a test that can't
    // run.
    #[test]
    fn ctest_reply_skips_a_test_with_no_command() {
        let reply = CtestReply {
            tests: vec![CtestTest {
                name: "unbuilt".to_string(),
                command: vec![],
                properties: vec![],
            }],
        };
        assert!(ctest_reply_to_tests(reply).is_empty());
    }

    #[test]
    fn rebase_tests_makes_working_directory_module_relative() {
        let mut tests = vec![
            Test {
                name: "at_root".to_string(),
                target: "t".to_string(),
                working_directory: "/proj".to_string(),
                pass_regex: None,
            },
            Test {
                name: "in_subdir".to_string(),
                target: "t".to_string(),
                working_directory: "/proj/tests/data".to_string(),
                pass_regex: None,
            },
        ];
        rebase_tests_to_module_root(&mut tests, Path::new("/proj"));
        // The module root itself becomes empty; a subdir becomes relative.
        assert_eq!(tests[0].working_directory, "");
        assert_eq!(tests[1].working_directory, "tests/data");
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
