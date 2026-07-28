//! Internal build-graph model shared between the CMake frontend and Bazel codegen.
//! See docs/architecture/cmake-frontend.md and docs/architecture/bazel-codegen.md.

use std::path::{Component, Path};

/// Whether `path` satisfies the contract every path-valued field of
/// [`Target`] is required to meet: relative to the converted module's root,
/// and not escaping it.
///
/// This is the single invariant standing between the translator and
/// non-portable output. The CMake File API reports a source path relative
/// to the project only when the file is actually inside it and absolute
/// otherwise, so an unvalidated passthrough bakes the build machine's
/// filesystem layout into a module that is supposed to be checked into
/// someone else's repo.
///
/// Bazel only catches part of this on its own: an absolute path in a
/// *label* attribute (`srcs`, `hdrs`, `deps`) is an analysis error, but
/// `includes` is a plain string list and an absolute path there is
/// accepted silently — the module then builds on the machine that
/// generated it and nowhere else. Hence the check lives here and is
/// enforced at codegen, not left to Bazel.
pub fn is_module_relative(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute() && !path.components().any(|c| matches!(c, Component::ParentDir))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetKind {
    Executable,
    Library,
    // Other CMake target types (OBJECT_LIBRARY, INTERFACE_LIBRARY, ...) are
    // known hard cases for now — see docs/architecture/cmake-frontend.md.
}

/// One translated build target.
///
/// Every path-valued field except `artifacts` is relative to the converted
/// module's root — see [`is_module_relative`], which is the contract they
/// all have to meet. That root is derived rather than assumed to be the
/// CMake project directory, so these are not simply the paths the File API
/// reported; `cmake_api::rebase_to_module_root` rewrites them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub kind: TargetKind,
    /// Private source file paths (compiled, not exposed to consumers).
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
    /// Include directories (from `target_include_directories`). Emitted as
    /// the `includes` attribute, which Bazel propagates transitively to
    /// consumers — so this only needs to be captured on the target that
    /// declared it, not duplicated onto every dependent. See
    /// docs/architecture/cmake-frontend.md.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildGraph {
    pub module: ModuleInfo,
    pub targets: Vec<Target>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_module_relative_accepts_paths_inside_the_module() {
        assert!(is_module_relative("src/main.cpp"));
        assert!(is_module_relative("include/greet.hpp"));
        // The widened-root case: a module rooted above the CMake project
        // holds the project's own sources under a subdirectory.
        assert!(is_module_relative("proj/src/main.cpp"));
    }

    // CMake only reports a project-relative path when the file is inside
    // the top-level source dir — an absolute path means it isn't, and a
    // `..` component would escape the module root the same way.
    #[test]
    fn is_module_relative_rejects_paths_outside_the_module() {
        assert!(!is_module_relative("/abs/shared/helper.cpp"));
        assert!(!is_module_relative("../shared/helper.cpp"));
        assert!(!is_module_relative("src/../../escape.cpp"));
    }
}
