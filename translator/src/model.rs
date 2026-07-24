//! Internal build-graph model shared between the CMake frontend and Bazel codegen.
//! See docs/architecture/cmake-frontend.md and docs/architecture/bazel-codegen.md.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetKind {
    Executable,
    // Other CMake target types (STATIC_LIBRARY, SHARED_LIBRARY, ...) are
    // known hard cases for now — see docs/architecture/cmake-frontend.md.
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub kind: TargetKind,
    /// Source file paths, relative to the CMake project root.
    pub sources: Vec<String>,
    /// Build-output artifact paths (e.g. the built binary), relative to
    /// the CMake build directory. Used to locate ground-truth artifacts
    /// for validation — see docs/architecture/build-verification.md.
    pub artifacts: Vec<String>,
}

/// Identifies the standalone Bazel module the translator produces for a
/// CMake project (its own `MODULE.bazel`, independent of bazelifier's own).
/// See docs/architecture/bazel-codegen.md.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInfo {
    /// Derived from CMake's `project()` name.
    pub name: String,
    /// `CMAKE_PROJECT_VERSION`, when the CMake project's `project()` call
    /// specified a `VERSION`. `None` when it didn't — Bazel's `module()`
    /// does not require a version, so codegen omits it rather than
    /// fabricating one.
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildGraph {
    pub module: ModuleInfo,
    pub targets: Vec<Target>,
}
