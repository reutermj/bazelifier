//! `configure_file` handling: recovers the config headers a project
//! generates and plans their reproduction as `@cc_config//:config_header`
//! rules.
//!
//! Separate from `cmake_api` because it reads a **different input, in a
//! different way**. `configure_file` calls appear in no File API reply at
//! all — the outputs are headers, not compiled units, so nothing reports
//! them and a target's dependence on one shows up only as an include path.
//! They have to be recovered from `cmake --trace-expand` output, and the
//! templates then read off disk. That makes this the one part of the
//! frontend that parses text rather than consuming a structured reply — a
//! deliberate exception to the rule stated in `cmake_api`'s own module doc,
//! and one worth keeping visibly separate from the code that follows it.
//! See docs/lore/cmake-configure-file-is-in-the-trace-not-the-file-api.md
//! and docs/architecture/configure-file-and-toolchain-probes.md.
//!
//! Nothing here reaches back into `cmake_api`: this module imports no part
//! of it — only the shared `model`, `needs_attention` and `paths` — and
//! every entry point is a pure function of its arguments.
//! `cmake_api::discover` is the driver: it calls down, gets values back, and
//! sequences them. Two consequences of that sequencing are worth knowing
//! before editing either side:
//!
//! - [`is_config_header_output`] is also called by the driver, to build the
//!   set of generated header *names* before the codemodel is read.
//!   `rebase_to_module_root` needs it to tell a build-dir source that a
//!   `config_header` rule reproduces (drop it — the rule supplies it via a
//!   label) from one nothing reproduces (escalate). Without that the same
//!   header would be both escalated and regenerated.
//! - [`build_config_headers`] has to run *after* the codemodel is read: it
//!   rebases template paths against the module root, and deriving that root
//!   is part of the codemodel walk. A data dependency in the driver, not a
//!   coupling between the modules.
//!
//! The `@cc_config//catalog:` labels in a plan's `catalog_probes` are built
//! here rather than in codegen, which is a layering wart — see bzl-689.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::headers::is_config_header_output;
use crate::model::ConfigHeader;
use crate::needs_attention::{
    ConfigDialect, NeedsAttention, generated_config_header_needs_attention,
    unmapped_config_macros_needs_attention,
};
use crate::paths::normalize_lexically;

/// Builds the `config_header` plans for a project's `configure_file` calls,
/// reading and parsing each template. Returns the plans and escalations for
/// any header the translator couldn't fully resolve (an unmapped
/// `#cmakedefine`, or a template that reaches outside the module root).
///
/// A template's location decides how it is handled, in three cases: one
/// already in the source tree is used where it sits; one the project generated
/// into the BUILD directory is staged into the module and planned against the
/// staged copy (see `ConfigHeader::template_source`); one anywhere else is
/// genuinely unreachable and escalates, the same boundary
/// `rebase_to_module_root` enforces for sources.
pub(crate) fn build_config_headers(
    configure_files: &[ConfigureFile],
    module_root: &Path,
    build_dir: &Path,
    cache: &HashMap<String, String>,
) -> (Vec<ConfigHeader>, Vec<NeedsAttention>) {
    let mut headers = Vec::new();
    let mut escalations = Vec::new();
    for call in configure_files {
        // configure_file also generates non-headers — pkg-config (.pc), CMake
        // package files (.cmake), CTest config (.tcl). Those are not headers a
        // C source compiles against, so the translator does not reproduce them
        // as a config_header; they're install/packaging artifacts a converted
        // module omits (like it omits install() rules). Only header outputs
        // continue.
        if !is_config_header_output(&call.output) {
            continue;
        }
        // Where the template will live inside the module, and where to copy it
        // from if it is not there already.
        let staged = match call.template.strip_prefix(module_root) {
            // The common case: a checked-in .in/.cmakein, already staged with
            // the other sources.
            Ok(relative) => Some((relative.to_path_buf(), None)),
            // Generated into the build directory at configure time, then
            // expanded — zlib assembles zconf.h.cmakein with
            // file(READ/WRITE/APPEND). The translator cannot reproduce that
            // assembly, but it does not need to: the file exists on disk now
            // and carries only #cmakedefine directives, so staging it and
            // expanding it against the CONSUMER's toolchain reproduces what
            // CMake did. Leaving it unresolved instead would ship a header with
            // no feature detection at all, which silently picks the project's
            // fallback code paths on any toolchain but this one.
            //
            // NOT the same as vendoring the generated HEADER, which stays
            // forbidden: that carries this host's probe answers.
            Err(_) if call.template.starts_with(build_dir) => call
                .template
                .file_name()
                .map(|name| (PathBuf::from(name), Some(call.template.clone()))),
            // Genuinely outside the deliverable — a system path, another
            // checkout. Nothing to stage.
            Err(_) => None,
        };
        let Some((template_rel, template_source)) = staged else {
            escalations.push(generated_config_header_needs_attention(
                &call.output.to_string_lossy(),
                &[call.template.to_string_lossy().into_owned()],
            ));
            continue;
        };
        let template_rel = template_rel.as_path();
        let Ok(template_text) = fs::read_to_string(&call.template) else {
            continue;
        };
        let macros = parse_template_macros(&template_text);
        let output_name = call
            .output
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| call.output.to_string_lossy().into_owned());

        let template_rel = template_rel.to_string_lossy().into_owned();
        let (mut header, unmapped) =
            resolve_config_header(&template_rel, &output_name, &macros, cache);
        header.template_source = template_source;
        if !unmapped.is_empty() {
            // The header IS reproduced (the probing module and template
            // wiring exist); the gap is specific macros with no catalog
            // probe — a different escalation from a header we can't reach.
            escalations.push(unmapped_config_macros_needs_attention(
                &output_name,
                &template_rel,
                &unmapped,
                ConfigDialect::CMake,
            ));
        }
        headers.push(header);
    }
    (headers, escalations)
}

/// One `configure_file(<template> <output> ...)` call the project itself made,
/// recovered from the configure trace with both paths resolved absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigureFile {
    pub(crate) template: PathBuf,
    pub(crate) output: PathBuf,
}

/// The macros a `configure_file` template references: `#cmakedefine` names
/// (each becomes a `#define`/`#undef` in the output, driven by whether the
/// name is set) and `@VAR@` names (each substituted with a value). A name can
/// be both — `#cmakedefine FOO @FOO@` — which is how CMake writes a define
/// whose value is a variable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct TemplateMacros {
    cmakedefines: Vec<String>,
    vars: Vec<String>,
}

/// The macros the shared `cc_config` catalog provides a probe for. This is a
/// hand-maintained copy of the catalog's define set (declared in Starlark in
/// `cc_config/catalog/BUILD.bazel`, which this Rust cannot read at codegen
/// time). The two lists are kept in sync by hand — the facts are stable
/// autoconf checks that rarely change — and the `//:catalog_sync_check` test
/// fails if they diverge, in either direction. Editing this list without the
/// catalog is the worse direction: the translator then emits a
/// `@cc_config//catalog:` label for a probe that does not exist, and the
/// generated module dies at analysis time in the unpacked workspace.
///
/// A template `#cmakedefine` naming one of these maps to
/// `@cc_config//catalog:<name lowercased>`; anything else the translator
/// cannot confidently resolve and escalates. Exact match only: a project that
/// prefixes its macros (json-c's `JSON_C_HAVE_INTTYPES_H`) does not match
/// `HAVE_INTTYPES_H` and is escalated rather than guessed.
const CATALOG_DEFINES: &[&str] = &[
    "HAVE_ARC4RANDOM",
    "HAVE_BSD_STDLIB_H",
    "HAVE_BYTESWAP_H",
    "HAVE_CLOCK_GETTIME",
    "HAVE_CPUID_H",
    "HAVE_DLFCN_H",
    "HAVE_DUPLOCALE",
    "HAVE_ENDIAN_H",
    "HAVE_FCNTL_H",
    "HAVE_FUTIMENS",
    "HAVE_GETOPT_H",
    "HAVE_GETOPT_LONG",
    "HAVE_GETRANDOM",
    "HAVE_GETRUSAGE",
    "HAVE_IMMINTRIN_H",
    "HAVE_INTTYPES_H",
    "HAVE_LIMITS_H",
    "HAVE_LOCALE_H",
    "HAVE_MBRTOWC",
    "HAVE_MEMORY_H",
    "HAVE_OPEN",
    "HAVE_POSIX_FADVISE",
    "HAVE_REALLOC",
    "HAVE_SETLOCALE",
    "HAVE_SETRLIMIT",
    "HAVE_SNPRINTF",
    "HAVE_STDARG_H",
    "HAVE_STDBOOL_H",
    "HAVE_STDINT_H",
    "HAVE_STDIO_H",
    "HAVE_STDLIB_H",
    "HAVE_STRCASECMP",
    "HAVE_STRDUP",
    "HAVE_STRERROR",
    "HAVE_STRING_H",
    "HAVE_STRINGS_H",
    "HAVE_STRNCASECMP",
    "HAVE_STRTOLL",
    "HAVE_STRTOULL",
    "HAVE_ARPA_INET_H",
    "HAVE_ERRNO_H",
    "HAVE_NET_IF_H",
    "HAVE_NETDB_H",
    "HAVE_NETINET_IN_H",
    "HAVE_NETINET_IP_H",
    "HAVE_NETINET_TCP_H",
    "HAVE_POLL_H",
    "HAVE_PTHREAD_H",
    "HAVE_SCHED_H",
    "HAVE_SIGNAL_H",
    "HAVE_STDALIGN_H",
    "HAVE_STDDEF_H",
    "HAVE_TIME_H",
    "HAVE_SYS_IOCTL_H",
    "HAVE_SYS_MMAN_H",
    "HAVE_SYS_MSG_H",
    "HAVE_SYS_SELECT_H",
    "HAVE_SYS_SOCKET_H",
    "HAVE_SYS_UIO_H",
    "HAVE_SYS_CDEFS_H",
    "HAVE_SYS_PARAM_H",
    "HAVE_SYS_RANDOM_H",
    "HAVE_SYS_RESOURCE_H",
    "HAVE_SYS_STAT_H",
    "HAVE_SYS_TIME_H",
    "HAVE_SYS_TYPES_H",
    "HAVE_SYSLOG_H",
    "HAVE_UNISTD_H",
    "HAVE_USELOCALE",
    "HAVE_USELOCALE",
    "HAVE_VASPRINTF",
    "HAVE_VPRINTF",
    "HAVE_VSNPRINTF",
    "HAVE_VSYSLOG",
    "HAVE_WCHAR_H",
    "HAVE_XLOCALE_H",
    "STDC_HEADERS",
    "SIZEOF_INT",
    "SIZEOF_INT64_T",
    "SIZEOF_LONG",
    "SIZEOF_LONG_LONG",
    "SIZEOF_SIZE_T",
    "SIZEOF_SSIZE_T",
];

/// The `@cc_config//catalog:<target>` label for a catalog define, or `None`
/// if the define isn't in the catalog. The target name is the define
/// lowercased — the convention `cc_config_catalog` uses (see catalog.bzl).
pub(crate) fn catalog_label(define: &str) -> Option<String> {
    CATALOG_DEFINES
        .contains(&define)
        .then(|| format!("@cc_config//catalog:{}", define.to_lowercase()))
}

/// Parses a `configure_file` template's text for the macros it references,
/// each deduplicated in first-seen order. The translator reads this to learn
/// which config values a generated header needs — the File API and trace give
/// the template/output pair but not its contents.
/// Every `@VAR@` name a template substitutes, deduplicated in first-seen
/// order.
///
/// Exposed for the Autotools frontend's `AC_CONFIG_FILES` headers, which use
/// this dialect and no other. Shared rather than reimplemented because the
/// `@@`-escape and identifier-character rules are exactly the same question,
/// and two answers to it would drift.
pub(crate) fn substituted_names(template: &str) -> Vec<String> {
    parse_template_macros(template).vars
}

fn parse_template_macros(template: &str) -> TemplateMacros {
    let mut cmakedefines = Vec::new();
    let mut seen_define = HashSet::new();
    let mut vars = Vec::new();
    let mut seen_var = HashSet::new();

    for line in template.lines() {
        let trimmed = line.trim_start();
        // `#cmakedefine NAME ...` or `#cmakedefine01 NAME`. The name is the
        // token after the directive.
        for directive in ["#cmakedefine01", "#cmakedefine"] {
            if let Some(rest) = trimmed.strip_prefix(directive) {
                if let Some(name) = rest.split_whitespace().next()
                    && seen_define.insert(name.to_string())
                {
                    cmakedefines.push(name.to_string());
                }
                break;
            }
        }

        // `@VAR@` anywhere. `@@` (a CMake escape for a literal `@`) yields an
        // empty name, which is skipped; a token with non-identifier
        // characters between the `@`s is not a variable reference.
        let mut i = 0;
        while let Some(at) = line[i..].find('@') {
            let start = i + at + 1;
            let Some(end_rel) = line[start..].find('@') else {
                break;
            };
            let name = &line[start..start + end_rel];
            if !name.is_empty()
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && seen_var.insert(name.to_string())
            {
                vars.push(name.to_string());
            }
            i = start + end_rel + 1;
        }
    }

    TemplateMacros { cmakedefines, vars }
}

/// Extracts the project's own `configure_file` calls from a `--trace-expand`
/// trace, keeping only the project's own.
///
/// CMake makes ~10 internal `configure_file` calls of its own (from
/// CMakeSystem.cmake.in, DartConfiguration.tcl.in, CPack templates, ...);
/// those originate under the CMake installation. Filtering on the template
/// being under `source_dir` OR `build_dir` selects exactly the project's: the
/// build directory counts because a project can generate a template at
/// configure time and then expand it (zlib assembles `zconf.h.cmakein`), and
/// that call is as much its own as a checked-in one.
///
/// Trace lines look like `<file>(<line>):  configure_file(<in> <out> <flags>)`.
/// A path argument may be absolute (json-c writes
/// `${PROJECT_SOURCE_DIR}/cmake/config.h.in`, which expands absolute) or
/// relative (`configure_file(config.h.in config.h)` — the common form); a
/// relative path is resolved against the calling file's own directory, taken
/// from the `<file>` site prefix, matching CMake's own resolution.
pub(crate) fn parse_configure_files(
    trace: &str,
    source_dir: &Path,
    build_dir: &Path,
) -> Vec<ConfigureFile> {
    let mut calls = Vec::new();
    for line in trace.lines() {
        let Some(idx) = line.find("configure_file(") else {
            continue;
        };

        // The site directory is the dir of the file named before "(<line>):".
        let site_dir = line[..idx]
            .rfind("):")
            .and_then(|paren| line[..paren].rfind('('))
            .map(|open| Path::new(&line[..open]))
            .and_then(Path::parent);

        let args = &line[idx + "configure_file(".len()..];
        let args = args.trim_end().trim_end_matches(')').trim();
        // First token is the template, second the output. Trailing flags
        // (@ONLY, COPYONLY, ...) are ignored.
        let mut tokens = args.split_whitespace();
        let (Some(template), Some(output)) = (tokens.next(), tokens.next()) else {
            continue;
        };

        let template = resolve_trace_path(template, site_dir);
        // Keep the project's own calls and drop CMake's internal ones (whose
        // templates live under the CMake installation). The BUILD directory
        // counts as the project's: a project can generate a template at
        // configure time and then expand it (zlib assembles zconf.h.cmakein),
        // and that call is as much the project's own as a checked-in one —
        // dropping it here silently loses the whole config header, before any
        // of the reachability logic downstream gets a say.
        if !template.starts_with(source_dir) && !template.starts_with(build_dir) {
            continue;
        }
        calls.push(ConfigureFile {
            template,
            output: resolve_trace_path(output, site_dir),
        });
    }
    calls
}

/// Resolves a `configure_file` path argument from the trace: absolute as-is,
/// relative against the calling file's directory (`site_dir`). Normalized so
/// the `starts_with(source_dir)` filter and later `strip_prefix` are exact.
fn resolve_trace_path(path: &str, site_dir: Option<&Path>) -> PathBuf {
    let path = Path::new(path);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match site_dir {
            Some(dir) => dir.join(path),
            None => path.to_path_buf(),
        }
    };
    normalize_lexically(&joined)
}

/// Looks a template variable up in the CMake cache, with a fallback for the
/// handful of common variables `project()` sets as plain variables that the
/// cache records under a `CMAKE_`-prefixed name: `@PROJECT_VERSION@` (and its
/// `_MAJOR`/`_MINOR`/`_PATCH` variants) live in the cache as
/// `CMAKE_PROJECT_VERSION*`. CMake variables and cache entries are different
/// namespaces; this covers the version family, which real config headers
/// reference constantly, without a general variable-resolution pass.
fn cache_value(name: &str, cache: &HashMap<String, String>) -> Option<String> {
    if let Some(v) = cache.get(name) {
        return Some(v.clone());
    }
    if let Some(rest) = name.strip_prefix("PROJECT_VERSION") {
        // PROJECT_VERSION{,_MAJOR,_MINOR,_PATCH} -> CMAKE_PROJECT_VERSION{...}
        return cache.get(&format!("CMAKE_PROJECT_VERSION{rest}")).cloned();
    }
    None
}

/// Turns a `configure_file` call into a plan for a `config_header` rule: maps
/// each `#cmakedefine` to a catalog probe, resolves `@VAR@`s from the cache,
/// and returns the plan alongside the `#cmakedefine`s the catalog does not
/// cover (which the caller escalates rather than emitting a header that would
/// be missing defines). `call_template_relative` and `output_relative` are
/// already module-relative.
///
/// A name a template references (as a `#cmakedefine` or an `@VAR@`) is
/// resolved by, in order:
///
/// - a **catalog probe** if it is a catalog fact — whether it appears as a
///   `#cmakedefine` or an `@VAR@`. The probe's result feeds BOTH the
///   `#cmakedefine` resolution and the `@VAR@` value in the config_header
///   expander, so a catalog fact is NEVER taken from the cache — that would
///   bake in this conversion host's answer (`SIZEOF_LONG=8`, `HAVE_X=1`)
///   instead of the consumer's.
/// - a **cache value** if it is a non-catalog `@VAR@` present in the cache
///   (a version string, an option) — these are toolchain-independent, so the
///   host's value is the right one.
/// - **unmapped** if it is a `#cmakedefine` that is neither a catalog fact nor
///   a cache value; the caller escalates it.
fn resolve_config_header(
    call_template_relative: &str,
    output_relative: &str,
    macros: &TemplateMacros,
    cache: &HashMap<String, String>,
) -> (ConfigHeader, Vec<String>) {
    // Catalog probes for every catalog fact the template names, by either
    // form, deduped. A var and a #cmakedefine of the same name share one probe.
    let mut catalog_probes = Vec::new();
    let mut probed = HashSet::new();
    for name in macros.cmakedefines.iter().chain(macros.vars.iter()) {
        if let Some(label) = catalog_label(name)
            && probed.insert(name.clone())
        {
            catalog_probes.push(label);
        }
    }

    // A name that is neither a catalog fact nor a cache value can't be
    // resolved — escalate it. This covers both forms:
    //   - a #cmakedefine (a cache-covered one is an option whose value drives
    //     the define; its value is added below), and
    //   - a plain @VAR@ (json.h.cmakein gates umbrella includes on
    //     @JSON_H_JSON_PATCH@ etc.). An unresolved @VAR@ is left LITERAL in the
    //     output, breaking every source that includes the header — so it must
    //     escalate, not slip through as a silent build breaker (bzl-fxa.9).
    // A name that is both (`#cmakedefine FOO @FOO@`) is escalated once: the
    // #cmakedefine pass adds it, and the @VAR@ pass skips what's already there.
    let mut unmapped = Vec::new();
    let mut escalated = HashSet::new();
    for name in macros.cmakedefines.iter().chain(macros.vars.iter()) {
        if catalog_label(name).is_none()
            && cache_value(name, cache).is_none()
            && escalated.insert(name.clone())
        {
            unmapped.push(name.clone());
        }
    }

    // Cache values for non-catalog names present in the cache. Catalog facts
    // are excluded — their value comes from the probe, not the host cache.
    let mut values = Vec::new();
    let mut valued = HashSet::new();
    for name in macros.vars.iter().chain(macros.cmakedefines.iter()) {
        if catalog_label(name).is_some() || valued.contains(name) {
            continue;
        }
        if let Some(value) = cache_value(name, cache) {
            valued.insert(name.clone());
            values.push((name.clone(), value));
        }
    }

    let header = ConfigHeader {
        output: output_relative.to_string(),
        template: call_template_relative.to_string(),
        template_source: None,
        catalog_probes,
        values,
        // Read off the template rather than assumed from the frontend: a
        // CMake `configure_file` target may be pure `@VAR@` substitution with
        // no `#cmakedefine` in it, and asserting the absence of a construct
        // the template never had is a gate that cannot fail.
        dialect: if macros.cmakedefines.is_empty() {
            crate::model::ConfigDialect::Substitution
        } else {
            crate::model::ConfigDialect::Cmakedefine
        },
        // A `configure_file` generates the project's own header, never a
        // replacement for a system one — CMake has no gnulib analogue.
        shadow_dir: None,
    };
    (header, unmapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `cmake --trace-expand` lines, captured verbatim from configuring
    // json-c (paths shortened). Mixes the project's own configure_file calls
    // (templates under /src/...) with CMake's internal ones (templates under
    // the CMake installation), exactly as the trace does — see
    // docs/lore/cmake-configure-file-is-in-the-trace-not-the-file-api.md.
    const CONFIGURE_TRACE: &str = "\
/usr/share/cmake-3.28/Modules/CMakeDetermineSystem.cmake(246):  configure_file(/usr/share/cmake-3.28/Modules/CMakeSystem.cmake.in /build/CMakeFiles/3.28.3/CMakeSystem.cmake @ONLY )
/src/CMakeLists.txt(339):  configure_file(/src/cmake/config.h.in /build/config.h )
/src/CMakeLists.txt(341):  configure_file(/src/cmake/json_config.h.in /build/json_config.h )
/src/CMakeLists.txt(520):  configure_file(/src/json.h.cmakein /build/json.h @ONLY )
/src/CMakeLists.txt(29):  configure_file(config.h.in config.h )
/src/CMakeLists.txt(126):  configure_file(/build/zconf.h.cmakein /build/zconf.h )
/usr/share/cmake-3.28/Modules/CTestTargets.cmake(28):  configure_file(/usr/share/cmake-3.28/Modules/DartConfiguration.tcl.in /build/DartConfiguration.tcl )
";

    #[test]
    fn parse_configure_files_keeps_only_project_templates() {
        let calls = parse_configure_files(CONFIGURE_TRACE, Path::new("/src"), Path::new("/build"));
        assert_eq!(
            calls,
            vec![
                ConfigureFile {
                    template: PathBuf::from("/src/cmake/config.h.in"),
                    output: PathBuf::from("/build/config.h"),
                },
                ConfigureFile {
                    template: PathBuf::from("/src/cmake/json_config.h.in"),
                    output: PathBuf::from("/build/json_config.h"),
                },
                ConfigureFile {
                    template: PathBuf::from("/src/json.h.cmakein"),
                    output: PathBuf::from("/build/json.h"),
                },
                // A RELATIVE template — the common `configure_file(config.h.in
                // config.h)` form — resolved against the calling file's own
                // directory (/src), and output likewise.
                ConfigureFile {
                    template: PathBuf::from("/src/config.h.in"),
                    output: PathBuf::from("/src/config.h"),
                },
                // A template the project GENERATED into the build directory
                // and then expanded (zlib assembles zconf.h.cmakein). Kept:
                // it is as much the project's own call as a checked-in one,
                // and dropping it here loses the whole config header before
                // any reachability logic downstream gets a say (bzl-41q).
                ConfigureFile {
                    template: PathBuf::from("/build/zconf.h.cmakein"),
                    output: PathBuf::from("/build/zconf.h"),
                },
            ],
            "project calls are kept (absolute OR relative-to-the-calling-file templates), \
             CMake's internal ones dropped, trailing @ONLY ignored"
        );
    }

    // The real shapes json-c's templates use: a plain #cmakedefine (a header
    // check), a #cmakedefine with a @VAR@ value (project-prefixed), a
    // #cmakedefine with a quoted value, the CMake `@@` escape, and a bare
    // @VAR@ (pure substitution). The comment holds a literal @COMMENT_VAR@ on
    // purpose: configure_file substitutes inside comments too (see
    // docs/lore/configure-file-substitutes-inside-comments.md), so the parser
    // MUST see it — skipping comments would miss a defined @VAR@ that CMake
    // really substitutes, and would silently un-do the bzl-fxa.9 escalation
    // for a stray comment @VAR@ (the trap that turned fixture 012 red).
    const TEMPLATE: &str = "\
/* comment @COMMENT_VAR@ */
#cmakedefine HAVE_ENDIAN_H
#cmakedefine JSON_C_HAVE_INTTYPES_H @JSON_C_HAVE_INTTYPES_H@
#cmakedefine ENABLE_RDRAND \"@ENABLE_RDRAND@\"
#cmakedefine ENABLE_THREADING \"@@\"
#cmakedefine HAVE_ENDIAN_H
#define VERSION \"@PROJECT_VERSION@\"
";

    // bzl-41q: dropping build-dir template support entirely left the suite
    // green, so this pins the whole path — the call survives parsing, the
    // plan names the STAGED location, and template_source says where to copy
    // it from. Without the last one the module ships a config_header whose
    // template is not in it.
    #[test]
    fn a_build_dir_template_is_planned_against_its_staged_copy() {
        let dir = std::env::temp_dir().join(format!("bzlf_bdt_{}", std::process::id()));
        let build = dir.join("build");
        fs::create_dir_all(&build).unwrap();
        let template = build.join("zconf.h.cmakein");
        fs::write(&template, "#cmakedefine HAVE_UNISTD_H 1\n").unwrap();

        let calls = vec![ConfigureFile {
            template: template.clone(),
            output: build.join("zconf.h"),
        }];
        let (headers, escalations) =
            build_config_headers(&calls, &dir.join("src"), &build, &HashMap::new());

        assert!(
            escalations.is_empty(),
            "a build-dir template is reachable — the translator stages it — so \
             it must not escalate: {escalations:?}"
        );
        let header = headers.first().expect("the config header must be planned");
        assert_eq!(
            header.template, "zconf.h.cmakein",
            "`template` must name where the file lands INSIDE the module, not \
             where CMake generated it"
        );
        assert_eq!(
            header.template_source.as_deref(),
            Some(template.as_path()),
            "and template_source must say where to copy it from, or the module \
             ships a config_header whose template is absent"
        );
    }

    #[test]
    fn parse_template_macros_collects_defines_and_vars_deduped() {
        let macros = parse_template_macros(TEMPLATE);
        assert_eq!(
            macros.cmakedefines,
            vec![
                "HAVE_ENDIAN_H".to_string(),
                "JSON_C_HAVE_INTTYPES_H".to_string(),
                "ENABLE_RDRAND".to_string(),
                "ENABLE_THREADING".to_string(),
            ],
            "#cmakedefine names, deduped in first-seen order (HAVE_ENDIAN_H appears twice)"
        );
        assert_eq!(
            macros.vars,
            vec![
                // From the comment — parsed on purpose (see the TEMPLATE note).
                "COMMENT_VAR".to_string(),
                "JSON_C_HAVE_INTTYPES_H".to_string(),
                "ENABLE_RDRAND".to_string(),
                "PROJECT_VERSION".to_string(),
            ],
            "@VAR@ names, deduped; comment @VAR@s are included (configure_file \
             substitutes them); the `@@` escape yields no variable"
        );
    }

    #[test]
    fn resolve_config_header_maps_catalog_defines_and_escalates_the_rest() {
        let macros = TemplateMacros {
            cmakedefines: vec![
                "HAVE_ENDIAN_H".to_string(),          // catalog -> label
                "HAVE_STDLIB_H".to_string(),          // catalog -> label (also in cache below)
                "JSON_C_HAVE_INTTYPES_H".to_string(), // prefixed, no exact match -> unmapped
                "ENABLE_RDRAND".to_string(),          // non-catalog, cache-covered option
            ],
            // A catalog fact referenced as a value (`#define SIZEOF_LONG
            // @SIZEOF_LONG@`) — must resolve via the probe, not the host cache.
            vars: vec!["PROJECT_VERSION".to_string(), "SIZEOF_LONG".to_string()],
        };
        let cache: HashMap<String, String> = [
            ("PROJECT_VERSION".to_string(), "1.2.3".to_string()),
            ("ENABLE_RDRAND".to_string(), "1".to_string()),
            // The host's answers for catalog facts — these must NOT leak into
            // `values` (they'd bake this host's config into the module).
            ("HAVE_STDLIB_H".to_string(), "1".to_string()),
            ("SIZEOF_LONG".to_string(), "8".to_string()),
        ]
        .into_iter()
        .collect();

        let (header, unmapped) =
            resolve_config_header("cmake/config.h.in", "config.h", &macros, &cache);

        assert_eq!(header.output, "config.h");
        assert_eq!(header.template, "cmake/config.h.in");
        assert_eq!(
            header.catalog_probes,
            vec![
                "@cc_config//catalog:have_endian_h".to_string(),
                "@cc_config//catalog:have_stdlib_h".to_string(),
                "@cc_config//catalog:sizeof_long".to_string(),
            ],
            "catalog facts map to labels — including SIZEOF_LONG referenced only as @VAR@"
        );
        assert_eq!(
            unmapped,
            vec!["JSON_C_HAVE_INTTYPES_H".to_string()],
            "a #cmakedefine that is neither a catalog probe nor a cache value is escalated"
        );
        assert_eq!(
            header.values,
            vec![
                ("PROJECT_VERSION".to_string(), "1.2.3".to_string()),
                ("ENABLE_RDRAND".to_string(), "1".to_string()),
            ],
            "only non-catalog names get cache values; HAVE_STDLIB_H and SIZEOF_LONG are catalog \
             facts, so their HOST cache answers must NOT appear here — the probe supplies them"
        );
    }

    #[test]
    fn resolve_config_header_escalates_an_unresolved_plain_var() {
        // bzl-fxa.9: json.h.cmakein gates umbrella includes on a plain @VAR@
        // (@JSON_H_JSON_PATCH@) the project computes itself. If left unresolved
        // it lands LITERAL in the output and breaks every includer, so it must
        // escalate. Both directions in one test: RESOLVED_VAR is in the cache
        // and must NOT escalate (it becomes a value); JSON_H_JSON_PATCH is in
        // neither the catalog nor the cache and MUST escalate.
        let macros = TemplateMacros {
            cmakedefines: vec![],
            vars: vec!["JSON_H_JSON_PATCH".to_string(), "RESOLVED_VAR".to_string()],
        };
        let cache: HashMap<String, String> = [("RESOLVED_VAR".to_string(), "yes".to_string())]
            .into_iter()
            .collect();

        let (header, unmapped) = resolve_config_header("json.h.cmakein", "json.h", &macros, &cache);

        assert_eq!(
            unmapped,
            vec!["JSON_H_JSON_PATCH".to_string()],
            "an unresolved plain @VAR@ escalates; a cache-resolved one does not"
        );
        assert_eq!(
            header.values,
            vec![("RESOLVED_VAR".to_string(), "yes".to_string())],
            "the resolved @VAR@ becomes a value and is not among the escalated names"
        );
    }

    #[test]
    fn resolve_config_header_escalates_a_define_and_var_of_the_same_name_once() {
        // A `#cmakedefine FOO @FOO@` names FOO in both cmakedefines and vars.
        // When FOO is unresolved it must escalate exactly once, not twice.
        let macros = TemplateMacros {
            cmakedefines: vec!["FOO".to_string()],
            vars: vec!["FOO".to_string()],
        };
        let cache: HashMap<String, String> = HashMap::new();

        let (_header, unmapped) = resolve_config_header("t.h.in", "t.h", &macros, &cache);

        assert_eq!(
            unmapped,
            vec!["FOO".to_string()],
            "a name that is both a #cmakedefine and a @VAR@ is escalated once, not duplicated"
        );
    }

    #[test]
    fn resolve_config_header_passes_false_token_option_values_through_verbatim() {
        // bzl-fxa.8: an option left OFF is a false token in CMake, so its
        // #cmakedefine must undef. The translator deliberately does NOT blank
        // it here — it passes "OFF" through as the value and lets the expander
        // apply CMake truthiness (is_set). Blanking it would be wrong for a
        // bare @VAR@, which CMake substitutes with the literal token ("OFF").
        // So the assertion is: a false-token cache value is neither escalated
        // nor rewritten; it reaches `values` verbatim.
        let macros = TemplateMacros {
            cmakedefines: vec!["ENABLE_RDRAND".to_string()],
            vars: vec![],
        };
        let cache: HashMap<String, String> = [("ENABLE_RDRAND".to_string(), "OFF".to_string())]
            .into_iter()
            .collect();

        let (header, unmapped) = resolve_config_header("config.h.in", "config.h", &macros, &cache);

        assert!(
            unmapped.is_empty(),
            "a cache-covered option (even OFF) is resolvable, not escalated; got {unmapped:?}"
        );
        assert_eq!(
            header.values,
            vec![("ENABLE_RDRAND".to_string(), "OFF".to_string())],
            "the false token is passed through verbatim; the expander decides truthiness"
        );
    }

    #[test]
    fn cache_value_falls_back_to_cmake_project_version() {
        // project() sets PROJECT_VERSION as a plain variable; the cache records
        // it under CMAKE_PROJECT_VERSION. A template's @PROJECT_VERSION@ must
        // still resolve.
        let cache: HashMap<String, String> = [
            ("CMAKE_PROJECT_VERSION".to_string(), "3.7.0".to_string()),
            ("CMAKE_PROJECT_VERSION_MAJOR".to_string(), "3".to_string()),
        ]
        .into_iter()
        .collect();

        assert_eq!(
            cache_value("PROJECT_VERSION", &cache).as_deref(),
            Some("3.7.0")
        );
        assert_eq!(
            cache_value("PROJECT_VERSION_MAJOR", &cache).as_deref(),
            Some("3")
        );
        assert_eq!(cache_value("NOT_A_VAR", &cache), None);
    }
}
