//! Internal build-graph model shared between the CMake frontend and Bazel codegen.
//! See docs/architecture/cmake-frontend.md and docs/architecture/bazel-codegen.md.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetKind {
    Executable,
    Library,
    // Other CMake target types (OBJECT_LIBRARY, INTERFACE_LIBRARY, ...) are
    // known hard cases for now — see docs/architecture/cmake-frontend.md.
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub kind: TargetKind,
    /// Private source file paths (compiled, not exposed to consumers),
    /// relative to the CMake project root.
    pub sources: Vec<String>,
    /// Public header file paths — only ones CMake can confidently identify
    /// as public, via a `target_sources(... FILE_SET ... TYPE HEADERS)`
    /// with `PUBLIC`/`INTERFACE` visibility. Headers added as plain
    /// sources (no file set) are NOT included here — see
    /// docs/architecture/cmake-frontend.md on why that distinction can't
    /// be guessed at.
    pub public_headers: Vec<String>,
    /// Names of other targets in this project that this target links
    /// against (from `target_link_libraries`), resolved from the CMake
    /// File API's opaque dependency ids back to target names.
    pub dependencies: Vec<String>,
    /// Include directories (from `target_include_directories`), relative
    /// to the CMake project root. Emitted as `cc_library`'s `includes`
    /// attribute, which Bazel propagates transitively to consumers — so
    /// this only needs to be captured on the target that declared it, not
    /// duplicated onto every dependent. See docs/architecture/cmake-frontend.md.
    pub includes: Vec<String>,
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

/// A gap the translator could not confidently resolve for a specific
/// conversion — written into the output tree's `needs_attention/` (see
/// docs/architecture/runbook-interface.md) for whoever picks up this
/// converted project to address, distinct from bazelifier's own
/// docs/runbooks/ interface docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsAttention {
    pub title: String,
    pub gap: String,
    pub context: String,
    pub expected_output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildGraph {
    pub module: ModuleInfo,
    pub targets: Vec<Target>,
    pub needs_attention: Vec<NeedsAttention>,
}

impl BuildGraph {
    pub fn new(module: ModuleInfo, targets: Vec<Target>) -> Self {
        BuildGraph {
            module,
            targets,
            needs_attention: Vec::new(),
        }
    }
}
