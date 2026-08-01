//! The Autotools frontend: recovers a build graph from `make -n`.
//!
//! Where `cmake_api` reads a structured reply, this reads the build system's
//! **resolved command stream**. That is the closest autotools analogue to the
//! CMake File API, and it is the same KIND of thing — the build system's own
//! answer after configuration, not the input it was configured from:
//!
//! - `Makefile.am` is declarative and closest in intent, but is NOT resolved:
//!   `foo_LDADD = $(LIBINTL) $(top_builddir)/lib/lib$(PACKAGE).a` names
//!   variables only `configure` fills in, and automake conditionals and
//!   `SUBDIRS` recursion mean the declared graph is not the built one. Parsing
//!   it is the "read the source build files" path `cmake_api`'s module doc
//!   explicitly rejects.
//! - The generated `Makefile` IS resolved but is thousands of lines of make
//!   syntax with recursive expansion; consuming it means implementing enough
//!   of make to be correct.
//! - `make -n` prints exactly what the build would run, fully expanded, and is
//!   byte-identical between runs (verified — more stable than the File API,
//!   which reports dependency order unstably, see bzl-sjp).
//!
//! What the command stream does NOT carry is target NAMES. automake knows a
//! program is called `greeter` because `bin_PROGRAMS` says so; the stream only
//! shows `-o greeter`. Identity is therefore inferred from the artifact, which
//! is this frontend's one genuine weakness versus `cmake_api` and is why
//! [`target_name_from_artifact`] exists as a named, testable decision rather
//! than an inline `file_stem`.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::error::Error;
use crate::model::{BuildGraph, ModuleInfo, Target, TargetKind};

/// One resolved command from the build stream, split into a program and its
/// arguments with shell noise already removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuildCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

/// Runs `make -n` in a configured tree and returns its output.
///
/// The tree must already be configured, exactly as `cmake_api` requires an
/// already-configured build directory — `make -n` on an unconfigured tree
/// reports the rules that would regenerate `Makefile`, not the build.
pub(crate) fn dry_run(build_dir: &Path) -> Result<String, Error> {
    let output = Command::new("make")
        .arg("-n")
        .current_dir(build_dir)
        .output()?;
    if !output.status.success() {
        return Err(Error::CmakeBuildFailed {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Runs `make -p` and returns make's resolved variable database.
///
/// `-n` alongside `-p` so nothing is actually built; the database is a side
/// effect of make reading the tree, not of running it.
pub(crate) fn variable_database(build_dir: &Path) -> Result<String, Error> {
    let output = Command::new("make")
        .arg("-p")
        .arg("-n")
        .current_dir(build_dir)
        .output()?;
    // Not checking status: `make -p` exits nonzero on a tree with nothing to
    // do, having already printed the database, which is all this needs.
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// One target automake declared, recovered from a primary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredTarget {
    /// The name automake knows it by (`greeter`, `libshout.la`).
    pub(crate) name: String,
    /// The install-destination prefix of the primary that declared it
    /// (`bin`, `lib`, `noinst`, `check`). This is where the public/private
    /// signal lives — `noinst_` means built but never installed.
    pub(crate) destination: String,
    /// Which primary declared it, so linkage does not have to be guessed
    /// from a filename: `LTLIBRARIES` builds a shared library, `LIBRARIES`
    /// an archive.
    pub(crate) primary: String,
}

/// Parses `make -p` output into a variable map.
///
/// Split from [`variable_database`] so a frozen real capture can drive it
/// without a configured tree. Only `NAME = value` lines are kept; make's
/// database also contains rules, comments and `NAME := value` forms, none of
/// which carry automake's declarations.
pub(crate) fn parse_variables(database: &str) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    for line in database.lines() {
        // Rules and comments both reach here; a leading tab means a recipe.
        if line.starts_with('\t') || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once(" = ") else {
            continue;
        };
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        // Later definitions win, matching make's own last-assignment-wins.
        vars.insert(name.to_string(), value.trim().to_string());
    }
    vars
}

/// Recovers every target automake DECLARED, from the primaries.
///
/// This is the identity source. The command stream shows `-o greeter` — a
/// filename — while `bin_PROGRAMS = greeter` is the name automake actually
/// uses, and the `bin_` prefix additionally states where it installs.
/// Inferring identity from artifact paths instead breaks on a binary whose
/// name differs from its target's, on libtool's `.libs/libfoo.so.1.0.0`
/// versus `libfoo.la`, and on a non-empty `EXEEXT`.
pub(crate) fn declared_targets(vars: &HashMap<String, String>) -> Vec<DeclaredTarget> {
    let mut targets = Vec::new();
    for (var, value) in vars {
        let Some((destination, primary)) = var.rsplit_once('_') else {
            continue;
        };
        if !matches!(primary, "PROGRAMS" | "LIBRARIES" | "LTLIBRARIES") {
            continue;
        }
        // `dist_`, `nodist_` and `nobase_` are modifiers automake allows in
        // front of the destination; the destination is the last segment.
        let destination = destination.rsplit('_').next().unwrap_or(destination);
        for name in value.split_whitespace() {
            // $(EXEEXT) is empty on Linux and `.exe` elsewhere; the database
            // provides it, so it is expanded rather than assumed.
            let exeext = vars.get("EXEEXT").map(String::as_str).unwrap_or("");
            let name = name.replace("$(EXEEXT)", exeext);
            if name.is_empty() {
                continue;
            }
            targets.push(DeclaredTarget {
                name,
                destination: destination.to_string(),
                primary: primary.to_string(),
            });
        }
    }
    targets.sort_by(|a, b| a.name.cmp(&b.name));
    targets.dedup();
    targets
}

/// The variable prefix automake derives from a target name.
///
/// automake canonicalises by replacing every character that is not
/// alphanumeric or `_` with `_`, so `libshout.la` owns `libshout_la_SOURCES`.
/// Without this the per-target variables cannot be found at all.
pub(crate) fn canonical_name(target_name: &str) -> String {
    target_name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

/// Splits `make -n` output into the commands that actually build something.
///
/// Deliberately conservative about what it keeps. The stream is interleaved
/// with shell bookkeeping automake emits around every compile — `depbase=...`
/// assignments, `mv -f`, `rm -f`, `test -z || mkdir` — and with `make`'s own
/// directory chatter. Recognising only the handful of programs that produce
/// build outputs means an unrecognised line is IGNORED rather than
/// misinterpreted, which is the safe direction: a missed command shows up as a
/// target with no sources, while a misparsed one would silently produce a
/// wrong rule.
pub(crate) fn parse_commands(stdout: &str) -> Vec<BuildCommand> {
    let mut commands = Vec::new();
    for line in stdout.lines() {
        // A `&&`/`;`-joined line can carry several commands; automake writes
        // `depbase=...; gcc ...` as one line.
        for piece in line.split("&&").flat_map(|p| p.split(';')) {
            let piece = piece.trim().trim_end_matches('\\').trim();
            if piece.is_empty() {
                continue;
            }
            let mut tokens = tokenize(piece);
            if tokens.is_empty() {
                continue;
            }
            // `/bin/bash ./libtool --tag=CC --mode=link gcc ...` — the real
            // compiler is several tokens in. Strip the wrappers AND libtool's
            // own flags, which sit between it and the command it wraps; a
            // stripper that stopped at the first non-wrapper would take
            // `--tag=CC` for the program and silently drop every libtool
            // target, which is most of an autotools library build.
            while tokens
                .first()
                .is_some_and(|t| is_shell_wrapper(t) || is_libtool_flag(t))
            {
                tokens.remove(0);
            }
            let program = tokens.remove(0);
            let program = Path::new(&program)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(program);
            if !is_build_program(&program) {
                continue;
            }
            commands.push(BuildCommand {
                program,
                args: tokens,
            });
        }
    }
    commands
}

/// Whether a token is one of libtool's own options rather than part of the
/// command it wraps. Only the ones that appear BEFORE the wrapped program —
/// `--mode=link` and friends — since everything after belongs to the compiler.
fn is_libtool_flag(token: &str) -> bool {
    token.starts_with("--tag=")
        || token.starts_with("--mode=")
        || matches!(token, "--silent" | "--quiet" | "--verbose")
}

/// Whether a token is a shell the real command is being run through.
fn is_shell_wrapper(token: &str) -> bool {
    matches!(
        Path::new(token).file_name().and_then(|n| n.to_str()),
        Some("bash" | "sh" | "libtool")
    )
}

/// Whether a program produces a build output worth modelling.
///
/// `ranlib` is deliberately absent: it indexes an archive `ar` already
/// created, so treating it as a target would double-count the library.
fn is_build_program(program: &str) -> bool {
    matches!(
        program,
        "gcc" | "g++" | "cc" | "c++" | "clang" | "clang++" | "ar"
    )
}

/// Splits a command into tokens, honouring quotes but not expanding anything.
///
/// Not a shell parser: `make -n` output is already expanded, so the only job
/// is to keep a quoted `-DPACKAGE_STRING="greeter 1.0"` in one piece rather
/// than splitting it on the space.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for c in line.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' | '\'' if quote.is_none() => quote = Some(c),
            c if Some(c) == quote => quote = None,
            c if c.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// The target name for a build artifact.
///
/// The command stream gives a path, not automake's target name, so this is an
/// inference — the one place this frontend is weaker than `cmake_api`, which
/// gets `name` directly from the reply. It strips a `lib` prefix and every
/// known suffix so `libgreet.a`, `libshout.la` and `.libs/libshout.so.1.0.0`
/// all resolve to the same target, which is what makes a dependency edge
/// findable at all.
pub(crate) fn target_name_from_artifact(artifact: &str) -> String {
    let base = Path::new(artifact)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| artifact.to_string());

    // Strip versioned shared-object suffixes (`libfoo.so.1.0.0`) before the
    // extension, since `Path::file_stem` would only remove `.0`.
    let base = match base.find(".so.") {
        Some(i) => base[..i + 3].to_string(),
        None => base,
    };
    let stem = base
        .strip_suffix(".la")
        .or_else(|| base.strip_suffix(".so"))
        .or_else(|| base.strip_suffix(".a"))
        .or_else(|| base.strip_suffix(".lo"))
        .or_else(|| base.strip_suffix(".o"))
        .unwrap_or(&base);

    stem.strip_prefix("lib").unwrap_or(stem).to_string()
}

/// Builds the graph, joining automake's DECLARATIONS to the build's own
/// COMMAND STREAM.
///
/// Each source answers what the other cannot, which is why both are read:
///
/// - the variable database gives target IDENTITY — the name automake uses,
///   the install destination, and which primary declared it. None of that is
///   in the command stream, which shows only `-o greeter`.
/// - the command stream gives what was actually BUILT — the resolved `-I` and
///   `-D` per compile, and which objects really went into which artifact.
///   None of that is in the database, which shows declarations before
///   automake's own conditionals and rules have had their say.
///
/// The two are joined on the artifact each target produces, which both name.
/// This mirrors `cmake_api`, where the File API is primary and the configure
/// trace is a deliberate, documented second source.
pub(crate) fn to_graph(
    commands: &[BuildCommand],
    declared: &[DeclaredTarget],
    vars: &HashMap<String, String>,
    project_name: &str,
) -> BuildGraph {
    // object path -> the source it was compiled from, and the flags it carried.
    let mut source_of: HashMap<String, String> = HashMap::new();
    let mut flags_of: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
    for cmd in commands.iter().filter(|c| c.program != "ar") {
        let Some(output) = flag_value(&cmd.args, "-o") else {
            continue;
        };
        if !cmd.args.iter().any(|a| a == "-c") {
            continue;
        }
        if let Some(source) = cmd.args.iter().rev().find(|a| is_source_file(a)) {
            source_of.insert(output.clone(), source.clone());
            flags_of.insert(output, (includes_of(&cmd.args), defines_of(&cmd.args)));
        }
    }

    // artifact basename -> the link/archive command that produced it, so a
    // declared target can find the step that built it.
    let mut built: HashMap<String, &BuildCommand> = HashMap::new();
    for cmd in commands {
        if let Some(artifact) = produced_artifact(cmd) {
            built.insert(basename(&artifact), cmd);
        }
    }

    let mut targets = Vec::new();
    for decl in declared {
        let canon = canonical_name(&decl.name);

        // Sources come from the DECLARATION, not from the link inputs: a
        // target's _SOURCES is what automake was told, while the link line
        // shows objects, which have to be mapped back. Headers appear in
        // _SOURCES too (automake accepts them so they reach the tarball), so
        // they are split out rather than compiled.
        let declared_sources = vars
            .get(&format!("{canon}_SOURCES"))
            .map(|v| v.split_whitespace().map(str::to_string).collect())
            .unwrap_or_else(Vec::new);
        let (headers, mut sources): (Vec<String>, Vec<String>) = declared_sources
            .into_iter()
            .partition(|s| !is_source_file(s));

        // Flags come from the COMMAND STREAM, via the objects this target's
        // sources compiled to — the database's *_CPPFLAGS are pre-expansion
        // and miss what configure and AM_CPPFLAGS contributed.
        let mut includes = Vec::new();
        let mut local_defines = Vec::new();
        for (object, source) in &source_of {
            if !sources.contains(source) {
                continue;
            }
            if let Some((inc, def)) = flags_of.get(object) {
                includes.extend(inc.iter().cloned());
                local_defines.extend(def.iter().cloned());
            }
        }

        // Dependencies come from the declaration too (_LDADD for programs,
        // _LIBADD for libraries), resolved against the other declared names.
        let declared_names: Vec<&str> = declared.iter().map(|d| d.name.as_str()).collect();
        let mut dependencies: Vec<String> = vars
            .get(&format!("{canon}_LDADD"))
            .or_else(|| vars.get(&format!("{canon}_LIBADD")))
            .map(|v| {
                v.split_whitespace()
                    .filter(|token| declared_names.contains(token))
                    .map(|token| target_label(token))
                    .collect()
            })
            .unwrap_or_else(Vec::new);

        let artifact = built
            .get(&decl.name)
            .and_then(|cmd| produced_artifact(cmd))
            .unwrap_or_else(|| decl.name.clone());

        sources.sort_unstable();
        sources.dedup();
        dependencies.sort_unstable();
        dependencies.dedup();
        includes.sort_unstable();
        includes.dedup();
        local_defines.sort_unstable();
        local_defines.dedup();

        targets.push(Target {
            name: target_label(&decl.name),
            kind: match decl.primary.as_str() {
                "PROGRAMS" => TargetKind::Executable,
                _ => TargetKind::Library,
            },
            // From the PRIMARY, not from a filename: LTLIBRARIES is libtool's
            // form and builds a shared library, LIBRARIES is a plain archive.
            is_shared: decl.primary == "LTLIBRARIES",
            sources,
            // `include_HEADERS` names the project's public headers, the same
            // statement CMake makes with install(FILES ... DESTINATION
            // include). A header in _SOURCES that is not installed stays
            // private.
            public_headers: headers
                .iter()
                .filter(|h| public_headers(vars).contains(*h))
                .cloned()
                .collect(),
            dependencies,
            includes,
            local_defines,
            artifacts: vec![artifact],
            ..Default::default()
        });
    }

    targets.sort_by(|a, b| a.name.cmp(&b.name));

    BuildGraph {
        module: ModuleInfo {
            name: project_name.to_string(),
            version: vars.get("VERSION").cloned(),
        },
        targets,
        tests: Vec::new(),
        config_headers: Vec::new(),
    }
}

/// Every header the project installs, from any `*_HEADERS` primary.
fn public_headers(vars: &HashMap<String, String>) -> Vec<String> {
    vars.iter()
        .filter(|(k, _)| {
            k.ends_with("_HEADERS") && !k.starts_with("noinst") && !k.starts_with("EXTRA")
        })
        .flat_map(|(_, v)| v.split_whitespace().map(str::to_string))
        .collect()
}

/// The Bazel target name for an automake target name, stripping the `lib`
/// prefix and library suffix so `libshout.la` is referred to as `shout`.
fn target_label(name: &str) -> String {
    target_name_from_artifact(name)
}

/// The artifact a link or archive command produces, or `None` for a compile.
fn produced_artifact(cmd: &BuildCommand) -> Option<String> {
    if cmd.program == "ar" {
        let mut rest = cmd.args.iter().filter(|a| !a.starts_with('-'));
        let _mode = rest.next();
        return rest.next().cloned();
    }
    if cmd.args.iter().any(|a| a == "-c") {
        return None;
    }
    flag_value(&cmd.args, "-o")
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// The value following `flag`, e.g. `-o foo` -> `foo`.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// `-I` directories, with the flag stripped and `.`-relative paths kept.
fn includes_of(args: &[String]) -> Vec<String> {
    args.iter()
        .filter_map(|a| a.strip_prefix("-I"))
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .collect()
}

/// `-D` definitions, with the flag stripped.
///
/// autoconf passes a large block of `-DPACKAGE_*` and `-DHAVE_*` on every
/// compile; those are the config-header facts, kept as-is here and left for a
/// later pass to route through `cc_config` the way `configure_file` does.
fn defines_of(args: &[String]) -> Vec<String> {
    args.iter()
        .filter_map(|a| a.strip_prefix("-D"))
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .collect()
}

fn is_source_file(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("c" | "cc" | "cpp" | "cxx" | "m" | "mm" | "s" | "S")
    )
}

fn is_object_or_library(path: &str) -> bool {
    is_object(path) || is_library(path)
}

fn is_object(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("o" | "lo")
    )
}

fn is_library(path: &str) -> bool {
    path.ends_with(".a") || path.ends_with(".la") || path.contains(".so")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen;

    /// A real `make -n` capture from fixture 001, trimmed of the autoconf
    /// -DPACKAGE_* block for readability. Frozen evidence: this is what
    /// automake actually emits, so the parser can be wrong against it.
    const STREAM: &str = r#"
depbase=`echo src/main.o | sed 's|[^/]*$|.deps/&|;s|\.o$||'`;\
gcc -DHAVE_CONFIG_H -I.  -g -O2 -MT src/main.o -MD -MP -MF $depbase.Tpo -c -o src/main.o src/main.c &&\
mv -f $depbase.Tpo $depbase.Po
depbase=`echo src/greet.o | sed 's|[^/]*$|.deps/&|;s|\.o$||'`;\
gcc -DHAVE_CONFIG_H -I. -Isrc -g -O2 -MT src/greet.o -MD -MP -MF $depbase.Tpo -c -o src/greet.o src/greet.c &&\
mv -f $depbase.Tpo $depbase.Po
rm -f libgreet.a
ar cru libgreet.a src/greet.o
ranlib libgreet.a
/bin/bash ./libtool  --tag=CC   --mode=compile gcc -DHAVE_CONFIG_H -I.  -g -O2 -MT src/shout.lo -MD -MP -c -o src/shout.lo src/shout.c
/bin/bash ./libtool  --tag=CC   --mode=link gcc  -g -O2   -o libshout.la -rpath /usr/local/lib src/shout.lo
/bin/bash ./libtool  --tag=CC   --mode=link gcc  -g -O2   -o greeter src/main.o libgreet.a libshout.la
"#;

    /// A real `make -p -n` capture from fixture 001, trimmed to the lines
    /// that carry declarations. Frozen evidence: this is what automake
    /// actually puts in make's database.
    const DATABASE: &str = "\
EXEEXT = \n\
VERSION = 1.0\n\
bin_PROGRAMS = greeter$(EXEEXT)\n\
noinst_LIBRARIES = libgreet.a\n\
lib_LTLIBRARIES = libshout.la\n\
include_HEADERS = src/greet.h\n\
greeter_SOURCES = src/main.c\n\
greeter_LDADD = libgreet.a libshout.la\n\
libgreet_a_SOURCES = src/greet.c src/greet.h\n\
libshout_la_SOURCES = src/shout.c\n\
";

    fn graph_from_captures() -> BuildGraph {
        let vars = parse_variables(DATABASE);
        let declared = declared_targets(&vars);
        to_graph(&parse_commands(STREAM), &declared, &vars, "greeter")
    }

    // automake canonicalises a target name into its variable prefix by
    // replacing every non-alphanumeric character. Without this,
    // `libshout.la`'s sources cannot be found at all.
    #[test]
    fn canonical_name_matches_automakes_variable_naming() {
        assert_eq!(canonical_name("greeter"), "greeter");
        assert_eq!(canonical_name("libshout.la"), "libshout_la");
        assert_eq!(canonical_name("libgreet.a"), "libgreet_a");
        assert_eq!(canonical_name("my-tool"), "my_tool");
    }

    // Identity comes from the primaries, not from artifact paths. The
    // destination prefix is carried because it is where the public/private
    // signal lives — noinst_ means built but never installed.
    #[test]
    fn declared_targets_recovers_names_destinations_and_primaries() {
        let declared = declared_targets(&parse_variables(DATABASE));
        assert_eq!(
            declared,
            vec![
                DeclaredTarget {
                    name: "greeter".to_string(),
                    destination: "bin".to_string(),
                    primary: "PROGRAMS".to_string(),
                },
                DeclaredTarget {
                    name: "libgreet.a".to_string(),
                    destination: "noinst".to_string(),
                    primary: "LIBRARIES".to_string(),
                },
                DeclaredTarget {
                    name: "libshout.la".to_string(),
                    destination: "lib".to_string(),
                    primary: "LTLIBRARIES".to_string(),
                },
            ],
            "$(EXEEXT) must be expanded from the database, not assumed empty"
        );
    }

    #[test]
    fn parses_the_real_command_stream_into_a_graph() {
        let graph = graph_from_captures();
        let names: Vec<&str> = graph.targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["greet", "greeter", "shout"], "{graph:#?}");

        let by_name = |n: &str| {
            graph
                .targets
                .iter()
                .find(|t| t.name == n)
                .unwrap_or_else(|| panic!("missing {n}"))
        };

        // noinst_LIBRARIES -> a static archive built by `ar`.
        let greet = by_name("greet");
        assert_eq!(greet.kind, TargetKind::Library);
        assert!(!greet.is_shared, "an ar archive is not shared");
        assert_eq!(greet.sources, vec!["src/greet.c"]);

        // lib_LTLIBRARIES -> a libtool library. This is the concept with no
        // CMake analogue, and the one the whole frontend was worth writing to
        // find out about.
        let shout = by_name("shout");
        assert_eq!(shout.kind, TargetKind::Library);
        assert!(
            shout.is_shared,
            "LTLIBRARIES is libtool's form and builds a shared library — read \
             from the PRIMARY, not guessed from the .la suffix"
        );
        assert_eq!(shout.sources, vec!["src/shout.c"]);

        // bin_PROGRAMS -> an executable, with BOTH libraries as dependencies.
        // The edges come from the link line's inputs, which is the only place
        // the command stream states them.
        let greeter = by_name("greeter");
        assert_eq!(greeter.kind, TargetKind::Executable);
        assert_eq!(greeter.sources, vec!["src/main.c"]);
        assert_eq!(
            greeter.dependencies,
            vec!["greet", "shout"],
            "both libgreet.a and libshout.la are link inputs"
        );

        // Compile flags are attributed to the target that owns the object,
        // not smeared across every target.
        assert_eq!(greet.includes, vec![".", "src"], "{greet:#?}");
        assert_eq!(greeter.includes, vec!["."], "{greeter:#?}");

        // include_HEADERS is automake's statement that a header is public —
        // the same claim CMake makes with install(FILES ... DESTINATION
        // include). greet.h is listed in libgreet_a_SOURCES AND installed, so
        // it is public; nothing else is.
        assert_eq!(greet.public_headers, vec!["src/greet.h"], "{greet:#?}");
        assert!(greeter.public_headers.is_empty(), "{greeter:#?}");
    }

    // The point of this frontend: codegen has only ever seen CMake. If the
    // model boundary is real, an Autotools graph renders through it with no
    // change to codegen at all.
    #[test]
    fn an_autotools_graph_renders_through_unmodified_codegen() {
        let graph = graph_from_captures();
        let rendered = codegen::render(&graph).build_bazel;

        assert!(
            rendered.contains("cc_library(\n    name = \"greet\","),
            "the static archive becomes a cc_library:\n{rendered}"
        );
        assert!(
            rendered.contains("cc_shared_library(\n    name = \"shout_shared\","),
            "and the libtool library gets the shared-library treatment zlib \
             drove, with no autotools-specific codegen:\n{rendered}"
        );
        assert!(
            rendered.contains("cc_binary(\n    name = \"greeter\","),
            "the program becomes a cc_binary:\n{rendered}"
        );
        assert!(
            rendered.contains("\":greet\",") && rendered.contains("\":shout\","),
            "with both library edges:\n{rendered}"
        );
    }
}
