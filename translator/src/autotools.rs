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

/// Builds the graph from the parsed command stream.
///
/// Every target comes from a LINK or ARCHIVE step, never from a compile: a
/// compile says a source exists, but only the step that produces an artifact
/// says which target owns it. Sources are then attributed by matching each
/// link's object files back to the compile that produced them, which is the
/// same "trust the build's own answer" reasoning the whole frontend rests on.
pub(crate) fn to_graph(commands: &[BuildCommand], project_name: &str) -> BuildGraph {
    // object path -> the source it was compiled from.
    let mut source_of: HashMap<String, String> = HashMap::new();
    // object path -> the flags its compile carried.
    let mut flags_of: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();

    for cmd in commands.iter().filter(|c| c.program != "ar") {
        let Some(output) = flag_value(&cmd.args, "-o") else {
            continue;
        };
        // A compile has `-c`; anything else with `-o` is a link.
        if !cmd.args.iter().any(|a| a == "-c") {
            continue;
        }
        if let Some(source) = cmd.args.iter().rev().find(|a| is_source_file(a)) {
            source_of.insert(output.clone(), source.clone());
            flags_of.insert(output, (includes_of(&cmd.args), defines_of(&cmd.args)));
        }
    }

    let mut targets = Vec::new();
    for cmd in commands {
        let (artifact, inputs) = match cmd.program.as_str() {
            // `ar cru libfoo.a a.o b.o` — the first non-flag argument is the
            // archive, the rest are its members.
            "ar" => {
                let mut rest = cmd.args.iter().filter(|a| !a.starts_with('-'));
                // Skip the mode string (`cru`), which is not flag-prefixed.
                let _mode = rest.next();
                let Some(archive) = rest.next() else { continue };
                (archive.clone(), rest.cloned().collect::<Vec<_>>())
            }
            _ => {
                let Some(output) = flag_value(&cmd.args, "-o") else {
                    continue;
                };
                if cmd.args.iter().any(|a| a == "-c") {
                    continue;
                }
                let inputs = cmd
                    .args
                    .iter()
                    .filter(|a| is_object_or_library(a))
                    .cloned()
                    .collect::<Vec<_>>();
                (output, inputs)
            }
        };

        let mut sources = Vec::new();
        let mut dependencies = Vec::new();
        let mut includes = Vec::new();
        let mut local_defines = Vec::new();
        for input in &inputs {
            if let Some(source) = source_of.get(input) {
                sources.push(source.clone());
                if let Some((inc, def)) = flags_of.get(input) {
                    includes.extend(inc.iter().cloned());
                    local_defines.extend(def.iter().cloned());
                }
            } else if is_library(input) {
                dependencies.push(target_name_from_artifact(input));
            }
        }
        sources.sort_unstable();
        sources.dedup();
        dependencies.sort_unstable();
        dependencies.dedup();
        includes.sort_unstable();
        includes.dedup();
        local_defines.sort_unstable();
        local_defines.dedup();

        targets.push(Target {
            name: target_name_from_artifact(&artifact),
            kind: if is_library(&artifact) {
                TargetKind::Library
            } else {
                TargetKind::Executable
            },
            is_shared: artifact.ends_with(".la") || artifact.contains(".so"),
            sources,
            dependencies,
            includes,
            local_defines,
            artifacts: vec![artifact],
            ..Default::default()
        });
    }

    targets.sort_by(|a, b| a.name.cmp(&b.name));
    targets.dedup_by(|a, b| a.name == b.name);

    BuildGraph {
        module: ModuleInfo {
            name: project_name.to_string(),
            version: None,
        },
        targets,
        tests: Vec::new(),
        config_headers: Vec::new(),
    }
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

    #[test]
    fn parses_the_real_command_stream_into_a_graph() {
        let commands = parse_commands(STREAM);
        let graph = to_graph(&commands, "greeter");
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
        assert!(shout.is_shared, "a .la builds a shared library");
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
    }

    // The point of this frontend: codegen has only ever seen CMake. If the
    // model boundary is real, an Autotools graph renders through it with no
    // change to codegen at all.
    #[test]
    fn an_autotools_graph_renders_through_unmodified_codegen() {
        let graph = to_graph(&parse_commands(STREAM), "greeter");
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
