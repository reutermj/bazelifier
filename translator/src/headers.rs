//! Header classification: which headers a target must carry, and which of
//! them the project declared *public*.
//!
//! CMake reports a target's headers in `sources` without saying whether they
//! are part of its interface, and reports headers reachable only via an
//! include path not at all. Both gaps are answered here, and the difference
//! between the two answers is what authority the project granted:
//!
//! - **public** — an `install(FILES ... DESTINATION <include>)` or a
//!   `FILE_SET`. The project said so. Becomes a library's `hdrs`.
//! - **present** — the header sits in one of the target's own include
//!   directories. A compiler can resolve it, which is all a compile needs.
//!   Becomes `sources`, where a header contributes nothing but its existence
//!   in the sandbox.
//!
//! Nothing here reaches back into `cmake_api`; only
//! [`installed_public_headers`] touches the File API at all, and only to
//! read `installers[]`. See docs/architecture/cmake-frontend.md and
//! docs/lore/everything-on-the-include-path-is-an-input.md.
//!
//! ## Two header predicates, deliberately different
//!
//! [`looks_like_header`] and [`is_header_file`] do not accept the same
//! extensions, and unifying them would be a silent behavior change. They
//! answer different questions and their failure directions are opposite:
//!
//! | | accepts | drives | a wrong `true` costs |
//! |---|---|---|---|
//! | [`looks_like_header`] | `h hpp hh hxx` | the header-visibility **escalation** | a spurious `needs_attention` item, i.e. wasted agent work on a non-gap |
//! | [`is_header_file`] | those **+ `inc ipp`** | what gets **staged** into the module | a file copied that nothing reads — free |
//!
//! So the staging predicate can afford to be generous and the escalation
//! predicate cannot. `.inc`/`.ipp` are the live case: they are frequently
//! `#include`d bodies rather than interface headers, so staging them is
//! right and escalating on them is not.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::model::{Target, TargetKind};
use crate::paths::normalize_lexically;

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
pub(crate) fn looks_like_header(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("h") | Some("hpp") | Some("hh") | Some("hxx")
    )
}

/// Adds files the compiler actually opened for a target but that nothing
/// declared — the enrichment `ninja_deps` exists to supply.
///
/// Two shapes reach here and nothing else does:
///
/// - A header found by the **quoted-include rule** searching the including
///   file's own directory. `inject_headers_on_include_dirs` covers the common
///   version by treating each source's directory as an implicit include path,
///   but only non-recursively; a relative include escaping that directory
///   (`"../other/x.h"`) is invisible to it.
/// - A **source textually `#include`d** rather than compiled, which CMake
///   deliberately keeps out of the target's source list so it is not also
///   built separately. fmt's `posix-mock-test` does
///   `#include "../src/os.cc"`.
///
/// Nothing lands in `public_headers`: having been opened is evidence of a
/// compile-time dependency, not of membership in the interface — the same
/// distinction that separates the two passes above. Headers go to `sources`
/// and non-headers to `textual_sources`, which need different Bazel
/// attributes (`srcs` compiles a `.cc`; `textual_hdrs` only stages it).
///
/// Only ever adds. It cannot remove: `ninja_deps` sees one configuration, and
/// a header behind a disabled `#ifdef` is absent from it while still being a
/// real input for a consumer configuring differently.
pub(crate) fn inject_opened_files(
    targets: &mut [Target],
    opened_by_target: &HashMap<String, Vec<String>>,
) {
    for target in targets.iter_mut() {
        let Some(opened) = opened_by_target.get(&target.name) else {
            continue;
        };
        let accounted: HashSet<&str> = target
            .sources
            .iter()
            .chain(target.public_headers.iter())
            .map(String::as_str)
            .collect();

        let (mut headers, mut textual): (Vec<String>, Vec<String>) = opened
            .iter()
            .filter(|f| !accounted.contains(f.as_str()))
            .filter(|f| !target.textual_sources.contains(f))
            .cloned()
            .partition(|f| is_header_file(Path::new(f)));

        headers.sort();
        target.sources.extend(headers);

        // A non-header that was opened is a source #included TEXTUALLY: CMake
        // left it out of this target's sources precisely so it is not also
        // compiled separately. It cannot go in `srcs` — Bazel treats a .cc
        // there as a translation unit and never stages it for the compile
        // that includes it — so it becomes `textual_hdrs`. See
        // `model::Target::textual_sources`.
        textual.sort();
        target.textual_sources.extend(textual);
    }
}

/// Whether an `install(FILES ...)` destination is a public-header include
/// directory. Matches a leading `include` (`install(... DESTINATION include)`,
/// the relative form) AND an absolute one anywhere in the path
/// (`.../include/json-c`, which is what `CMAKE_INSTALL_FULL_INCLUDEDIR`
/// expands to — json-c installs there). The absolute form is common: a project
/// that uses the `GNUInstallDirs`/`CMAKE_INSTALL_FULL_*` variables gets a fully
/// resolved `<prefix>/include/...` rather than a bare `include`.
///
/// A nested `include` under `lib`/`lib64`/`share` is deliberately NOT matched:
/// `lib/include` is a build-private include tree, not the public install
/// location, and treating its contents as public headers would be wrong.
pub(crate) fn is_include_destination(destination: &str) -> bool {
    let components: Vec<&str> = Path::new(destination)
        .components()
        .filter_map(|c| match c {
            Component::Normal(c) => c.to_str(),
            _ => None,
        })
        .collect();
    components.iter().enumerate().any(|(i, &c)| {
        c == "include"
            && !matches!(
                i.checked_sub(1).map(|p| components[p]),
                Some("lib") | Some("lib64") | Some("share")
            )
    })
}

/// Adds public headers the project `install()`s to an include destination but
/// that NO target's source list enumerated, attaching each to the library
/// whose include path it sits on.
///
/// Why this is needed at all: a huge class of C projects list only `.c` files
/// on a target and leave headers ambient on the include path (see
/// docs/architecture/cmake-frontend.md). CMake compiles each `.c` and finds
/// its headers via `-I`; the header is never a build input, so the File API
/// never reports it as a target source, so `copy_referenced_sources` never
/// copies it — and the converted library fails to compile the moment one of
/// its `.c` files `#include`s it. json-c is the live case: a CMakeLists
/// ordering bug drops `json_pointer.h`/`json_patch.h` from the library's
/// header list, yet `json_pointer.c` includes `json_pointer.h`.
///
/// The `install(FILES ... DESTINATION <include>)` rule is the project's own
/// authoritative statement that these headers are public, so an unenumerated
/// one is added to `public_headers` (→ a library's `hdrs`), not `sources`.
/// This is deliberately narrower than "copy every header under the include
/// dirs": only headers the project explicitly declared public are injected, so
/// a header with no such evidence still defaults to whatever the target already
/// said about it (private, or absent) rather than being guessed public.
///
/// Attribution is by include path: the header is added to every LIBRARY whose
/// own include directories contain the header's parent directory. That handles
/// json-c's two libraries (shared + static, same include dirs) getting the
/// same headers, and avoids attaching a header to a library that can't see it.
///
/// Paths here are still in the File API's frame (pre-rebase): target
/// `sources`/`public_headers` and the relative install paths are
/// project-relative; include dirs and the generated headers' install paths are
/// absolute. An install path that resolves outside `source_dir` (a generated
/// header in the build dir, like `json.h`) is skipped — those are reproduced
/// by the config_header machinery, not copied.
pub(crate) fn inject_unenumerated_installed_headers(
    targets: &mut [Target],
    installed_headers: &HashSet<String>,
    source_dir: &Path,
) {
    // Every header path any target already accounts for, so an install-declared
    // header a target DID enumerate isn't added a second time. Owned (not
    // borrowed from `targets`) so the mutable pass below is free to push.
    let enumerated: HashSet<String> = targets
        .iter()
        .flat_map(|t| t.sources.iter().chain(t.public_headers.iter()))
        .cloned()
        .collect();

    // Sorted: `installed_headers` is a HashSet, and pushing in iteration order
    // would make `public_headers` (hence the generated `hdrs`) nondeterministic
    // across runs — the reproducible-output invariant every other list holds.
    let mut candidates: Vec<&String> = installed_headers.iter().collect();
    candidates.sort();

    for header in candidates {
        if enumerated.contains(header.as_str()) {
            continue;
        }
        // Resolve to absolute to both bound it to the source tree and match it
        // against the (absolute) include dirs. An absolute install path that is
        // not under source_dir is a generated/out-of-tree file — skip it.
        let absolute = if Path::new(header).is_absolute() {
            normalize_lexically(Path::new(header))
        } else {
            normalize_lexically(&source_dir.join(header))
        };
        if !absolute.starts_with(source_dir) || !absolute.is_file() {
            continue;
        }
        let Some(parent) = absolute.parent() else {
            continue;
        };

        for target in targets.iter_mut() {
            if target.kind != TargetKind::Library {
                continue;
            }
            let on_include_path = target
                .includes
                .iter()
                .any(|dir| normalize_lexically(Path::new(dir)) == parent);
            if on_include_path && !target.public_headers.contains(header) {
                target.public_headers.push(header.clone());
            }
        }
    }
}

/// Attaches every header sitting in a target's own include directories that
/// no target enumerated, so Bazel stages it into the compile sandbox.
///
/// The two build systems disagree about what a header *is*. CMake compiles a
/// `.c` and lets the preprocessor find headers on disk via `-I`; a header is
/// therefore never a build input and the File API never reports it as a
/// target source. Bazel stages only DECLARED inputs, so the same header is
/// absent at compile time and the build fails with `file not found` while the
/// `-I` flag is present and correct.
///
/// **Everything on the include path is an input.** That is CMake's own
/// semantic, and it is the whole declaration available: CMake has no per-file
/// statement that a target uses one header from a directory and not another,
/// so there is nothing here to be more precise than.
///
/// An earlier version of this scanned each source for `#include "..."` and
/// staged only the named headers. That was rejected: it re-implemented a
/// preprocessor badly (no `#if` evaluation, no computed includes, quoted form
/// only), it made the frontend read source text when its source of truth is
/// the File API (docs/architecture/cmake-frontend.md), and it bought nothing
/// — on json-c the two approaches select the identical set of headers.
///
/// Distinct from `inject_unenumerated_installed_headers`, which acts on the
/// project's `install()` declarations — its authoritative statement that a
/// header is PUBLIC — and populates a library's `public_headers` (→ `hdrs`).
/// An include directory carries no such claim, so headers land in `sources`,
/// where a header contributes nothing but its presence in the sandbox.
///
/// Include dirs outside the source tree are skipped: those are the build
/// directory (whose headers the config_header machinery reproduces) and
/// toolchain paths, neither of which the module may carry. Within one, the
/// walk is recursive — see `headers_under`.
///
/// Paths are in the File API's frame (pre-rebase): `sources` are
/// project-relative, include dirs absolute.
pub(crate) fn inject_headers_on_include_dirs(targets: &mut [Target], source_dir: &Path) {
    for target in targets.iter_mut() {
        // Per-target, NOT global: a header can be enumerated on one target and
        // reachable only via the include path on another, and asking "does any
        // target list it?" would suppress it exactly where it is needed.
        // json-c is the live case — `test1Formatted` lists parse_flags.h while
        // its sibling `test1` (same include dir, one source) does not.
        let enumerated: HashSet<&str> = target
            .sources
            .iter()
            .chain(target.public_headers.iter())
            .map(String::as_str)
            .collect();

        let mut discovered: Vec<String> = Vec::new();

        // Each compiled source's OWN directory is an include path the File API
        // never reports, because it is a language rule rather than a build
        // setting: the QUOTED form of #include searches the including file's
        // directory first, before any -I. fmt's test/util.cc does
        // `#include "util.h"` with test/util.h beside it, listed on no target
        // and covered by no include dir (bzl-2wi.4).
        //
        // Scanned NON-recursively, unlike a declared include dir: the rule
        // reaches siblings, not a nested tree. `#include "sub/foo.h"` from
        // here resolves relative to this directory, and sub/ would need to be
        // walked for that — deliberately left out until a project needs it,
        // since recursing here would sweep in a whole source tree for any
        // target with a source at its root.
        let implicit: Vec<String> = target
            .sources
            .iter()
            .filter(|s| !is_header_file(Path::new(s)))
            .filter_map(|s| Path::new(s).parent())
            .map(|p| source_dir.join(p).to_string_lossy().into_owned())
            .collect();

        let declared = target.includes.iter().map(|d| (d, true));
        let implicit_dirs = implicit.iter().map(|d| (d, false));
        for (dir, recursive) in declared.chain(implicit_dirs) {
            let absolute = normalize_lexically(Path::new(dir));
            // The `strip_prefix` below would also reject an outside directory,
            // so this looks redundant and is not: it skips the `read_dir` of a
            // toolchain include path entirely (/usr/include and friends are
            // large), rather than listing it and discarding every entry.
            if !absolute.starts_with(source_dir) {
                continue;
            }
            for path in headers_in(&absolute, recursive) {
                let Ok(relative) = path.strip_prefix(source_dir) else {
                    continue;
                };
                let relative = relative.to_string_lossy().into_owned();
                if !enumerated.contains(relative.as_str()) && !discovered.contains(&relative) {
                    discovered.push(relative);
                }
            }
        }

        // Sorted: read_dir order is filesystem-dependent, and the generated
        // `srcs` must not vary between runs.
        discovered.sort();
        target.sources.extend(discovered);
    }
}

/// Every header in `dir` — at or below it when `recursive`, otherwise only
/// its immediate entries.
///
/// Recursive because `-Iinclude` with `#include "sub/foo.h"` is ordinary C:
/// the header sits at `include/sub/foo.h`, and a flat listing of `include/`
/// misses it entirely. That was bzl-htv, and its symptom is indistinguishable
/// from the bug this whole module fixes — a `file not found` for a header the
/// `-I` flag plainly covers — so it would have looked like a regression of
/// something already fixed rather than a separate gap.
///
/// Symlinks ARE followed: under Bazel every staged input is a symlink into
/// the sandbox, so not following them finds nothing. The cost is that a
/// symlinked directory cycle would loop; no CMake project seen so far has
/// one, and the alternative — working on a checkout and silently doing
/// nothing under Bazel — is far worse, since the fixture tier is the only
/// place that difference is visible.
fn headers_in(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Follows symlinks, deliberately. Bazel stages every input file
            // into the sandbox AS a symlink, so refusing to follow them makes
            // this find nothing at all under Bazel while working fine on a
            // plain checkout — a difference that shows up only in the fixture
            // tier and looks like the feature was never implemented.
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                if recursive {
                    stack.push(path);
                }
            } else if meta.is_file() && is_header_file(&path) {
                found.push(path);
            }
        }
    }
    found
}

/// Whether a path names a C/C++ header by extension. Extension-based because
/// that is what a compiler's include resolution and CMake's own header
/// classification go on; a directory on the include path can also hold `.c`
/// files (json-c's `tests/`), which are sources of some other target and must
/// not be swept in as headers.
pub(crate) fn is_header_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "h" | "hpp" | "hh" | "hxx" | "inc" | "ipp"))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // Local copy: cmake_api has an identical helper for its own tests. Two
    // small builders beat exporting test scaffolding across a module
    // boundary, which would make one module's tests able to break the other's.
    fn library_target(name: &str, includes: Vec<String>) -> Target {
        Target {
            name: name.to_string(),
            kind: TargetKind::Library,
            sources: Vec::new(),
            public_headers: Vec::new(),
            dependencies: Vec::new(),
            includes,
            local_defines: Vec::new(),
            artifacts: Vec::new(),
            ..Default::default()
        }
    }

    // bzl-htv: -Iinclude with #include "sub/foo.h" is ordinary C, and a flat
    // listing of include/ misses include/sub/foo.h entirely.
    #[test]
    fn a_header_in_a_subdirectory_of_an_include_dir_is_found() {
        let dir =
            std::env::temp_dir().join(format!("bzlf_hdrsub_{}_{}", std::process::id(), line!()));
        let inc = dir.join("include");
        let nested = inc.join("proj").join("detail");
        fs::create_dir_all(&nested).unwrap();
        fs::write(dir.join("app.c"), b"int main(void){return 0;}\n").unwrap();
        fs::write(inc.join("top.h"), b"#pragma once\n").unwrap();
        fs::write(inc.join("proj").join("mid.h"), b"#pragma once\n").unwrap();
        fs::write(nested.join("deep.h"), b"#pragma once\n").unwrap();
        // A non-header in the tree must not be swept in at any depth.
        fs::write(nested.join("notes.txt"), b"x\n").unwrap();
        let src = dir.to_string_lossy().into_owned();

        let mut targets = vec![{
            let mut t = library_target("app", vec![inc.to_string_lossy().into_owned()]);
            t.kind = TargetKind::Executable;
            t.sources.push("app.c".to_string());
            t
        }];

        inject_headers_on_include_dirs(&mut targets, Path::new(&src));

        assert_eq!(
            targets[0].sources,
            vec![
                "app.c".to_string(),
                "include/proj/detail/deep.h".to_string(),
                "include/proj/mid.h".to_string(),
                "include/top.h".to_string(),
            ],
            "every header at or below the include dir must be staged, at any \
             depth, and nothing that isn't a header"
        );
    }

    // bzl-6jn: the two predicates deliberately disagree, and unifying them
    // would silently change behavior. Pinned in BOTH directions so neither
    // "tidy up the duplicate" nor "make the escalation generous" passes
    // unnoticed. The rationale lives in the module doc; this is the check.
    #[test]
    fn the_staging_predicate_is_broader_than_the_escalation_predicate() {
        for shared in ["a.h", "a.hpp", "a.hh", "a.hxx"] {
            assert!(
                looks_like_header(shared) && is_header_file(Path::new(shared)),
                "{shared} is a header to both predicates"
            );
        }

        for inclusion in ["a.inc", "a.ipp"] {
            assert!(
                is_header_file(Path::new(inclusion)),
                "{inclusion} must STAGE: an unread staged file costs a symlink"
            );
            assert!(
                !looks_like_header(inclusion),
                "{inclusion} must NOT drive the header-visibility escalation: \
                 these are usually #included bodies, not interface headers, so \
                 treating one as an unclassified header raises a needs_attention \
                 item for a non-gap and burns an agent cycle"
            );
        }

        for neither in ["a.c", "a.cpp", "a.txt"] {
            assert!(
                !looks_like_header(neither) && !is_header_file(Path::new(neither)),
                "{neither} is a header to neither predicate"
            );
        }
    }

    #[test]
    fn is_include_destination_matches_include_dirs_only() {
        assert!(is_include_destination("include"));
        assert!(is_include_destination("include/tinyxml2"));
        // The absolute form CMAKE_INSTALL_FULL_INCLUDEDIR expands to — what
        // json-c installs its public headers to (bzl-fxa.14).
        assert!(is_include_destination("/usr/local/include/json-c"));
        assert!(is_include_destination("/usr/include"));
        assert!(is_include_destination("/opt/thing/include/sub"));
        // Not a header destination — these are where a target install, a
        // pkgconfig file, and cmake package files land.
        assert!(!is_include_destination("lib"));
        assert!(!is_include_destination("lib/pkgconfig"));
        assert!(!is_include_destination("lib/cmake/tinyxml2"));
        assert!(!is_include_destination("share/doc"));
        assert!(!is_include_destination("/usr/local/lib/cmake/json-c"));
        // A build-private include tree nested under lib/lib64/share is not the
        // public install location — only a real include prefix counts.
        assert!(!is_include_destination("lib/include"));
        assert!(!is_include_destination("/usr/local/lib/include"));
        assert!(!is_include_destination("share/include"));
    }

    // bzl-fxa.18: json-c's `test1` lists one .c and reaches parse_flags.h
    // purely through its own include dir. CMake finds it on disk; Bazel stages
    // only declared inputs, so it must become a source.
    #[test]
    fn headers_on_a_targets_include_dirs_are_added_to_its_sources() {
        let dir =
            std::env::temp_dir().join(format!("bzlf_incdir_{}_{}", std::process::id(), line!()));
        let tests_dir = dir.join("tests");
        fs::create_dir_all(&tests_dir).unwrap();
        fs::write(tests_dir.join("test1.c"), b"int main(void){return 0;}\n").unwrap();
        fs::write(tests_dir.join("parse_flags.h"), b"#pragma once\n").unwrap();
        // A .c in the same directory: it is another target's source, and a
        // header injection must not sweep it in.
        fs::write(tests_dir.join("parse_flags.c"), b"int f(void){return 0;}\n").unwrap();
        let src = dir.to_string_lossy().into_owned();

        let mut targets = vec![{
            let mut t = library_target("test1", vec![tests_dir.to_string_lossy().into_owned()]);
            t.kind = TargetKind::Executable;
            t.sources.push("tests/test1.c".to_string());
            t
        }];

        inject_headers_on_include_dirs(&mut targets, Path::new(&src));

        assert_eq!(
            targets[0].sources,
            vec![
                "tests/test1.c".to_string(),
                "tests/parse_flags.h".to_string()
            ],
            "every header on the include path is an input (CMake's own semantic), but a \
             .c file there belongs to some other target and must not be swept in"
        );
    }

    // The dedup must be PER-TARGET. Asking "does any target list this header?"
    // suppresses it exactly where it's missing — json-c's test1Formatted lists
    // parse_flags.h while its sibling test1, same include dir, does not.
    #[test]
    fn a_header_enumerated_on_a_sibling_target_is_still_injected_here() {
        let dir =
            std::env::temp_dir().join(format!("bzlf_incdir_{}_{}", std::process::id(), line!()));
        let tests_dir = dir.join("tests");
        fs::create_dir_all(&tests_dir).unwrap();
        fs::write(tests_dir.join("test1.c"), b"int main(void){return 0;}\n").unwrap();
        fs::write(tests_dir.join("parse_flags.h"), b"#pragma once\n").unwrap();
        let src = dir.to_string_lossy().into_owned();
        let include = tests_dir.to_string_lossy().into_owned();

        let mut targets = vec![
            {
                let mut t = library_target("test1", vec![include.clone()]);
                t.kind = TargetKind::Executable;
                t.sources.push("tests/test1.c".to_string());
                t
            },
            {
                let mut t = library_target("test1Formatted", vec![include]);
                t.kind = TargetKind::Executable;
                t.sources.push("tests/test1.c".to_string());
                // CMake enumerated it here, and only here.
                t.sources.push("tests/parse_flags.h".to_string());
                t
            },
        ];

        inject_headers_on_include_dirs(&mut targets, Path::new(&src));

        let by_name = |n: &str| targets.iter().find(|t| t.name == n).unwrap();
        assert!(
            by_name("test1")
                .sources
                .contains(&"tests/parse_flags.h".to_string()),
            "the sibling listing it must not suppress it here:\n{:?}",
            by_name("test1").sources
        );
        assert_eq!(
            by_name("test1Formatted")
                .sources
                .iter()
                .filter(|s| *s == "tests/parse_flags.h")
                .count(),
            1,
            "and the target that DID enumerate it must not list it twice:\n{:?}",
            by_name("test1Formatted").sources
        );
    }

    #[test]
    fn an_include_dir_outside_the_source_tree_contributes_nothing() {
        let dir =
            std::env::temp_dir().join(format!("bzlf_incdir_{}_{}", std::process::id(), line!()));
        let src_root = dir.join("proj");
        let build_dir = dir.join("build");
        fs::create_dir_all(&src_root).unwrap();
        fs::create_dir_all(&build_dir).unwrap();
        fs::write(src_root.join("app.c"), b"int main(void){return 0;}\n").unwrap();
        // A generated header in the build tree: reproduced by the
        // config_header machinery, never copied.
        fs::write(build_dir.join("config.h"), b"#pragma once\n").unwrap();
        let src = src_root.to_string_lossy().into_owned();

        let mut targets = vec![{
            let mut t = library_target("app", vec![build_dir.to_string_lossy().into_owned()]);
            t.kind = TargetKind::Executable;
            t.sources.push("app.c".to_string());
            t
        }];

        inject_headers_on_include_dirs(&mut targets, Path::new(&src));

        assert_eq!(
            targets[0].sources,
            vec!["app.c".to_string()],
            "a build-dir include path must contribute nothing — its headers are \
             reproduced by config_header, and the module may not carry them"
        );
    }

    #[test]
    fn inject_unenumerated_installed_headers_adds_them_to_libraries_on_the_include_path() {
        // bzl-fxa.10: a public header the project install()s but that no target
        // enumerated (json-c's json_pointer.h — dropped from the library's
        // header list by a CMakeLists ordering bug, yet #included by its .c and
        // install()d as public). It must be injected as a public header on the
        // libraries whose include path it sits on, so it's copied and reachable.
        let dir =
            std::env::temp_dir().join(format!("bzlf_inject_{}_{}", std::process::id(), line!()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("json_pointer.h"), b"#pragma once\n").unwrap();
        fs::write(dir.join("json_object.h"), b"#pragma once\n").unwrap();
        let src = dir.to_string_lossy().into_owned();

        // Two libraries on the same include dir (shared + static, as json-c
        // has), plus an executable that must NOT receive the header, plus a
        // library whose include dir does not contain it.
        let mut targets = vec![
            {
                let mut t = library_target("json-c", vec![src.clone()]);
                // json_object.h is enumerated AND install-declared -> already
                // public via to_target; must not be injected a second time.
                t.public_headers.push("json_object.h".to_string());
                t.sources.push("json_object.c".to_string());
                t
            },
            library_target("json-c-static", vec![src.clone()]),
            {
                let mut t = library_target("elsewhere", vec!["/other/include".to_string()]);
                t.kind = TargetKind::Library;
                t
            },
            {
                let mut t = library_target("app", vec![src.clone()]);
                t.kind = TargetKind::Executable;
                t
            },
        ];

        let installed: HashSet<String> =
            ["json_pointer.h".to_string(), "json_object.h".to_string()]
                .into_iter()
                .collect();

        inject_unenumerated_installed_headers(&mut targets, &installed, Path::new(&src));

        let by_name = |n: &str| targets.iter().find(|t| t.name == n).unwrap();
        assert_eq!(
            by_name("json-c").public_headers,
            vec!["json_object.h".to_string(), "json_pointer.h".to_string()],
            "the unenumerated install-declared header is appended; the enumerated one is not duplicated"
        );
        assert_eq!(
            by_name("json-c-static").public_headers,
            vec!["json_pointer.h".to_string()],
            "the sibling library on the same include path also gets it"
        );
        assert!(
            by_name("elsewhere").public_headers.is_empty(),
            "a library whose include dirs don't contain the header is not given it"
        );
        assert!(
            by_name("app").public_headers.is_empty(),
            "an executable is never given injected headers (cc_binary has no hdrs)"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inject_unenumerated_installed_headers_skips_out_of_tree_and_missing() {
        // A generated header installed by absolute build-dir path (json-c's
        // json.h) resolves outside source_dir and must be skipped — it's
        // reproduced by the config_header machinery, not copied. A declared
        // header that doesn't exist on disk is skipped too (copying it would
        // hard-fail).
        let dir = std::env::temp_dir().join(format!(
            "bzlf_inject_skip_{}_{}",
            std::process::id(),
            line!()
        ));
        fs::create_dir_all(&dir).unwrap();
        let src = dir.to_string_lossy().into_owned();
        let mut targets = vec![library_target("lib", vec![src.clone()])];

        let installed: HashSet<String> = [
            "/some/build/json.h".to_string(), // absolute, outside source_dir
            "phantom.h".to_string(),          // under source_dir but not on disk
        ]
        .into_iter()
        .collect();

        inject_unenumerated_installed_headers(&mut targets, &installed, Path::new(&src));

        assert!(
            targets[0].public_headers.is_empty(),
            "neither an out-of-tree generated header nor a nonexistent one is injected"
        );

        fs::remove_dir_all(&dir).ok();
    }
}
