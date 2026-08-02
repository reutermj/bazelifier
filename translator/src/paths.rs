//! Path geometry shared by both frontends: normalizing, absolutizing, finding
//! a common ancestor, resolving a reported path against the source directory,
//! and rendering one for a reader who cannot see this machine.
//!
//! Deliberately free of any CMake or build-graph knowledge — the frontends'
//! rebasing logic is built on top of these, but nothing here knows about
//! targets or tests, so it generalizes to a third frontend. See
//! docs/architecture/cmake-frontend.md on module-root derivation.
//!
//! [`anchor_for_display`] is the one that names escalation ANCHORS
//! (`<build dir>/`, `<deliverable root>/`), which is closer to the
//! escalation interface than the rest. It lives here anyway because it is
//! still pure geometry — two roots and a path in, a string out — and because
//! the alternative was proven worse: it existed only on the CMake side, and
//! the Autotools frontend shipped raw sandbox paths for months (bzl-ti2).

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

/// Renders an absolute path the way an escalation must name it: anchored on
/// a root the READER can locate, never on this conversion's filesystem.
///
/// An item ships to an agent working in an unpacked workspace, where the
/// paths this conversion used do not exist. Under Bazel they are a sandbox
/// directory carrying a run-specific number
/// (`.../sandbox/linux-sandbox/780/execroot/...`), so an absolute path
/// identifies nothing and is not even stable between runs.
///
/// Four tiers, in descending order of how much the reader can do with them:
/// inside the build dir, inside the deliverable, outside both but sharing an
/// ancestor with the deliverable, and — when even that is `/` — the last two
/// components, which are what actually name the file to someone who cannot
/// see this machine.
///
/// Here rather than in a frontend because it is pure path geometry and both
/// frontends emit the same escalation: the CMake side had this and the
/// Autotools side shipped raw sandbox paths for months, which is the whole
/// reason it moved (bzl-ti2).
pub fn anchor_for_display(absolute: &Path, build_dir: &Path, deliverable_root: &Path) -> String {
    if let Ok(rel) = absolute.strip_prefix(build_dir) {
        return format!("<build dir>/{}", rel.display());
    }
    if let Ok(rel) = absolute.strip_prefix(deliverable_root) {
        return format!("<deliverable root>/{}", rel.display());
    }
    // Genuinely outside both. Anchor on the deepest directory it shares with
    // the deliverable, so the reader has something to orient by.
    let shared = common_ancestor(deliverable_root, &[absolute.to_path_buf()]);
    // `parent().is_some()` excludes `/`: stripping that leaves the whole
    // absolute path, which is what this function exists to avoid.
    if shared.parent().is_some()
        && let Ok(rel) = absolute.strip_prefix(&shared)
    {
        return format!("<outside the deliverable>/{}", rel.display());
    }
    let tail: PathBuf = absolute
        .components()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!(".../{}", tail.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every path an escalation names is read by an agent in an unpacked
    // workspace, where this conversion's absolute paths do not exist. Under
    // Bazel they are a sandbox directory with a run-specific number in them,
    // so they identify nothing and are not even stable across runs.
    #[test]
    fn a_path_is_anchored_on_whichever_root_contains_it() {
        let build = Path::new("/sandbox/780/execroot/build");
        let deliverable = Path::new("/sandbox/780/execroot/src");

        assert_eq!(
            anchor_for_display(
                Path::new("/sandbox/780/execroot/build/config.h"),
                build,
                deliverable
            ),
            "<build dir>/config.h",
            "a generated file is named against the build dir"
        );
        assert_eq!(
            anchor_for_display(
                Path::new("/sandbox/780/execroot/src/lib/a.c"),
                build,
                deliverable
            ),
            "<deliverable root>/lib/a.c",
            "a shipped file is named against the deliverable"
        );
    }

    #[test]
    fn a_path_outside_both_roots_is_anchored_on_what_it_shares() {
        assert_eq!(
            anchor_for_display(
                Path::new("/work/vendor/blob.c"),
                Path::new("/work/proj/build"),
                Path::new("/work/proj"),
            ),
            "<outside the deliverable>/vendor/blob.c",
            "fixture 008's shape: outside the deliverable but sharing an \
             ancestor with it, so the ancestor is what the reader can locate"
        );
    }

    #[test]
    fn a_path_sharing_nothing_falls_back_to_its_tail() {
        assert_eq!(
            anchor_for_display(
                Path::new("/opt/vendor/blob.c"),
                Path::new("/work/proj/build"),
                Path::new("/work/proj"),
            ),
            ".../vendor/blob.c",
            "the common ancestor is `/`, so stripping it would leave the whole \
             absolute path — the last two components are what actually \
             identify the file to someone who cannot see this machine"
        );
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
