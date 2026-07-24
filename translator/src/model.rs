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
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildGraph {
    pub targets: Vec<Target>,
}
