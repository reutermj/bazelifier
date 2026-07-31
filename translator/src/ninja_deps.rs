//! Ninja's dependency database: which files each translation unit actually
//! opened, per target.
//!
//! Separate from `cmake_api` because it reads a **fourth source**, and the
//! only one that is the *compiler's* answer rather than CMake's. The File API
//! reports what the project DECLARED; `ninja -t deps` reports what the
//! preprocessor actually read, resolved against the real include search path.
//! That difference is the whole value: it is correct on `#if`, computed
//! includes, macro-built paths, the quoted-include rule's
//! search-the-including-file's-directory-first behavior, and a source
//! textually `#include`d rather than compiled — none of which appear in any
//! CMake reply.
//!
//! **This is not the `#include`-scanning approach rejected in 2343e0c.** That
//! one parsed source text and re-implemented a preprocessor badly. This
//! consumes build output, the same category as the File API, and is correct
//! by construction because the compiler produced it. See
//! docs/lore/everything-on-the-include-path-is-an-input.md.
//!
//! Two limits, and the second is the one that matters:
//!
//! - **Ninja-only.** The database is Ninja's, and `convert_cmake_project`
//!   already pins `-G Ninja`. A project built with another generator yields
//!   nothing here and falls back on the declared include paths alone.
//! - **It reflects ONE configuration.** A header behind a disabled `#ifdef`
//!   was never opened, so it is absent — yet it is still an input for a
//!   consumer who configures differently. So this **supplements** the
//!   include-directory walk in `headers`; it must never replace it, or the
//!   converted module would silently carry only the subset this machine's
//!   configuration happened to reach.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::paths::normalize_lexically;

/// Files each target's compilations opened, keyed by CMake target name and
/// bounded to `source_dir`.
///
/// Everything outside the source tree is dropped here rather than by the
/// caller: the raw database is dominated by `/usr/include` and toolchain
/// headers, which must never enter a converted module, and a build-directory
/// path is a generated file the module reproduces by other means.
///
/// A failed or missing `ninja` yields an empty map rather than an error —
/// this is an enrichment, and a project whose deps cannot be read still
/// converts on its declared include paths.
pub(crate) fn opened_files_by_target(
    build_dir: &Path,
    source_dir: &Path,
    build_prefixes: &HashMap<String, String>,
) -> HashMap<String, Vec<String>> {
    let Ok(output) = Command::new("ninja")
        .arg("-t")
        .arg("deps")
        .current_dir(build_dir)
        .output()
    else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }

    parse_deps(
        &String::from_utf8_lossy(&output.stdout),
        source_dir,
        build_prefixes,
    )
}

/// Splits `ninja -t deps` output into per-target file lists.
///
/// Split from `opened_files_by_target` so a frozen real capture can drive it
/// without a build. The format is a header line naming an object file, then
/// indented paths:
///
/// ```text
/// test/CMakeFiles/util.dir/util.cc.o: #deps 5, deps mtime 1785 (VALID)
///     /abs/src/test/util.cc
///     /abs/src/test/util.h
/// ```
fn parse_deps(
    stdout: &str,
    source_dir: &Path,
    build_prefixes: &HashMap<String, String>,
) -> HashMap<String, Vec<String>> {
    let mut by_target: HashMap<String, Vec<String>> = HashMap::new();
    let mut current: Option<&str> = None;

    for line in stdout.lines() {
        if let Some(object) = line.split_once(": #deps ").map(|(o, _)| o) {
            // A header line: resolve which emitted target owns this object.
            current = target_for_object(object, build_prefixes);
            continue;
        }
        if !line.starts_with(' ') {
            // A blank line, or anything unrecognized, ends the current entry
            // rather than silently attributing its files to the last target.
            current = None;
            continue;
        }
        let Some(target) = current else { continue };

        let path = normalize_lexically(Path::new(line.trim()));
        // Bounded to the source tree: toolchain headers must never enter a
        // module, and a build-dir path is a generated file reproduced by
        // other means (config_header) or not reproducible at all.
        let Ok(relative) = path.strip_prefix(source_dir) else {
            continue;
        };
        let relative = relative.to_string_lossy().into_owned();
        let files = by_target.entry(target.to_string()).or_default();
        if !files.contains(&relative) {
            files.push(relative);
        }
    }

    for files in by_target.values_mut() {
        // read order is Ninja's; the generated srcs must not vary between runs.
        files.sort();
    }
    by_target
}

/// The emitted target owning `object`, or `None` when no target claims it.
///
/// CMake names object files `<target build dir>/CMakeFiles/<target>.dir/...`.
/// The prefix is DERIVED from each target's own `paths.build` (File API)
/// rather than recovered by pattern-matching the object path, so the mapping
/// rests on a value CMake reports instead of on a layout convention. An
/// object no prefix claims — CMake's own bookkeeping targets, another
/// generator's layout — is skipped rather than guessed at.
fn target_for_object<'a>(
    object: &str,
    build_prefixes: &'a HashMap<String, String>,
) -> Option<&'a str> {
    build_prefixes
        .iter()
        .filter(|(_, prefix)| object.starts_with(prefix.as_str()))
        // Longest prefix wins: a target in a subdirectory has a strictly
        // longer prefix than one at the root, and both can match.
        .max_by_key(|(_, prefix)| prefix.len())
        .map(|(name, _)| name.as_str())
}

/// The object-file prefix CMake uses for a target, from its `paths.build`.
pub(crate) fn build_prefix(target_name: &str, build_path: &str) -> String {
    let dir = if build_path == "." || build_path.is_empty() {
        String::new()
    } else {
        format!("{build_path}/")
    };
    format!("{dir}CMakeFiles/{target_name}.dir/")
}
