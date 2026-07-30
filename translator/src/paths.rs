//! Path geometry shared by the frontend: normalizing, absolutizing, finding a
//! common ancestor, and resolving a File-API-reported path against the source
//! directory. Deliberately free of any CMake or build-graph knowledge — the
//! frontend's rebasing logic (`cmake_api::rebase_to_module_root`) is built on
//! top of these, but these functions know nothing about targets, tests, or
//! escalations, so they generalize to other build-system frontends. See
//! docs/architecture/cmake-frontend.md on module-root derivation.

use std::path::{Component, Path, PathBuf};

/// Resolves `.` and `..` textually, without touching the filesystem.
///
/// Deliberately not `fs::canonicalize`: under a Bazel sandbox the source
/// tree is a web of symlinks into the execroot and the output base, and
/// resolving them would yield paths that are correct on this machine but
/// meaningless as a description of the module's layout.
pub fn normalize_lexically(path: &Path) -> PathBuf {
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

pub fn absolutize(path: &Path) -> std::io::Result<PathBuf> {
    Ok(normalize_lexically(&std::path::absolute(path)?))
}

/// Deepest directory containing `base` and every path in `others`.
pub fn common_ancestor(base: &Path, others: &[PathBuf]) -> PathBuf {
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
pub fn resolve_against(path: &str, source_dir: &Path) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        normalize_lexically(path)
    } else {
        normalize_lexically(&source_dir.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // The abs-vs-relative branch is the whole of resolve_against: a relative
    // path is joined onto the source dir (CMake reports it relative only when
    // it's inside the project), an absolute one is taken as-is.
    #[test]
    fn resolve_against_joins_relative_but_keeps_absolute() {
        assert_eq!(
            resolve_against("src/main.cpp", Path::new("/deliv/proj")),
            PathBuf::from("/deliv/proj/src/main.cpp")
        );
        assert_eq!(
            resolve_against("/usr/include/x.h", Path::new("/deliv/proj")),
            PathBuf::from("/usr/include/x.h")
        );
    }
}
