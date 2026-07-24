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

use crate::model::{BuildGraph, ModuleInfo, Target, TargetKind};

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
    artifacts: Vec<TargetArtifact>,
}

#[derive(Debug, Deserialize)]
struct TargetSource {
    path: String,
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
    let (module, targets) = read_codemodel_reply(&reply_dir)?;
    let version = read_project_version(&reply_dir)?;

    Ok(BuildGraph {
        module: ModuleInfo {
            name: module,
            version,
        },
        targets,
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

fn read_codemodel_reply(reply_dir: &Path) -> Result<(String, Vec<Target>), Error> {
    let index_path = find_reply_file(reply_dir, "codemodel-v2-")?;
    let index: CodemodelIndexReply = serde_json::from_str(&fs::read_to_string(index_path)?)?;

    let configuration = index.configurations.first().ok_or(Error::NoProject)?;
    let project = configuration.projects.first().ok_or(Error::NoProject)?;

    let mut targets = Vec::new();
    for target_ref in &configuration.targets {
        let target_path = reply_dir.join(&target_ref.json_file);
        let target_reply: TargetReply = serde_json::from_str(&fs::read_to_string(target_path)?)?;
        targets.push(to_target(target_reply)?);
    }

    Ok((project.name.clone(), targets))
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

fn to_target(reply: TargetReply) -> Result<Target, Error> {
    let kind = match reply.cmake_type.as_str() {
        "EXECUTABLE" => TargetKind::Executable,
        other => {
            return Err(Error::UnsupportedTargetType {
                target: reply.name,
                cmake_type: other.to_string(),
            });
        }
    };

    Ok(Target {
        name: reply.name,
        kind,
        sources: reply.sources.into_iter().map(|s| s.path).collect(),
        artifacts: reply.artifacts.into_iter().map(|a| a.path).collect(),
    })
}
