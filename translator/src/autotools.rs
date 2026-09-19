//! The Autotools frontend: recovers a build graph from the build's own output.
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
//! - make ECHOES every command as it runs it, fully expanded, and in a stable
//!   command ORDER — more stable than the File API, which reports dependency
//!   order unstably (bzl-sjp). So the build's own stdout is the stream, and
//!   there is no separate interrogation pass. Not byte-identical, since make
//!   interleaves its own progress chatter, which is one reason
//!   `parse_commands` recognises only the programs that build something and
//!   ignores the rest.
//!
//! What the command stream does NOT carry is target NAMES. automake knows a
//! program is called `greeter` because `bin_PROGRAMS` says so; the stream only
//! shows `-o greeter`. So identity comes from a SECOND source, `make -p`'s
//! variable database, via [`declared_targets`] — a declaration, not an
//! inference. Deriving names from artifact paths was tried and abandoned:
//! stripping the `lib` prefix and the suffix collapsed GNU hello's
//! `bin_PROGRAMS = hello` and `noinst_LIBRARIES = lib/libhello.a` onto one
//! name, producing a module that could not load.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config_header::{
    parse_config_file_headers, parse_config_headers, parse_resolved_macro_values,
    plan_config_header, plan_substitution_header,
};
use crate::dependencies::Dependencies;
use crate::error::Error;
use crate::headers::{
    inject_headers_on_include_dirs, is_buildable_source, is_header_file, is_translation_unit,
};
use crate::model::{
    BuildGraph, Discovery, ExternalDependency, ModuleInfo, Target, TargetKind, Test,
};
use crate::needs_attention::{
    ConfigDialect, TestDialect, ctest_command_not_a_target_needs_attention,
    generated_headers_needs_attention, shared_library_absorbs_static_needs_attention,
    sources_outside_deliverable_needs_attention, unconverted_dependency_needs_attention,
    unmapped_config_macros_needs_attention,
};
use crate::paths::{absolutize, anchor_for_display, common_ancestor, normalize_lexically};

/// One resolved command from the build stream, split into a program and its
/// arguments with shell noise already removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuildCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    /// The directory this command runs in, absolute.
    ///
    /// Recursive make descends into each subdirectory and runs its commands
    /// from there, so a path in `args` is relative to THIS, not to the build
    /// root: xz's `src/xz` compiles `../common/tuklib_mbstr_width.c`, and
    /// fixture 002 compiles `../common/util.c`. Recording the directory is
    /// what makes those resolvable at all — `../common/util.c` is neither
    /// absolute nor module-relative, so emitting it verbatim produces a label
    /// that reaches above its own module, which codegen refuses.
    ///
    /// CMake+Ninja needs no equivalent: it runs every command from the build
    /// root, so its paths are already root-relative. This is a structural
    /// difference between the two build systems rather than an oversight in
    /// the CMake frontend.
    pub(crate) dir: PathBuf,
}

/// Converts an Autotools project, mirroring `cmake_api::discover`.
///
/// The tree is CONFIGURED (so `make` has resolved variables to report), then
/// really BUILT — and the build's own stdout IS the command stream, because
/// make echoes every command as it runs it.
///
/// Then built a SECOND time with `make check TESTS=`, because automake defers
/// anything `check_`-prefixed: plain `make` compiles no test program and so
/// emits no command for one. Two build passes, not one — see
/// [`build_check_programs`], which also explains why its failure is not fatal.
///
/// There is no dry-run pass. `make -n -B` was used for this and produced a
/// byte-identical set of build commands 18x slower, because `-B` marks the
/// MAINTAINER rules out of date too: 1,404 `config.status` invocations on xz
/// for files this frontend never reads. See
/// docs/lore/make-n-answers-differently-before-and-after-a-build.md.
///
/// The build has to happen regardless — ground-truth artifacts come from it
/// (docs/architecture/build-verification.md) — so the stream is free.
///
/// What this costs is that `build_dir` must be EMPTY: make reports only work
/// it actually does, so a second run over a built tree yields nothing.
/// Enforced below rather than trusted.
pub fn discover(
    source_dir: &Path,
    build_dir: &Path,
    deliverable_root: &Path,
    deps: &Dependencies,
    install_dir: Option<&Path>,
) -> Result<Discovery, Error> {
    configure(source_dir, build_dir, &deps.configure_env())?;
    // The build IS the interrogation: make echoes every command as it runs
    // them, so its stdout is the resolved command stream.
    let stream = build(build_dir, &[])?;
    // Installed for a DEPENDENT's sake, not this module's: the tree is what
    // another project's configure will be pointed at. Under the fixed prefix
    // so the .pc files it writes are path-free — see
    // dependencies::INSTALL_PREFIX.
    if let Some(dir) = install_dir {
        install(build_dir, dir)?;
    }
    // Then a second pass for the test programs, which the first cannot see:
    // automake defers anything `check_`-prefixed, so plain `make` compiles
    // none of them and emits no command for them. Appended rather than parsed
    // separately — a test program is a target like any other, and the whole
    // point is that it arrives with the same OBSERVED evidence as the rest
    // rather than being inferred from the declaration. See
    // `build_check_programs` for why failure here is not fatal.
    let stream = format!("{stream}\n{}", build_check_programs(build_dir));
    let database = variable_database(build_dir)?;

    // An empty stream is a FAILED conversion, not a project with no targets.
    // make prints nothing when everything is already up to date, so a
    // build_dir that was populated by an earlier run yields a graph with no
    // sources and no error — the exact shape of a check that passes because
    // it is looking at nothing. Asserted rather than assumed: `discover`
    // creating its own build_dir is a caller's convention, not something this
    // function can see.
    // Checked through parse_commands rather than a second recogniser: what
    // matters is whether the GRAPH will have anything, and a guard with its
    // own idea of a build command could pass while parse_commands finds
    // nothing.
    if parse_commands(&stream, build_dir).is_empty() {
        return Err(Error::BuildFailed {
            stderr: format!(
                "`make` in {} produced no build commands. The build directory \
                 must be EMPTY: make echoes commands only for work it actually \
                 does, so an already-built tree reports nothing and the \
                 command stream — this frontend's entire input — comes back \
                 empty.",
                build_dir.display()
            ),
        });
    }

    let db = VariableDatabase::parse(&database, &absolutize(build_dir)?);

    // AC_CONFIG_HEADERS, from config.status — see parse_config_headers on why
    // not configure.ac. The template lives in the SOURCE tree (autoconf's
    // config.h.in is checked in or generated by autoheader at bootstrap
    // time), unlike CMake's, which a project can generate into the build dir.
    let mut config_headers = Vec::new();
    let mut needs_attention = Vec::new();
    let status = std::fs::read_to_string(build_dir.join("config.status")).unwrap_or_default();
    // What configure resolved each macro to, quoted into the escalations as
    // evidence for the agent's decision. Read once; both loops below use it.
    let resolved = parse_resolved_macro_values(&status);
    // Which of those a BOOLEAN build option controls. Elicited by
    // re-configuring per flag; see `resolve_flag_macros` for why it cannot
    // simply be read, and why valued flags are left out.
    // Explicit AC_DEFINEs this build selected, from the m4 expansion
    // intersected with config.status. Read once per conversion.
    let traced = resolve_traced_defines(source_dir, build_dir, &resolved);
    let flag_macros = resolve_flag_macros(
        source_dir,
        build_dir,
        &read_configure_flags(source_dir),
        &resolved,
    );
    for (output, template) in parse_config_headers(&status) {
        let template_path = source_dir.join(&template);
        let Ok(text) = std::fs::read_to_string(&template_path) else {
            continue;
        };
        let (header, unmapped) = plan_config_header(
            &output,
            &template,
            &text,
            db.scope_for(&output),
            &flag_macros,
            &traced,
        );
        if !unmapped.is_empty() {
            needs_attention.push(unmapped_config_macros_needs_attention(
                &output,
                &template,
                &unmapped,
                ConfigDialect::Autoconf,
                &resolved,
            ));
        }
        config_headers.push(header);
    }
    // And the OTHER way autoconf generates a header: inside AC_CONFIG_FILES,
    // alongside the Makefiles. jansson declares its private header with
    // AC_CONFIG_HEADERS and its public one — jansson_config.h, included by
    // the installed jansson.h — here, so reading only the first reproduced
    // the private header and silently dropped the public one.
    //
    // A different dialect, not just a different list: these templates are
    // `@VAR@` substitution with no `#undef` declarations, which is CMake's
    // configure_file dialect rather than autoconf's config.h one. Hence
    // `plan_substitution_header` rather than `plan_config_header`.
    for output in parse_config_file_headers(&status) {
        let template = format!("{output}.in");
        let Ok(text) = std::fs::read_to_string(source_dir.join(&template)) else {
            continue;
        };
        let (mut header, unmapped) =
            plan_substitution_header(&output, &template, &text, db.scope_for(&output));
        header.options = flag_options(&header, &flag_macros);
        if !unmapped.is_empty() {
            needs_attention.push(unmapped_config_macros_needs_attention(
                &output,
                &template,
                &unmapped,
                ConfigDialect::Autoconf,
                &resolved,
            ));
        }
        config_headers.push(header);
    }
    let project_name = db
        .root()
        .get("PACKAGE")
        .or_else(|| db.root().get("PACKAGE_NAME"))
        .cloned()
        .unwrap_or_else(|| "project".to_string());
    let source_dir_abs = absolutize(source_dir)?;
    let deliverable_root = absolutize(deliverable_root)?;
    // Same precondition the CMake frontend enforces: a deliverable root that
    // does not contain the project cannot cap anything, and every path would
    // escape it.
    if !source_dir_abs.starts_with(&deliverable_root) {
        return Err(Error::SourceDirOutsideDeliverableRoot {
            source_dir: source_dir_abs.to_string_lossy().into_owned(),
            deliverable_root: deliverable_root.to_string_lossy().into_owned(),
        });
    }
    let (mut graph, graph_needs_attention, module_root) = to_graph_with_dependencies(
        &parse_commands(&stream, build_dir),
        &db,
        &project_name,
        &source_dir_abs,
        &deliverable_root,
        deps,
    );
    graph.config_headers = config_headers;
    // And a THIRD source: headers the project generates as REPLACEMENTS for
    // system ones. gnulib is the case that exists — it ships `string.in.h`
    // and generates a `string.h` that shadows the platform's, so
    // `#include <string.h>` finds gnulib's and its `#include_next` reaches
    // the real one behind it.
    //
    // Reproduced rather than skipped even where the replacement turns out to
    // be inert on this libc: that is a fact about the conversion host, and
    // gnulib exists precisely because it is false elsewhere. See
    // docs/architecture/overview.md.
    //
    // Unlike the other two sources this is not a declaration anywhere — no
    // AC_CONFIG_HEADERS, no AC_CONFIG_FILES. The only evidence is the
    // generation recipe in the build's own output, which is why it is parsed
    // from the stream. See `parse_replacement_headers`.
    for replacement in parse_replacement_headers(&stream, &absolutize(build_dir)?) {
        let Some((template, shadow_dir)) = rebase_shadow_dir(&replacement, &module_root) else {
            // Generated somewhere the module cannot name. Skipped rather than
            // escalated for now: no corpus project hits it, and inventing an
            // escalation text with no project behind it is how they go stale.
            continue;
        };
        graph.config_headers.push(crate::model::ConfigHeader {
            output: replacement.output.clone(),
            template,
            template_source: None,
            catalog_probes: Vec::new(),
            // Nothing escalates for a replacement header: every name it
            // references is a gnulib substitution the recipe supplies.
            unresolved: Vec::new(),
            // Plain `@VAR@` throughout — gnulib templates declare nothing
            // with `#undef`, and every value is already resolved in the
            // recipe, so there is nothing for a catalog probe to answer.
            values: replacement.values.clone(),
            dialect: crate::model::ConfigDialect::Substitution,
            // A shadowing header is a gnulib replacement; its `values` come
            // from the generation recipe, not from an option.
            options: Vec::new(),
            splices: rebase_splices(&replacement.splices, &shadow_dir, &module_root),
            shadow_dir: Some(shadow_dir),
        });
    }
    needs_attention.extend(graph_needs_attention);

    // A header the build wrote into its own tree that no rule above
    // reproduces. Compiles reach it through a `-I` into the build tree, which
    // the include rebase drops (rightly: it is not a module path), so
    // without this the module compiles against nothing and fails on a
    // missing header far from the cause — libevent's event-config.h, made
    // by `sed` from config.h at make time.
    let generated = generated_headers_in_build_tree(
        &parse_commands(&stream, build_dir),
        &absolutize(build_dir)?,
        &source_dir_abs,
        &graph.config_headers,
        &stream,
    );
    if !generated.is_empty() {
        needs_attention.push(generated_headers_needs_attention(&generated));
    }

    // The same header-staging passes the CMake frontend runs, and for the
    // same reasons: a header reachable on the include path is an input Bazel
    // must be told about, and a header beside its source is found by the
    // quoted-include rule that no build system reports. Neither is
    // CMake-specific — `headers` takes a `[Target]` and a source root and
    // knows nothing about where the graph came from.
    inject_headers_on_include_dirs(&mut graph.targets, &module_root);
    inject_textual_includes(&mut graph.targets, &module_root);
    // After the sweep above, because that is what puts an `include_HEADERS`
    // file no `_SOURCES` lists into a target at all. automake attaches a
    // public header to no target — `include_HEADERS` is a project-level
    // statement — so the target that can SEE it (it is among the target's
    // inputs) and the declaration that it is public are combined here, the
    // way the CMake frontend combines `install(FILES)` with the include
    // path. libevent and hwloc declare every public header exactly this way.
    promote_installed_headers(&mut graph.targets, &db);

    // A module with no targets cannot be built, compared or tested, so every
    // downstream tier reports vacuous success — the repo's named recurring
    // failure, one level below where the empty-stream guard above looks.
    //
    // Those two guards are not redundant. The stream guard catches "make did
    // nothing"; this catches "make did something and none of it became a
    // target", which is what a wrong `make` invocation produces. Measured:
    // passing AUTOMAKE=: to suppress automake's regeneration rules made
    // libidn2 convert to 29 config headers and ZERO targets, exit 0, module
    // written. Nothing objected until a sweep delta was read by hand.
    //
    // Config headers and tests are named in the message because they are
    // what a zero-target conversion usually DOES produce, and seeing "29
    // config headers, 0 targets" points at the build invocation rather than
    // at the project.
    if graph.targets.is_empty() {
        return Err(Error::BuildFailed {
            stderr: format!(
                "the conversion produced NO build targets (but {} config \
                 header(s) and {} test(s)). `make` ran and its command stream \
                 was not empty, so the loss is between the stream and target \
                 extraction — check the `make` invocation before suspecting \
                 the project. A module with no targets builds, compares and \
                 tests vacuously, so this fails here rather than downstream.",
                graph.config_headers.len(),
                graph.tests.len(),
            ),
        });
    }

    Ok(Discovery {
        graph,
        // Unmapped config macros, and sources that escaped the module. Two
        // gaps remain undetected — an external library the project links but
        // does not build, and a declared target `make` never produced — which
        // is honest now and wrong once it meets a project it cannot fully
        // convert. See bzl-yjn.5.
        needs_attention,
        module_root,
    })
}

/// Records every textually-`#include`d source that is NOT already compiled
/// into `textual_sources`, reading each target's sources off disk to find
/// them.
///
/// Reads the filesystem, so it sits here rather than in `to_graph`, which is
/// pure over the command stream and variable database and is testable for
/// that reason. The scan itself is `headers::textually_included_sources` —
/// shared, because the CMake frontend faces the same question and neither
/// frontend may import the other.
///
/// Two things happen per include, and only the second is obvious. The path is
/// resolved against the INCLUDING file's directory, since `#include "impl.c"`
/// in `lib/xmltok.c` means `lib/impl.c` and the caller is the only one who
/// knows where the includer sits. And a file that is ALSO a declared source
/// stays in `srcs` and is not recorded here at all — being `#include`d is no
/// evidence that a file is not separately compiled. See the body for why;
/// that one was believed backwards until libmicrohttpd disproved it.
fn inject_textual_includes(targets: &mut [Target], module_root: &Path) {
    for target in targets.iter_mut() {
        let mut textual: Vec<String> = Vec::new();
        for source in &target.sources {
            let Ok(text) = std::fs::read_to_string(module_root.join(source)) else {
                // A generated or already-staged source may not be on disk at
                // this point. Silently skipped rather than escalated: the
                // scan is an enrichment, and a file we cannot read is
                // indistinguishable from one with no textual includes.
                continue;
            };
            let includer_dir = Path::new(source).parent().unwrap_or(Path::new(""));
            for included in crate::headers::textually_included_sources(&text) {
                let resolved = normalize_lexically(&includer_dir.join(&included));
                let Some(resolved) = resolved.to_str() else {
                    continue;
                };
                // An include escaping the module root cannot be expressed as
                // a label; the existing escaped-source paths own that case.
                if resolved.starts_with("..") {
                    continue;
                }
                textual.push(resolved.to_string());
            }
        }
        textual.sort();
        textual.dedup();
        // A file that is BOTH included and declared as a source is compiled
        // in its own right, and stays in `srcs`. Being `#include`d is not
        // evidence to the contrary: libmicrohttpd's test_postprocessor_md.c
        // includes internal.c, mhd_str.c and mhd_panic.c while `_SOURCES`
        // declares all three, and automake compiles each into a per-target
        // object. Removing them left the target undefined at link on the
        // symbols they define.
        //
        // So the textual set is only those NOT already compiled. expat's
        // xmltok_impl.c is in neither `_SOURCES` nor any command, which is
        // what made it textual-only there.
        textual.retain(|t| !target.sources.contains(t));
        target.textual_sources.extend(textual);
        target.textual_sources.sort();
        target.textual_sources.dedup();
    }
}

/// Runs `configure` in `build_dir`, out of tree.
///
/// Autotools supports building outside the source directory, which is what
/// keeps the source tree clean the way CMake's `-B` does.
/// Macros the project defines with an explicit literal AND that this build
/// actually defined — `name -> value`.
///
/// TWO SOURCES, INTERSECTED, because neither answers alone:
///
/// - `autoconf --trace` runs autoconf's own m4 expansion and reports every
///   `AC_DEFINE` with its literal value. It supplies the VALUE and the
///   PROVENANCE (an explicit define, not a probe result) — but it reports
///   every branch, so it says what CAN be defined, not what IS.
/// - `config.status`'s `D[]` table says which macros this configure run
///   actually defined. It supplies the SELECTION — but not the provenance,
///   so on its own it cannot tell a project literal from a host probe
///   answer, which is why bzl-yjn.10 only quotes it as evidence.
///
/// Using the trace alone shipped and was reverted: xz picks ONE cpucores
/// backend in a shell `case` with a separate `AC_DEFINE` per branch, and
/// the trace emitted all five. The module then compiled `sys/systemcfg.h`,
/// an AIX-only header, and xz stopped building. Each of those five names is
/// defined exactly once, so a per-name ambiguity filter cannot see it —
/// only `D[]` knows which branch ran.
///
/// Still excluded after intersecting:
///
/// - An EMPTY trace value. `AC_CHECK_HEADERS`, `AC_CHECK_FUNCS` and
///   `AC_USE_SYSTEM_EXTENSIONS` all trace with no `$2` because autoconf
///   supplies the answer from a probe; those stay probes, resolved against
///   the CONSUMER's toolchain rather than this host's.
/// - A macro whose trace carries several DIFFERENT values. libmicrohttpd's
///   `_MHD_EXTERN` has three, one per platform.
fn resolve_traced_defines(
    source_dir: &Path,
    scratch_root: &Path,
    resolved: &HashMap<String, String>,
) -> HashMap<String, String> {
    intersect_traced_with_selected(read_traced_defines(source_dir, scratch_root), resolved)
}

/// Keeps only the traced macros `config.status` actually selected.
///
/// Split from the `autoconf` invocation above so a test can drive the
/// decision without running autoconf — the invocation is what made the
/// earlier version of this untestable, and the reverted `375b058` bug lived
/// entirely in this filter.
fn intersect_traced_with_selected(
    traced: HashMap<String, String>,
    resolved: &HashMap<String, String>,
) -> HashMap<String, String> {
    traced
        .into_iter()
        // The intersection. A name the trace knows but `config.status` did
        // not define belongs to a branch this build did not take.
        .filter(|(name, _)| resolved.contains_key(name))
        .collect()
}

/// Every `AC_DEFINE` autoconf's m4 expansion reports, with its literal
/// value — `name -> value`, before intersecting with what was selected.
///
/// Not a parse of `configure.ac`, which could not answer this: xz builds its
/// macro names with `AC_DEFINE(HAVE_DECODER_[]m4_toupper(NAME), ...)` inside
/// an `m4_foreach`, so the literal `HAVE_DECODER_LZMA2` appears in the
/// source exactly zero times. The trace prints it, resolved.
///
/// Runs against a COPY of the source tree. autom4te always creates
/// `autom4te.cache` in its working directory and honours no flag to move it
/// (`-C` adds a location; `--no-cache` is an autom4te option that
/// `autoconf` REJECTS, and passing it makes every trace fail silently),
/// while the sandbox source is read-only. A WHOLE copy, because
/// `configure.ac` can reference anything — xz's `AC_INIT` shells out to
/// `build-aux/version.sh`, and a partial copy fails with "AC_INIT should be
/// called with package and version arguments". Measured ~1 MB and under
/// 60ms per corpus project.
///
/// Tracing in the source tree itself is also wrong: it touches
/// `aclocal.m4`, and the later `make` then believes `Makefile.in` is stale.
/// A read of the inputs must not change what the build does.
fn read_traced_defines(source_dir: &Path, scratch_root: &Path) -> HashMap<String, String> {
    let Ok(source) = absolutize(source_dir) else {
        return HashMap::new();
    };
    let scratch = scratch_root.join("_autoconf_trace");
    let _ = std::fs::remove_dir_all(&scratch);
    if copy_tree(&source, &scratch).is_err() {
        return HashMap::new();
    }
    let traced = Command::new("autoconf")
        .arg("--trace=AC_DEFINE:$1|$2")
        .current_dir(&scratch)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_traced_defines(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&scratch);
    traced
}

/// Recursive directory copy, for staging the tree the trace reads.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else if entry.path().is_file() {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

/// The `name -> value` map from `autoconf --trace=AC_DEFINE:$1|$2` output.
/// Split from the invocation so a captured real trace can drive it.
fn parse_traced_defines(trace: &str) -> HashMap<String, String> {
    let mut values: HashMap<&str, std::collections::HashSet<&str>> = HashMap::new();
    for line in trace.lines() {
        let Some((name, value)) = line.split_once('|') else {
            continue;
        };
        if !name.is_empty() {
            values.entry(name).or_default().insert(value);
        }
    }
    values
        .into_iter()
        .filter_map(|(name, vs)| {
            let mut it = vs.into_iter();
            let (Some(value), None) = (it.next(), it.next()) else {
                return None; // several distinct definitions: conditional
            };
            (!value.is_empty()).then(|| (name.to_string(), value.to_string()))
        })
        .collect()
}

/// A build option the project exposes, as `configure --help` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigureFlag {
    /// The flag as a consumer would pass it, always in `--enable-`/`--with-`
    /// form even where `--help` lists the `--disable-` spelling. autoconf
    /// treats `--disable-X` as `--enable-X=no`, so the positive form is the
    /// one name for the pair.
    pub(crate) name: String,
    /// Whether the flag takes a VALUE (`--enable-decoders=LIST`) rather than
    /// being a plain on/off switch.
    ///
    /// autoconf's own convention, and the discriminator for whether a
    /// `bool_flag` can express the option at all: a valued flag either sets
    /// many macros at once (`--enable-decoders` toggles 12) or changes one
    /// macro's VALUE (`--enable-assume-ram=256` moves ASSUME_RAM from 128 to
    /// 256), and a boolean knob expresses neither.
    pub(crate) valued: bool,
}

/// Reads the build options out of `configure --help`.
///
/// Resolved output, not a parse of `configure` as shell: this runs the
/// project's own script and reads what it prints, the same category as
/// running the build and reading its stdout.
///
/// Measured per project: xz 26 boolean / 9 valued, libmicrohttpd 24 / 10,
/// libidn2 21 / 4.
pub(crate) fn read_configure_flags(source_dir: &Path) -> Vec<ConfigureFlag> {
    let Ok(configure) = absolutize(source_dir) else {
        return Vec::new();
    };
    let Ok(output) = Command::new(configure.join("configure"))
        .arg("--help")
        .current_dir(&configure)
        .output()
    else {
        return Vec::new();
    };
    parse_configure_flags(&String::from_utf8_lossy(&output.stdout))
}

/// The flag list from `configure --help` text. Split from the invocation so
/// a captured real `--help` can drive it without running anything.
pub(crate) fn parse_configure_flags(help: &str) -> Vec<ConfigureFlag> {
    let mut flags = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in help.lines() {
        let trimmed = line.trim_start();
        // Indentation is what separates a flag line from the prose that
        // wraps under it — an unindented `--enable-FEATURE` in the preamble
        // is autoconf's own generic help, not this project's option.
        if line.len() == trimmed.len() || !trimmed.starts_with("--") {
            continue;
        }
        let token = trimmed.split_whitespace().next().unwrap_or_default();
        // `--enable-FEATURE[=ARG]` marks the value OPTIONAL, so the bracket
        // opens before the `=` and splitting alone leaves `--enable-FEATURE[`.
        // Trimmed here rather than in the placeholder check below, so a real
        // project flag written `--enable-x[=ARG]` is still recognised as
        // valued rather than escaping as a name with a bracket in it.
        let token = token.trim_end_matches(']');
        let (flag, valued) = match token.split_once('=') {
            Some((flag, _)) => (flag.trim_end_matches('['), true),
            None => (token, false),
        };
        // The positive spelling names the pair. `--disable-X` is autoconf's
        // shorthand for `--enable-X=no`, so keying on the literal text would
        // record two options where the project exposes one.
        let Some(name) = flag
            .strip_prefix("--disable-")
            .map(|rest| format!("--enable-{rest}"))
            .or_else(|| {
                flag.strip_prefix("--without-")
                    .map(|rest| format!("--with-{rest}"))
            })
            .or_else(|| {
                (flag.starts_with("--enable-") || flag.starts_with("--with-"))
                    .then(|| flag.to_string())
            })
        else {
            continue;
        };
        // autoconf's own flags, not the project's. Two kinds: the preamble
        // documents the SYNTAX with `--enable-FEATURE`/`--with-PACKAGE`
        // placeholders, and every generated configure carries a handful of
        // real-but-generic switches. Neither is a choice this project
        // exposes, and emitting a knob for them would put the same ones in
        // every converted module.
        const AUTOCONF_OWN: &[&str] = &[
            "--enable-option-checking",
            "--enable-dependency-tracking",
            "--enable-silent-rules",
            "--enable-shared",
            "--enable-static",
            "--enable-fast-install",
            "--with-pic",
            "--with-gnu-ld",
            "--with-sysroot",
            "--enable-libtool-lock",
        ];
        if name.ends_with("-FEATURE") || name.ends_with("-PACKAGE") {
            continue;
        }
        if AUTOCONF_OWN.contains(&name.as_str()) {
            continue;
        }
        if seen.insert(name.clone()) {
            flags.push(ConfigureFlag { name, valued });
        }
    }
    flags
}

/// Which config macros each BOOLEAN build option controls, as
/// `macro -> flag`.
///
/// Determined by configuring again with the flag disabled and diffing
/// `config.status`'s `D[]` table against the baseline: a macro present in
/// the default build and absent with the flag off is one that flag controls.
/// autoconf records the mapping nowhere readable — `ac_cs_config` is empty
/// for a default build, `make -p` carries no `enable_*`, the command stream
/// never mentions it — so it is ELICITED rather than read (bzl-1p6.1).
///
/// Only boolean flags, because only they map onto a `bool_flag`. A valued
/// one either toggles many macros at once or changes a macro's VALUE rather
/// than its presence, and this diff would report the first as a pile of
/// unrelated booleans and miss the second entirely.
///
/// A macro the flag merely GATES A PROBE FOR is excluded: `--disable-poll`
/// removes both `HAVE_POLL` and `HAVE_POLL_H` from libmicrohttpd's table,
/// but the second is a `check_include_file` result the flag only skips
/// running. `config.log` records which names were probed, so the two are
/// separable — and baking a probe result in as a project choice is exactly
/// the failure the escalation exists to prevent.
///
/// Costs one configure per flag (~3s measured on xz, 24 boolean flags), on a
/// pipeline that already configures and builds. Returns an empty map rather
/// than failing: an option nobody could classify escalates, which is the
/// status quo.
fn resolve_flag_macros(
    source_dir: &Path,
    build_dir: &Path,
    flags: &[ConfigureFlag],
    baseline: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut owned = HashMap::new();
    let Ok(source) = absolutize(source_dir) else {
        return owned;
    };
    let probed = probed_names(build_dir);
    for flag in flags.iter().filter(|f| !f.valued) {
        let off = build_dir.with_file_name(format!(
            "{}_flagprobe",
            build_dir.file_name().unwrap_or_default().to_string_lossy()
        ));
        let _ = std::fs::remove_dir_all(&off);
        if std::fs::create_dir_all(&off).is_err() {
            continue;
        }
        let disabled = flag.name.replacen("--enable-", "--disable-", 1);
        let disabled = disabled.replacen("--with-", "--without-", 1);
        let ran = Command::new(source.join("configure"))
            .arg("-q")
            .arg(&disabled)
            .current_dir(&off)
            .output();
        if ran.is_ok_and(|o| o.status.success()) {
            let status = std::fs::read_to_string(off.join("config.status")).unwrap_or_default();
            let without = crate::config_header::parse_resolved_macro_values(&status);
            for name in baseline.keys() {
                // Present by default, gone with the flag off — and not a
                // probe the flag merely skipped.
                if !without.contains_key(name) && !probed.contains(name) {
                    owned.insert(name.clone(), flag.name.clone());
                }
            }
        }
        let _ = std::fs::remove_dir_all(&off);
    }
    owned
}

/// The `(macro, flag)` pairs for values on this header that a build option
/// controls, in the shape `model::ConfigHeader::options` takes.
///
/// The flag name is carried rather than derived, because codegen turns it
/// into a `bool_flag` target name and a `--enable-x` does not map to a
/// label by any rule codegen could apply without knowing autoconf.
fn flag_options(
    header: &crate::model::ConfigHeader,
    flag_macros: &HashMap<String, String>,
) -> Vec<(String, String)> {
    header
        .values
        .iter()
        .filter_map(|(name, _)| {
            flag_macros
                .get(name)
                .map(|flag| (name.clone(), flag.clone()))
        })
        .collect()
}

/// The macro names `configure` answered with a PROBE, from `config.log`'s
/// `ac_cv_*` cache lines.
///
/// The discriminator between a macro a flag defines and one it merely gates
/// a probe for. Without it, `--disable-poll` looks like it owns
/// `HAVE_POLL_H`, which is a `check_include_file` result.
fn probed_names(build_dir: &Path) -> std::collections::HashSet<String> {
    let log = std::fs::read_to_string(build_dir.join("config.log")).unwrap_or_default();
    let mut names = std::collections::HashSet::new();
    for line in log.lines() {
        let line = line.trim();
        for prefix in [
            "ac_cv_header_",
            "ac_cv_func_",
            "ac_cv_type_",
            "ac_cv_have_decl_",
        ] {
            if let Some(rest) = line.strip_prefix(prefix)
                && let Some((var, _)) = rest.split_once('=')
            {
                names.insert(format!("HAVE_{}", var.to_ascii_uppercase()));
            }
        }
    }
    names
}

fn configure(source_dir: &Path, build_dir: &Path, env: &[(String, String)]) -> Result<(), Error> {
    std::fs::create_dir_all(build_dir)?;
    // Absolutized because the command runs with `current_dir(build_dir)`: a
    // caller-supplied source_dir is usually relative (Bazel passes an
    // execroot-relative path), and a relative path stops resolving the moment
    // the working directory changes. Same reason cmake_api absolutizes before
    // comparing paths.
    let configure = absolutize(source_dir)?.join("configure");
    let output = Command::new(&configure)
        // A prefix pkg-config's relocation can stand in for: the install
        // step below writes under it, and dependents read it through a
        // sysroot. Explicit rather than autoconf's default, which is the
        // same value, so the two halves cannot drift.
        //
        // And NOTHING else: the ground truth is the project's own default
        // configuration. A consumer's choices are its own flags on the
        // converted module's options, and what this host happens to have
        // installed surfaces as an escalation — see overview.md, "Convert
        // the project, not the consumer's use of it". *(History: a
        // configure_args attribute carried Open MPI's flags into libevent's
        // and hwloc's pins for one day, 2026-09-18, and a guessed
        // --disable-pci showed why no gate could catch it.)*
        .arg(format!("--prefix={}", crate::dependencies::INSTALL_PREFIX))
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(build_dir)
        .output()
        .map_err(|e| {
            // A source tree with configure.ac but no configure has not been
            // bootstrapped; saying so beats "no such file or directory".
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::ConfigureFailed {
                    stderr: format!(
                        "{} not found — the project ships configure.ac but has not been \
                         bootstrapped. Run autoreconf -i in the source tree first.",
                        configure.display()
                    ),
                }
            } else {
                Error::Io(e)
            }
        })?;
    if !output.status.success() {
        return Err(Error::ConfigureFailed {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

/// `make install` into `dest` as a `DESTDIR`, after the ground-truth build.
/// Conversion-time scaffolding for a dependent's configure; nothing here
/// ships. See `dependencies`.
fn install(build_dir: &Path, dest: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(dest)?;
    let output = Command::new("make")
        .arg("install")
        .arg(format!("DESTDIR={}", absolutize(dest)?.display()))
        .current_dir(build_dir)
        .output()?;
    if !output.status.success() {
        return Err(Error::BuildFailed {
            stderr: format!(
                "make install failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ),
        });
    }
    Ok(())
}

/// Really builds the project, producing the ground-truth artifacts.
///
/// `extra_args` is how the test-programs pass is expressed — see
/// [`build_check_programs`]. Empty for the ordinary build.
fn build(build_dir: &Path, extra_args: &[&str]) -> Result<String, Error> {
    // Parallel: the build is the ground-truth capture, and nothing about it
    // needs to be serial. Measured 14s -> 2s on xz. `-j` with no argument
    // would be unbounded, which on a large project starves the machine, so
    // the job count is passed explicitly.
    //
    // HALF the cores, not all of them, because this runs INSIDE a Bazel
    // action and the two schedulers multiply. Bazel may run several
    // conversions at once, each spawning its own `make -j`, so the machine
    // sees jobs x actions concurrent compiles — and on a 32-core, 30 GB
    // container that is what kills it. The .bazelrc caps Bazel's half; this
    // caps the inner half. Neither alone bounds the product.
    //
    // Honoured from MAKEFLAGS when Bazel sets it (the jobserver), so a
    // future --local_cpu_resources change does not need this line updated.
    let jobs = std::env::var("BAZELIFIER_BUILD_JOBS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| (n.get() / 2).max(1))
                .unwrap_or(1)
        });
    let output = Command::new("make")
        .arg(format!("-j{jobs}"))
        // automake's escape hatch from AM_SILENT_RULES. A project that
        // enables it prints "  CC foo.o" instead of the command, and this
        // frontend's ENTIRE input is the command stream — so such a project
        // converts to an empty graph rather than failing. Measured on
        // libidn2: 0 compile commands without this flag, 226 with it.
        //
        // Passed unconditionally rather than only when configure.ac has
        // AM_SILENT_RULES, because the narrower version would mean reading
        // configure.ac, which this frontend deliberately does not do — and
        // the broad version measured free. All four Autotools corpus
        // projects convert byte-identically with it (modulo a sandbox number
        // that already varied run to run), with identical target, test and
        // escalation counts.
        .arg("V=1")
        // Keeps make's `Entering directory` announcements attached to the
        // commands they enclose. Under `-j` a sub-make's output can otherwise
        // interleave with its siblings', and `parse_commands` reads those
        // announcements as a STACK — an interleaved one silently attributes a
        // compile to the wrong directory, which surfaces as a source path that
        // does not resolve. Measured no slower (still 2s on xz).
        .arg("--output-sync=recurse")
        .args(extra_args)
        .current_dir(build_dir)
        .output()?;
    if !output.status.success() {
        return Err(Error::BuildFailed {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Builds the project's `check_PROGRAMS` and returns their command stream.
///
/// A second pass, because automake's `check_` prefix means plain `make` never
/// compiles them: they are deferred so someone installing the project does
/// not pay to build its test suite. Measured on jansson — the ordinary build
/// emits 36 compile commands and mentions a test program in NONE of them,
/// while this pass emits 19 that name all 18. Without it the frontend cannot
/// see the tests at all, and reports `tests: 0`, which is a false positive
/// claim rather than an absence.
///
/// `TESTS=` is what keeps this to a BUILD. `make check` would also RUN the
/// suite, which costs time the conversion does not need and makes the exit
/// status depend on whether the project's own tests pass — jansson's
/// `check-exports` fails natively while all 18 compiled tests pass, so
/// running them would abort a conversion over something unrelated to
/// translation. Emptying `TESTS` is automake's own idiom for the build half.
///
/// Failure is NOT fatal, and that is the important part. A project whose test
/// programs do not compile still converts, minus its tests; the alternative
/// is that one broken test target blocks the whole conversion. The caller
/// gets an empty stream and carries on.
fn build_check_programs(build_dir: &Path) -> String {
    build(build_dir, &["check", "TESTS="]).unwrap_or_default()
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
    /// The build directory of the make that declared it — the scope its
    /// per-target variables (`<canon>_SOURCES`, ...) are read from. Two
    /// targets in different directories can canonicalise to one variable
    /// name (hwloc's utility `hwloc-bind` and its test `hwloc_bind` both own
    /// `hwloc_bind_SOURCES`), so the name alone cannot say which is meant.
    pub(crate) directory: PathBuf,
}

/// make's variable database, one scope per directory whose make printed it.
///
/// `make -p -n` on a recursive project prints one database per sub-make,
/// each preceded by that make's `# make[N]: Entering directory`
/// announcement (commented, like everything else in -p output) and printed
/// AFTER its own sub-makes have returned — so the text following a
/// `Leaving directory` line is the PARENT's. Hence a stack, not "the last
/// directory entered": splitting on `Entering` alone handed hwloc's
/// `utils/hwloc` database, its nine `TESTS`, to the last subdirectory
/// entered before it was printed, and 67 of hwloc's 214 escalated tests
/// were that misattribution. The top-level make's own database follows the
/// last `Leaving` and is keyed by the build root. A directory entered twice
/// (automake recurses into `.` for `SUBDIRS = . sub`) is one scope.
///
/// Within a scope the last definition wins, which is make's own rule.
/// Across scopes NOTHING is merged, and holding that line is the reason
/// this is a type rather than a map: recursive make never sees two
/// directories' definitions together, and every name automake emits is
/// scoped to its Makefile — the `am__*` indirection (`am__EXEEXT_1` is
/// `test_md5` in one of libmicrohttpd's directories and
/// `basicauthentication` in another), the per-target `<canon>_SOURCES`
/// (hwloc's `hwloc-bind` and `hwloc_bind`), and the primaries and `TESTS`,
/// which a project declares once per directory. A consumer that wants
/// every directory's declarations walks [`VariableDatabase::scopes`] and
/// merges by NAME at its own level, where a merge means something
/// (`declared_targets`, `installed_headers`, `classify_tests_per_directory`).
/// A value `configure` substituted into every Makefile (`PACKAGE`,
/// `VERSION`, `EXEEXT`, `includedir`) reads the same in any scope and is
/// taken from the root.
///
/// *(History: until 2026-09-19 the frontend flattened every scope into one
/// map, merging a fixed list of names — `TESTS`, `am__*`, the primaries —
/// and letting the last definition win for the rest. Five bugs were that
/// one decision seen from five consumers, each fixed at the consumer with a
/// per-directory read beside the flat map; see autotools-frontend.md, "The
/// flattened database is a bug class". This type is the per-directory read
/// made the only model.)*
pub(crate) struct VariableDatabase {
    build_root: PathBuf,
    /// The root first, then directories in the order make printed them —
    /// the order it finished walking the tree, and the only declaration
    /// order there is (tests are emitted in it). The root is pinned first
    /// rather than placed where its database appears, which is LAST in real
    /// output (the top-level make prints after every sub-make has returned),
    /// so a project's own top-level tests lead its list.
    scopes: Vec<(PathBuf, HashMap<String, String>)>,
}

impl VariableDatabase {
    /// Parses `make -p` output.
    ///
    /// Split from [`variable_database`] so a frozen real capture can drive
    /// it without a configured tree. Only `NAME = value` lines are kept;
    /// the database also holds rules, comments and `NAME := value` forms,
    /// none of which carry automake's declarations.
    pub(crate) fn parse(database: &str, build_root: &Path) -> Self {
        let build_root = build_root.to_path_buf();
        let mut scopes: Vec<(PathBuf, HashMap<String, String>)> =
            vec![(build_root.clone(), HashMap::new())];
        let mut index: HashMap<PathBuf, usize> = HashMap::from([(build_root.clone(), 0)]);
        let mut stack: Vec<PathBuf> = vec![build_root.clone()];
        for line in database.lines() {
            // The announcements are `# make[N]: ...` comment lines, so they
            // have to be read BEFORE the comment filter below.
            if line.trim_start_matches("# ").starts_with("make[") {
                if let Some(entered) = entering_directory(line) {
                    stack.push(entered);
                } else if leaving_directory(line) && stack.len() > 1 {
                    stack.pop();
                }
                continue;
            }
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
            let current = stack.last().expect("the root scope is never popped");
            let slot = *index.entry(current.clone()).or_insert_with(|| {
                scopes.push((current.clone(), HashMap::new()));
                scopes.len() - 1
            });
            scopes[slot]
                .1
                .insert(name.to_string(), value.trim().to_string());
        }
        Self { build_root, scopes }
    }

    pub(crate) fn build_root(&self) -> &Path {
        &self.build_root
    }

    /// The variables of the make that ran in `dir`, if one did.
    pub(crate) fn scope(&self, dir: &Path) -> Option<&HashMap<String, String>> {
        self.scopes
            .iter()
            .find(|(d, _)| d == dir)
            .map(|(_, vars)| vars)
    }

    /// Every scope, in first-seen order.
    pub(crate) fn scopes(&self) -> impl Iterator<Item = (&Path, &HashMap<String, String>)> {
        self.scopes.iter().map(|(d, vars)| (d.as_path(), vars))
    }

    /// The top-level make's scope: where a value `configure` substituted
    /// into every Makefile is read. Always present (empty at worst).
    pub(crate) fn root(&self) -> &HashMap<String, String> {
        &self.scopes[0].1
    }

    /// The scope a build-tree file belongs to: the nearest directory at or
    /// above it whose make printed a database, else the root's. A config
    /// header's `@VAR@`s are `configure` substitutions, present in every
    /// Makefile, so for jansson's `src/jansson_config.h` the `src` scope and
    /// the root agree; a template in a directory with no Makefile of its own
    /// (hwloc's `include/hwloc/autogen/config.h`) reads its ancestor's.
    pub(crate) fn scope_for(&self, path_in_build_tree: &str) -> &HashMap<String, String> {
        let mut dir = self.build_root.join(path_in_build_tree);
        while dir.pop() {
            if let Some(scope) = self.scope(&dir) {
                return scope;
            }
            if dir == self.build_root {
                break;
            }
        }
        self.root()
    }

    /// Every target automake declared, each primary expanded in the scope
    /// of the directory whose Makefile declared it.
    ///
    /// This is the identity source. The command stream shows `-o greeter`
    /// — a filename — while `bin_PROGRAMS = greeter` is the name automake
    /// actually uses, and the `bin_` prefix additionally states where it
    /// installs. Inferring identity from artifact paths instead breaks on a
    /// binary whose name differs from its target's, on libtool's
    /// `.libs/libfoo.so.1.0.0` versus `libfoo.la`, and on a non-empty
    /// `EXEEXT`.
    ///
    /// Per directory because that is where a primary's references resolve:
    /// automake compiles a conditional target through `$(am__EXEEXT_N)`,
    /// and the counter restarts in every Makefile — hwloc's `tests/hwloc`
    /// says `am__EXEEXT_2 = shmem` while `utils/hwloc` says something else
    /// under the same name. Declarations are then merged by name across
    /// directories ([`dedup_declarations`]): xz declares `bin_PROGRAMS` in
    /// four directories and every one of them is a target.
    pub(crate) fn declared_targets(&self) -> Vec<DeclaredTarget> {
        let mut targets = Vec::new();
        for (directory, scope) in self.scopes() {
            targets.extend(declared_targets_in(directory, scope));
        }
        dedup_declarations(targets)
    }
}

/// What an automake `TESTS` entry turns out to be.
///
/// Three variants because real projects use all three, measured across six:
/// `$(check_PROGRAMS)` naming binaries directly (libmicrohttpd, gsl, wget),
/// a shell script that drives binaries indirectly (jansson, xz), and a
/// project-specific variable nothing resolves (libgd, wget, gsl). The third
/// is load-bearing rather than a fallback — it is 2 of the 6.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum TestEntry {
    /// Names a `check_PROGRAMS` binary, run directly. Expressible as a test
    /// rule around a target this module already builds.
    Binary(String),
    /// A script that runs the tests itself. The binaries it drives are still
    /// built, but what to RUN is the script.
    Script(String),
    /// A reference the variable database does not answer. Escalated, never
    /// guessed: assuming it is a binary would invent a target, and dropping
    /// it would report fewer tests than the project has.
    Unresolved(String),
}

/// Classifies every entry in make's `TESTS` variable.
///
/// The evidence is the variable database, not the `Makefile.am`, for the same
/// reason the rest of this frontend reads resolved output — `TESTS` is
/// routinely a variable reference and the input text cannot say what it holds.
///
/// Expansion is TRANSITIVE but bounded. `make -p` reports
/// `TESTS = $(check_PROGRAMS)`, and libmicrohttpd's `check_PROGRAMS` in turn
/// carries `$(am__EXEEXT_1)` — automake's per-conditional indirection, the
/// same shape as the `$(am__append_N)` already handled for `_SOURCES`. One
/// level left 14 of its tests reported as the literal text
/// `$(am__EXEEXT_1)`. The bound is what keeps this from becoming make's own
/// expander: a reference that has already been followed is not followed
/// again, so a cycle terminates as `Unresolved` rather than looping.
/// Every directory's `TESTS`, each resolved against ITS OWN variables.
///
/// automake's `am__*` indirection is directory-scoped, so a `TESTS` must not
/// be resolved against the flattened database: libmicrohttpd defines
/// `am__EXEEXT_1` in six directories, and resolving `src/testcurl`'s
/// `TESTS = $(check_PROGRAMS) = $(am__EXEEXT_1)` against the union produced
/// 102 "tests" — including `doc/examples`' example programs, from a
/// directory that declares no `TESTS`. See [`VariableDatabase`] on why no
/// scope ever merges with another.
fn classify_tests_per_directory(db: &VariableDatabase) -> Vec<TestEntry> {
    let mut entries = Vec::new();
    for (directory, scope) in db.scopes() {
        // Where the declaring `Makefile.am` sits, relative to the module.
        // A `TESTS` entry names a file relative to ITS directory, so xz's
        // `tests/Makefile.am` saying `test_files.sh` means
        // `tests/test_files.sh` — and a bare name is what reaches the
        // escalation and the copier, both of which resolve from the module
        // root. Unqualified, both looked for it at the top level and found
        // nothing.
        let prefix = directory
            .strip_prefix(db.build_root())
            .ok()
            .filter(|rel| !rel.as_os_str().is_empty());
        for entry in classify_tests(scope) {
            let entry = match (&prefix, entry) {
                (Some(rel), TestEntry::Script(name)) if !name.contains('/') => {
                    TestEntry::Script(rel.join(&name).to_string_lossy().into_owned())
                }
                (_, other) => other,
            };
            if !entries.contains(&entry) {
                entries.push(entry);
            }
        }
    }
    entries
}

fn classify_tests(vars: &HashMap<String, String>) -> Vec<TestEntry> {
    let Some(raw) = vars.get("TESTS") else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    // (token, remaining expansion budget). automake's own nesting is two or
    // three deep (TESTS -> check_PROGRAMS -> am__EXEEXT_N); this is generous
    // and finite, which is all the bound has to be.
    const MAX_EXPANSION_DEPTH: u32 = 16;
    let mut queue: Vec<(String, u32)> = raw
        .split_whitespace()
        .map(|t| (t.to_string(), MAX_EXPANSION_DEPTH))
        .collect();
    queue.reverse();

    while let Some((token, depth)) = queue.pop() {
        // `$(check_PROGRAMS)` / `${VAR}`: resolve against the database and
        // push the expansion back on, so a reference inside it resolves too.
        if let Some(name) = token
            .strip_prefix("$(")
            .and_then(|t| t.strip_suffix(')'))
            .or_else(|| token.strip_prefix("${").and_then(|t| t.strip_suffix('}')))
        {
            match vars.get(name) {
                // The bound is on DEPTH, not on having seen the name before.
                // A repeat is not a cycle: `make -p` reports a directory's
                // database once per sub-make, so accumulation legitimately
                // yields `$(am__EXEEXT_2) ... $(am__EXEEXT_2)`, and refusing
                // the second occurrence shipped the literal text in an
                // escalation beside the tests it expands to.
                //
                // A self-referential variable still terminates: it re-enters
                // with one less budget each time and falls through to
                // Unresolved at zero.
                Some(expansion) if depth > 0 => {
                    let mut parts: Vec<(String, u32)> = expansion
                        .split_whitespace()
                        .map(|t| (t.to_string(), depth - 1))
                        .collect();
                    parts.reverse();
                    queue.extend(parts);
                }
                // automake's own indirection, undefined in this directory:
                // the conditional it stood for was false, and make expands
                // an undefined variable to nothing. hwloc's `check_PROGRAMS`
                // carries eleven of these for Windows, CUDA and friends.
                // Only automake's namespace gets that reading — a project's
                // own undefined variable is still escalated, because the
                // project may define it somewhere the database does not show.
                None if name.starts_with("am__") => {}
                // The database has no such name, or the budget ran out on a
                // self-reference. Either way this token cannot be resolved.
                _ => entries.push(TestEntry::Unresolved(token.to_string())),
            }
            continue;
        }
        // A bare name. A path or a known script extension is something to
        // RUN; anything else is a binary if the project builds one by that
        // name, and a script if it does not — the distinction jansson turns
        // on, where `run-suites` sits beside 18 real binaries.
        let bare = expand_exeext(&token, vars);
        let is_script = bare.contains('/')
            || matches!(
                Path::new(&bare).extension().and_then(|e| e.to_str()),
                Some("sh" | "py" | "pl" | "test")
            );
        if is_script {
            entries.push(TestEntry::Script(bare));
        } else if is_declared_check_program(vars, &bare) {
            entries.push(TestEntry::Binary(bare));
        } else {
            entries.push(TestEntry::Script(bare));
        }
    }
    // Deduplicated, first-seen order preserved. Accumulating TESTS across
    // directories means one declaration arrives once per sub-make that read
    // it — jansson's five `TESTS = ` lines are two distinct tests — and a
    // repeat is the same test seen twice, never a second one. Order is kept
    // rather than sorted because it is the order the project declared them
    // in, which is the only ordering evidence there is.
    let mut seen = HashSet::new();
    entries.retain(|e| seen.insert(e.clone()));
    entries
}

/// Whether `name` is one of the project's `check_PROGRAMS`, following the
/// same indirections `classify_tests` does.
///
/// Its own function because the membership test has to see through
/// `$(am__EXEEXT_N)` exactly as the expansion does; comparing against the raw
/// variable text would classify every conditional test as a script.
fn is_declared_check_program(vars: &HashMap<String, String>, name: &str) -> bool {
    let Some(raw) = vars.get("check_PROGRAMS") else {
        return false;
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = raw.split_whitespace().map(str::to_string).collect();
    while let Some(token) = queue.pop() {
        if let Some(var) = token
            .strip_prefix("$(")
            .and_then(|t| t.strip_suffix(')'))
            .or_else(|| token.strip_prefix("${").and_then(|t| t.strip_suffix('}')))
        {
            if let Some(expansion) = vars.get(var)
                && seen.insert(var.to_string())
            {
                queue.extend(expansion.split_whitespace().map(str::to_string));
            }
            continue;
        }
        if expand_exeext(&token, vars) == name {
            return true;
        }
    }
    false
}

/// Expands automake's `$(EXEEXT)` from the variable database, in either
/// bracket form.
///
/// From the database rather than assumed empty, even though it IS empty on
/// every platform this converts for today. The database states the answer, so
/// assuming one is the guessing this frontend exists not to do — and the two
/// answers have already disagreed once: `declared_targets` expanded while a
/// second copy stripped, so on a `.exe` toolchain a target named `prog.exe`
/// would never match a test entry named `prog`, every test would escalate as
/// inexpressible, and the project would convert with `tests: 0` and nothing
/// failing.
///
/// Both bracket forms, because they are one variable written two ways and
/// only one copy used to know that.
fn expand_exeext(token: &str, vars: &HashMap<String, String>) -> String {
    let exeext = vars.get("EXEEXT").map(String::as_str).unwrap_or("");
    token
        .replace("$(EXEEXT)", exeext)
        .replace("${EXEEXT}", exeext)
}

/// Expands `$(NAME)` / `${NAME}` references in a primary's value the way
/// make would, from the variable map, recursively.
///
/// libevent declares every library through one: `lib_LTLIBRARIES =
/// $(LIBEVENT_LIBS_LA)`, itself `libevent.la ... $(am__append_1)`, and its
/// programs through automake's `$(am__EXEEXT_N)` chain — so without this the
/// project declares nothing and converts to zero targets.
///
/// A reference the map does not define is left as written, and the caller
/// then drops it: for automake's own `am__*` names that is a conditional
/// that was false, and for anything else there is no answer to invent.
/// Substitution references (`$(V:.c=.o)`) are likewise left alone; no
/// primary has needed one. Depth-capped against a self-referencing
/// definition, which make would reject and this must not loop on.
fn expand_references(value: &str, vars: &HashMap<String, String>) -> String {
    fn go(value: &str, vars: &HashMap<String, String>, depth: u8) -> String {
        if depth == 0 || !value.contains('$') {
            return value.to_string();
        }
        let mut out = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(start) = rest.find('$') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let (close, body_start) = match after.chars().next() {
                Some('(') => (')', 1),
                Some('{') => ('}', 1),
                _ => {
                    out.push('$');
                    rest = after;
                    continue;
                }
            };
            let Some(end) = after.find(close) else {
                out.push_str(rest);
                return out;
            };
            let name = &after[body_start..end];
            let simple = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
            match vars.get(name) {
                Some(replacement) if simple => {
                    out.push_str(&go(replacement, vars, depth - 1));
                }
                _ => out.push_str(&rest[start..start + 1 + end + 1]),
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        out
    }
    go(value, vars, 8)
}

/// The targets ONE scope declares, from its primaries. The whole answer for
/// a single-directory project; [`VariableDatabase::declared_targets`] walks
/// every scope of a recursive one.
fn declared_targets_in(directory: &Path, vars: &HashMap<String, String>) -> Vec<DeclaredTarget> {
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
        // Expanded as a whole before splitting: one reference can stand for
        // several names.
        let value = expand_references(value, vars);
        for name in value.split_whitespace() {
            // Through the shared helper, not inline: a target's name and the
            // test entry naming it have to expand identically or the two
            // never match. They did not, once.
            let name = expand_exeext(name, vars);
            if name.is_empty() {
                continue;
            }
            // An unexpanded reference, not a target: an `am__*` name this
            // scope does not define is a conditional that was false, and a
            // project's own undefined variable has no answer to invent.
            // Taking one as a name produces a target automake never declared,
            // matched against no build command, escalated as unbuilt.
            if name.contains("$(") {
                continue;
            }
            targets.push(DeclaredTarget {
                name,
                destination: destination.to_string(),
                primary: primary.to_string(),
                directory: directory.to_path_buf(),
            });
        }
    }
    dedup_declarations(targets)
}

/// One declaration per NAME. automake lets a program appear under two
/// primaries — `EXTRA_PROGRAMS = foo` beside a conditional
/// `noinst_PROGRAMS += foo`, or `check_` beside `noinst_` — and it is one
/// target either way; libmicrohttpd's authorization_example rendered
/// twice, and two rules of one name is a load error. The destination
/// that says most about the target wins: an install destination over
/// `noinst` (never installed) over `check` (built by `make check` only)
/// over `EXTRA` (may be built), which is what automake itself does when
/// the same name is built by plain `make`.
fn dedup_declarations(mut targets: Vec<DeclaredTarget>) -> Vec<DeclaredTarget> {
    targets.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(destination_rank(&a.destination).cmp(&destination_rank(&b.destination)))
    });
    targets.dedup_by(|a, b| a.name == b.name);
    targets
}

/// How much a primary's destination says about a target, lowest first: an
/// install directory, `noinst`, `check`, `EXTRA`. See `dedup_declarations`.
fn destination_rank(destination: &str) -> u8 {
    match destination {
        "EXTRA" => 3,
        "check" => 2,
        "noinst" => 1,
        _ => 0,
    }
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

/// Splits make's output into the commands that actually build something.
///
/// Deliberately conservative about what it keeps. The stream is interleaved
/// with shell bookkeeping automake emits around every compile — `depbase=...`
/// assignments, `mv -f`, `rm -f`, `test -z || mkdir` — and with `make`'s own
/// directory chatter. Recognising only the handful of programs that produce
/// build outputs means an unrecognised line is IGNORED rather than
/// misinterpreted, which is the safe direction: a missed command shows up as a
/// target with no sources, while a misparsed one would silently produce a
/// wrong rule.
/// A system header a project generates a REPLACEMENT for — gnulib's
/// mechanism, recovered from the build's own generation recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplacementHeader {
    /// The generated header, as it will be included (`string.h`).
    pub(crate) output: String,
    /// The `.in.h` template, as the recipe named it.
    pub(crate) template: String,
    /// Every `@VAR@` the recipe substitutes, sorted by name.
    pub(crate) values: Vec<(String, String)>,
    /// Every whole file the recipe splices in, as `(marker, path)`, in
    /// recipe order — sed's `r`. See `model::ConfigHeader::splices`.
    pub(crate) splices: Vec<(String, String)>,
    /// The directory the recipe ran in, absolute.
    pub(crate) dir: PathBuf,
}

/// Recovers every replacement header from the build's `V=1` output.
///
/// gnulib ships `string.in.h` and generates a real `string.h` into the build
/// tree, placed ahead of `/usr/include` so `#include <string.h>` finds it and
/// its `#include_next` reaches the system one. Reproducing that is what lets
/// a converted module behave the way the project's build does on a platform
/// where the replacement is NOT inert — see docs/architecture/overview.md.
///
/// The stream is the only source that answers this. Two alternatives were
/// measured on libidn2 and both fail:
///
/// - The `<HEADER>_H` variables in `make -p` look like a per-header decision
///   (`ALLOCA_H = alloca.h`, `ASSERT_H` empty) but MISS six of the eleven —
///   there is no `STRING_H` at all — and INVENT `iconv.h`, which is set but
///   never generated. They state intent, not outcome.
/// - Counting the `<name>.h:` rules in the generated Makefile over-reports:
///   16 rules exist and 11 fire.
///
/// Requires `V=1`, because these recipes are silenced by `AM_SILENT_RULES` —
/// see [`build`], which passes it for a different reason that happens to
/// make this possible.
fn parse_replacement_headers(stdout: &str, build_dir: &Path) -> Vec<ReplacementHeader> {
    let mut found = Vec::new();
    let mut dir = build_dir.to_path_buf();
    // One recipe spans several lines, continued with a trailing backslash, so
    // substitutions accumulate until the redirect that ends it.
    let mut values: Vec<(String, String)> = Vec::new();
    let mut splices: Vec<(String, String)> = Vec::new();
    let mut template: Option<String> = None;

    for line in stdout.lines() {
        if let Some(entered) = entering_directory(line) {
            dir = entered;
            continue;
        }
        // Both delimiter forms appear in ONE recipe: `s|...|` where the value
        // may contain a slash (`<string.h>`), `s/.../` elsewhere. A parser
        // that knows one silently drops half the substitutions.
        for (name, value) in sed_substitutions(line) {
            values.push((name, value));
        }
        // Not a substitution: `r` splices a whole file in at a marker. Kept
        // ordered and undeduplicated, unlike `values` — see `sed_file_splices`.
        splices.extend(sed_file_splices(line));
        // The recipe redirects to a TEMP file and a later `mv` renames it —
        // gnulib writes atomically. So the redirect names the template and
        // the `mv` names the header, and both are needed: keying on the
        // redirect alone yields `string.h-t1`, which is not a header anyone
        // includes.
        //
        // THREE recipe forms, and libidn2 uses all three:
        //
        //   1. template on stdin:    `< foo.in.h > foo.h-t`   (4 headers)
        //   2. template positional:  `foo.in.h > foo.h-t`     (7 headers)
        //   3. sed's `w`, NO redirect:
        //        `sed ... -n -e 'w foo.h-t' ./foo.in.h`       (3 headers)
        //
        // Handling only the first found 4 of 11 and looked like it worked, so
        // the `<` is matched optionally. Form 3 has no `>` at all — `-n`
        // suppresses the default output and `w` does the writing — so keying
        // on the redirect missed exactly libidn2's uniconv.h, unistr.h and
        // unitypes.h, which is what the build then failed on.
        //
        // Its template is the trailing POSITIONAL argument, after the whole
        // script, so it is found the same way: the last `.in.h` token on the
        // line.
        let names_output = line.contains('>') || line.contains("-e 'w ");
        if names_output
            && let Some(src) = line.split_whitespace().rev().find(|t| t.ends_with(".in.h"))
        {
            template = Some(src.to_string());
        }
        if let Some(renamed) = atomic_rename(line)
            && let Some(template) = template.take()
        {
            values.sort();
            values.dedup();
            found.push(ReplacementHeader {
                output: renamed,
                template,
                values: std::mem::take(&mut values),
                splices: std::mem::take(&mut splices),
                dir: dir.clone(),
            });
        }
    }
    found
}

/// The module-relative directory a replacement header is generated into, or
/// `None` when it lands outside the module.
///
/// The recipe runs in the BUILD tree (`<build>/gl`), and the module is laid
/// out like the SOURCE tree, so the build-relative part is what carries
/// over — the same rebasing every other path in this frontend gets, and the
/// reason a raw `dir` cannot be used directly.
fn rebase_shadow_dir(header: &ReplacementHeader, module_root: &Path) -> Option<(String, String)> {
    // The template sits beside its output in the source tree, so its
    // directory is the one the module will have. Derived from the template
    // rather than from `dir` because `dir` is the build tree's copy, which
    // an out-of-tree build puts somewhere the module never mirrors.
    //
    // Returns the template MODULE-RELATIVE alongside the directory: the
    // recipe names it absolutely, and every path-valued field of the model
    // is required to be module-relative (`model::is_module_relative`).
    // Leaving it absolute made the source copier resolve it back to the
    // original file and try to copy it onto itself.
    let template = Path::new(&header.template);
    let rel_template = template.strip_prefix(module_root).ok()?.to_str()?;
    let rel_dir = Path::new(rel_template).parent()?.to_str()?;
    (!rel_dir.is_empty()).then(|| (rel_template.to_string(), rel_dir.to_string()))
}

/// Splice paths made module-relative, by joining them onto the directory the
/// recipe ran in.
///
/// Every path-valued field of the model must be module-relative
/// (`model::is_module_relative`), and the recipe names these BOTH ways
/// depending on how the build was invoked:
///
/// - `./c++defs.h`, relative to the directory the recipe ran in, for an
///   in-tree build;
/// - an absolute path under the source tree for an out-of-tree one, which is
///   how the translator itself builds every project.
///
/// Handling only the first emitted `gl/<absolute path>` — a concatenation
/// naming nothing — for all 14 of libidn2's splices, and Bazel reports that
/// as a missing label rather than as a bad path.
fn rebase_splices(
    splices: &[(String, String)],
    shadow_dir: &str,
    module_root: &Path,
) -> Vec<(String, String)> {
    splices
        .iter()
        .filter_map(|(marker, file)| {
            let path = Path::new(file);
            let rebased = if path.is_absolute() {
                path.strip_prefix(module_root).ok()?.to_str()?.to_string()
            } else {
                let bare = file.strip_prefix("./").unwrap_or(file);
                format!("{shadow_dir}/{bare}")
            };
            Some((marker.clone(), rebased))
        })
        .collect()
}

/// The final name in gnulib's atomic `mv <name>.h-t <name>.h`.
///
/// Its own step because the generation recipe redirects to the TEMP file, so
/// the header's real name appears only here.
fn atomic_rename(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("mv ")?;
    let rest = rest.strip_prefix("-f ").unwrap_or(rest);
    let (from, to) = rest.split_once(' ')?;
    let to = to.trim();
    if !from.contains(".h-t") || !to.ends_with(".h") {
        return None;
    }
    // The FILENAME, never the path the recipe wrote. gnulib's recipes run
    // inside the directory they generate into and so rename a bare
    // `string.h`, but a project whose recipe runs one level up writes
    // `mv gl/mystring.h-t gl/mystring.h`. `config_header_output` re-adds the
    // directory from `shadow_dir`, so keeping the path here produced
    // `output = "gl/gl/mystring.h"` — a header generated two levels down from
    // where its own include path points, and so unreachable.
    Path::new(to)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
}

/// Every `-e '/PATTERN/r FILE'` on one line, as `(pattern, file)`.
///
/// sed's `r` command inserts a whole file after each line matching the
/// pattern. gnulib uses it to assemble a generated header out of parts: the
/// template carries `/* The definitions of _GL_FUNCDECL_RPL etc. are copied
/// here. */` and the recipe splices `c++defs.h` in at that comment.
///
/// Nothing here is a substitution, so this is its own parser rather than a
/// branch of [`sed_substitutions`] — and the two run over the same line,
/// since one recipe uses both.
fn sed_file_splices(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = line;
    // Anchored on `-e '` rather than on `/`, because a bare `/` appears in
    // every path in the recipe. The quote also bounds the filename, which
    // may not be the last thing on the line.
    while let Some(at) = rest.find("-e '/") {
        let after = &rest[at + "-e '/".len()..];
        let Some((quoted, remainder)) = after.split_once('\'') else {
            break;
        };
        rest = remainder;
        // `/PATTERN/r FILE`. Split at the LAST `/r ` so a pattern containing
        // one does not truncate it.
        if let Some((pattern, file)) = quoted.rsplit_once("/r ")
            && !pattern.is_empty()
            && !file.trim().is_empty()
        {
            out.push((pattern.to_string(), file.trim().to_string()));
        }
    }
    out
}

/// Every `s|@''NAME''@|value|` and `s/@''NAME''@/value/g` on one line.
///
/// The doubled quotes are automake's own escaping, present so `configure`
/// does not substitute the placeholder while writing the Makefile.
fn sed_substitutions(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (open, close) in [('|', '|'), ('/', '/')] {
        let marker = format!("s{open}@''");
        let mut rest = line;
        while let Some(at) = rest.find(&marker) {
            let after = &rest[at + marker.len()..];
            let Some((name, tail)) = after.split_once("''@") else {
                break;
            };
            let Some(tail) = tail.strip_prefix(close) else {
                rest = after;
                continue;
            };
            let Some((value, remainder)) = tail.split_once(close) else {
                break;
            };
            if name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            {
                out.push((name.to_string(), value.to_string()));
            }
            rest = remainder;
        }
    }
    out
}

pub(crate) fn parse_commands(stdout: &str, build_dir: &Path) -> Vec<BuildCommand> {
    let mut commands = Vec::new();
    // make announces each descent, and the announcements nest — `make[2]`
    // inside `make[1]`. Tracked as a stack rather than a single "current"
    // value so a Leaving line restores the enclosing directory rather than
    // guessing at the root.
    let mut dirs: Vec<PathBuf> = vec![build_dir.to_path_buf()];
    for line in stdout.lines() {
        if let Some(dir) = entering_directory(line) {
            dirs.push(dir);
            continue;
        }
        if leaving_directory(line) {
            // Never pop the base: an unbalanced Leaving (a sub-make whose
            // Entering was filtered out, say) must not leave the stack empty.
            if dirs.len() > 1 {
                dirs.pop();
            }
            continue;
        }
        let current = dirs.last().expect("the base directory is never popped");
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
                dir: current.clone(),
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

/// The directory a `make[N]: Entering directory '...'` line announces.
fn entering_directory(line: &str) -> Option<PathBuf> {
    let rest = line.split_once("Entering directory ")?.1;
    Some(PathBuf::from(rest.trim().trim_matches(&['\'', '`'][..])))
}

/// Whether a line announces make leaving a directory.
fn leaving_directory(line: &str) -> bool {
    line.contains("Leaving directory ")
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
/// Not a shell parser: make's output is already expanded, so the only job
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
///
/// Returns the module root it chose alongside the graph, because that root is
/// DERIVED: it widens from `source_dir` to cover anything the build references
/// from inside `deliverable_root`, and callers have to rebase against the same
/// answer. Same rule as `cmake_api::rebase_to_module_root` — see
/// docs/architecture/build-verification.md on why a wider root is the
/// resolution rather than a workaround.
/// [`to_graph_with_dependencies`] for a project converted on its own — every
/// test's case, and the one where a library the project does not build can
/// only be escalated.
#[cfg(test)]
pub(crate) fn to_graph(
    commands: &[BuildCommand],
    db: &VariableDatabase,
    project_name: &str,
    source_dir: &Path,
    deliverable_root: &Path,
) -> (
    BuildGraph,
    Vec<crate::needs_attention::NeedsAttention>,
    PathBuf,
) {
    to_graph_with_dependencies(
        commands,
        db,
        project_name,
        source_dir,
        deliverable_root,
        &Dependencies::default(),
    )
}

pub(crate) fn to_graph_with_dependencies(
    commands: &[BuildCommand],
    db: &VariableDatabase,
    project_name: &str,
    source_dir: &Path,
    deliverable_root: &Path,
    deps: &Dependencies,
) -> (
    BuildGraph,
    Vec<crate::needs_attention::NeedsAttention>,
    PathBuf,
) {
    let build_root = db.build_root();
    let declared = db.declared_targets();
    // artifact basename -> the link/archive command that produced it, so a
    // declared target can find the step that built it. Needed before the
    // module root can be chosen, because the root depends on where each
    // target's sources resolve to and that is per-command.
    let mut built: HashMap<String, &BuildCommand> = HashMap::new();
    for cmd in commands {
        if let Some(artifact) = produced_artifact(cmd) {
            built.insert(basename(&artifact), cmd);
        }
    }

    // The directory a target's `_SOURCES` are declared relative to: the
    // Makefile.am that declared them, expressed in the SOURCE tree.
    let declaring_dir = |name: &str| -> PathBuf {
        built
            .get(&basename(name))
            .and_then(|cmd| cmd.dir.strip_prefix(build_root).ok())
            .map(|rel| source_dir.join(rel))
            .unwrap_or_else(|| source_dir.to_path_buf())
    };
    // A per-target variable, read from the scope that DECLARED the target
    // — see `DeclaredTarget::directory` for the collision that makes the
    // scope necessary. No fallback to any other scope: a name this scope
    // lacks is one the declaring Makefile did not set.
    let target_var = |decl: &DeclaredTarget, key: &str| -> Option<&String> {
        db.scope(&decl.directory).and_then(|scope| scope.get(key))
    };

    // The module root, widened to cover anything the build references from
    // inside the deliverable. Same rule as `cmake_api::rebase_to_module_root`
    // and the same reason: a Bazel label cannot reach above its own module, so
    // a project compiling a sibling directory's sources needs a root that
    // contains both. `deliverable_root` caps the widening — a file outside it
    // cannot be reproduced from what the project ships, so the module is not
    // grown to swallow it and it is escalated instead.
    //
    // Surveyed before anything is rebased, unlike the rest of this function,
    // because the root is a fact about ALL the targets and rebasing needs it
    // already decided.
    // object path -> the source it was compiled from, and the flags it
    // carried. Built before the module root, because the root survey needs it
    // to see through an unexpanded `_SOURCES`.
    let mut source_of: HashMap<String, (String, PathBuf)> = HashMap::new();
    let mut flags_of: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
    for cmd in commands.iter().filter(|c| c.program != "ar") {
        let Some(output) = flag_value(&cmd.args, "-o") else {
            continue;
        };
        if !cmd.args.iter().any(|a| a == "-c") {
            continue;
        }
        if let Some(source) = compiled_source(&cmd.args) {
            source_of.insert(output.clone(), (source, cmd.dir.clone()));
            flags_of.insert(output, (includes_of(&cmd.args), defines_of(&cmd.args)));
        }
    }

    let module_root = {
        let mut shipped = Vec::new();
        for decl in &declared {
            let dir = declaring_dir(&decl.name);
            let canon = canonical_name(&decl.name);
            let raw = target_var(decl, &format!("{canon}_SOURCES"))
                .map(String::as_str)
                .unwrap_or_default();
            // Same fallback the target loop uses, and for the same reason: an
            // unexpanded `$(am__append_N)` cannot say what the sources are, so
            // surveying the literal text picks the root from xz's 11-file base
            // while the graph is built from all 80. A conditional source
            // reaching a sibling directory would then be dropped by a root
            // that never widened for it.
            let sources: Vec<String> = if raw.contains("$(") {
                link_inputs(built.get(&basename(&decl.name)).copied())
                    .iter()
                    .filter_map(|obj| source_of.get(obj.as_str()))
                    .map(|(source, from)| {
                        normalize_lexically(&from.join(source))
                            .to_string_lossy()
                            .into_owned()
                    })
                    .collect()
            } else {
                raw.split_whitespace().map(str::to_string).collect()
            };
            // Everything the build REFERENCES, not just what it compiles.
            // CMake surveys sources, public headers and include dirs
            // together; surveying only sources here meant an Autotools
            // project whose one outside reference was an installed header or
            // an `-I` did not widen where its CMake equivalent did — two
            // structurally identical projects converting differently, which
            // is what bzl-kga was filed about.
            let headers = public_headers(db);
            let includes = built
                .get(&basename(&decl.name))
                .map(|cmd| includes_of(&cmd.args))
                .unwrap_or_default();
            for path in sources.iter().chain(&headers).chain(&includes) {
                let absolute = normalize_lexically(&dir.join(path));
                if absolute.starts_with(deliverable_root) {
                    shipped.push(absolute);
                }
            }
        }
        common_ancestor(source_dir, &shipped)
    };
    let module_root = &module_root;

    // Resolve a path reported by a command against the directory that command
    // ran in, then express it relative to the module root. Returns None when
    // it escapes the module, which a Bazel label cannot express.
    // Two roots, because an out-of-tree build reports paths against both. A
    // compile in <build>/src/lzmainfo carries `-I../..` (the build root) AND
    // `-I<source>/src/common` (the source tree) on the same line, and both
    // name real directories. Resolving against the command's directory then
    // trying each root is what makes them comparable; the build-root form
    // maps onto the source tree because the two trees mirror each other.
    let rebase = |path: &str, dir: &Path| -> Option<String> {
        let absolute = normalize_lexically(&dir.join(path));
        if let Ok(rel) = absolute.strip_prefix(module_root) {
            return Some(rel.to_string_lossy().into_owned());
        }
        absolute
            .strip_prefix(build_root)
            .ok()
            .map(|rel| rel.to_string_lossy().into_owned())
    };
    // object path -> the source it was compiled from, and the flags it carried.
    // Targets automake declared but the build never produced. `check_PROGRAMS`
    // are the live case: `make` does not build them (that is `make check`), so
    // no link command exists and nothing says which directory their sources
    // are relative to. Skipping them is honest — a target with sources
    // resolved against a guessed directory is worse than an absent one, and
    // the guess fails loudly on copy, which is how this was found. Recorded
    // for the escalation they deserve.
    let mut unbuilt: Vec<String> = Vec::new();
    let mut targets = Vec::new();
    // Libraries a target links that neither this project nor any converted
    // dependency builds, per target: the escalation has to name the rule
    // that is now incomplete, not just the library.
    let mut external_links: Vec<(String, Vec<String>)> = Vec::new();
    // Sources a target compiles that lie outside the module, paired with the
    // target that lost them: a dropped source is a link failure, and the
    // escalation has to say which rule is now incomplete. Flattening these
    // into one module-wide list would throw away exactly that.
    let mut outside_module: Vec<(String, Vec<String>)> = Vec::new();
    for decl in &declared {
        let canon = canonical_name(&decl.name);
        if built.get(&basename(&decl.name)).is_none() {
            unbuilt.push(decl.name.clone());
            continue;
        }

        let decl_dir = declaring_dir(&decl.name);

        // Sources come from the DECLARATION, not from the link inputs: a
        // target's _SOURCES is what automake was told, while the link line
        // shows objects, which have to be mapped back. Headers appear in
        // _SOURCES too (automake accepts them so they reach the tarball), so
        // they are split out rather than compiled.
        //
        // Unless the declaration is UNEXPANDED, which is where that preference
        // inverts. automake compiles conditional sources by appending
        // `$(am__append_N)` to `_SOURCES`, one per `if`, and make's database
        // reports those references verbatim. xz's liblzma is 45 of them around
        // an 11-file base: taking the literal text yields 11 sources, the
        // library builds, and the link fails on ~70 undefined symbols far from
        // the cause. The command stream has all 80 — this is exactly the split
        // the module doc describes, so when the declaration cannot answer, the
        // stream does.
        let declared_raw = target_var(decl, &format!("{canon}_SOURCES"))
            .map(String::as_str)
            .unwrap_or_default();
        let declared_sources: Vec<String> = if declared_raw.contains("$(") {
            link_inputs(built.get(&basename(&decl.name)).copied())
                .iter()
                .filter_map(|obj| source_of.get(obj.as_str()))
                .map(|(source, dir)| {
                    // Back to the declaring directory's frame, since that is
                    // what the non-conditional path produces and everything
                    // downstream rebases from there.
                    let absolute = normalize_lexically(&dir.join(source));
                    pathdiff(&absolute, &decl_dir)
                })
                .collect()
        } else {
            declared_raw
                .split_whitespace()
                .map(str::to_string)
                .collect()
        };
        // Three categories, not two. Partitioning on `!is_translation_unit`
        // made every unrecognised extension a header, so a `.c++` the
        // predicate did not know about was filtered against public_headers,
        // found not installed, and dropped with no escalation — surfacing as
        // an undefined symbol at link time.
        //
        // The third category is real: automake permits a README or a
        // ChangeLog in `_SOURCES`, the same IDE convenience CMake allows.
        // Dropped rather than escalated, and that decision is DELIBERATELY
        // the CMake frontend's — `is_buildable_source` is the shared
        // predicate and `cmake_api`'s use of it carries the argument (make
        // does not build the file either, so nothing is missing, and an
        // escalation per README would be noise on most projects).
        //
        // Routed through the shared predicate rather than reproduced, because
        // the two frontends silently disagreeing about what a source IS is
        // how the `.c++` bug happened.
        let (declared_sources, not_compiled): (Vec<String>, Vec<String>) = declared_sources
            .into_iter()
            .partition(|s| is_translation_unit(s));
        let headers: Vec<String> = not_compiled
            .iter()
            .filter(|s| is_header_file(Path::new(s)))
            .cloned()
            .collect();
        // A file in neither bucket that `is_buildable_source` WOULD accept
        // means the two predicates have drifted: the shared one knows an
        // extension the translation-unit list does not, and this frontend is
        // about to drop a file rules_cc could have built. Asserted rather
        // than collected, because there is no correct runtime behaviour here
        // — either the lists agree or the translator has a bug, and a fourth
        // silently-collected list is what this bead was filed about.
        debug_assert!(
            !not_compiled
                .iter()
                .any(|p| !is_header_file(Path::new(p)) && is_buildable_source(p)),
            "a _SOURCES entry is buildable but neither a translation unit nor \
             a header, so it would be dropped silently — headers.rs's two \
             predicates have drifted: {not_compiled:?}"
        );

        // A target's _SOURCES are relative to the Makefile.am that DECLARED
        // them — fixture 002 declares `tool_SOURCES = main.c ../common/util.c`
        // in app/, so `../common/util.c` means nothing until resolved against
        // app/. The link command runs in that same directory, but in the BUILD
        // tree; for an out-of-tree build the sources live under the SOURCE
        // tree, so the build-relative part of that directory is what carries
        // over.
        let mut sources: Vec<String> = declared_sources
            .iter()
            .filter_map(|src| rebase(src, &decl_dir))
            .collect();
        // Anchored, not absolute: this string ships in an escalation to an
        // agent who cannot see this machine, and under Bazel the raw path is
        // a sandbox directory with a run-specific number in it. The CMake
        // frontend has done this since bzl-0vq; the shared helper is what
        // stops the two answering differently again (bzl-ti2).
        let mut escaped: Vec<String> = declared_sources
            .iter()
            .filter(|src| rebase(src, &decl_dir).is_none())
            .map(|src| {
                anchor_for_display(
                    &normalize_lexically(&decl_dir.join(src)),
                    build_root,
                    deliverable_root,
                )
            })
            .collect();

        // Flags come from the COMMAND STREAM, via the objects this target's
        // sources compiled to — the database's *_CPPFLAGS are pre-expansion
        // and miss what configure and AM_CPPFLAGS contributed.
        let mut includes = Vec::new();
        let mut local_defines = Vec::new();
        let mut needs_root_include = false;
        for (object, (source, dir)) in &source_of {
            // Compare in module-relative terms: the object map keys sources as
            // the command reported them, which is per-directory.
            if !rebase(source, dir).is_some_and(|s| sources.contains(&s)) {
                continue;
            }
            if let Some((inc, def)) = flags_of.get(object) {
                // An include dir is likewise relative to the compiling
                // directory. One that resolves TO the module root is recorded
                // as `needs_root_include`, the way the CMake frontend records
                // it, rather than as an `includes` entry of "." — see that
                // field's doc for why the two are kept apart.
                for i in inc {
                    match rebase(i, dir) {
                        Some(rel) if rel.is_empty() => needs_root_include = true,
                        Some(rel) => includes.push(rel),
                        // An include path outside the module — a system or
                        // toolchain directory. Dropped rather than emitted:
                        // an absolute build-machine path in `includes` is
                        // accepted by Bazel silently and makes the module
                        // build only where it was converted.
                        None => {}
                    }
                }
                local_defines.extend(def.iter().cloned());
            }
        }

        let link = built.get(&basename(&decl.name));
        // Relative to the BUILD dir, not the module: this is where the real
        // built binary sits, and copy_ground_truth_artifacts reads it from
        // there. The link command reports `-o tool` from inside app/, so
        // without rebasing the artifact records as bare `tool` and the copy
        // fails on a path that does not exist.
        let artifact = link
            .and_then(|cmd| {
                let out = produced_artifact(cmd)?;
                let absolute = normalize_lexically(&cmd.dir.join(&out));
                absolute
                    .strip_prefix(build_root)
                    .ok()
                    .map(|rel| rel.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| decl.name.clone());

        // Dependencies come from the COMMAND STREAM, not from _LDADD.
        //
        // The database reports LDADD unexpanded — GNU hello's is literally
        //   hello_LDADD = $(LIBINTL) $(top_builddir)/lib/lib$(PACKAGE).a
        // so matching its tokens against declared names finds nothing and
        // drops every edge in silence. That is the same lesson cmake_api
        // records about CMakeLists.txt: the declaration is the unresolved
        // half. The link line carries the resolved `./lib/libhello.a`.
        let mut dependencies: Vec<String> = Vec::new();
        let mut external_dependencies: Vec<ExternalDependency> = Vec::new();
        let mut unresolved: Vec<String> = Vec::new();
        let link_args = link.map(|cmd| cmd.args.as_slice()).unwrap_or(&[]);
        let link_dir = link.map(|cmd| cmd.dir.clone()).unwrap_or_default();
        // The `-L` directories, resolved against the directory the link ran
        // in, so a `-l<name>` can be looked up the way the linker looks it
        // up. Collected first because `-L` may follow the `-l` it serves.
        let search_dirs: Vec<PathBuf> = link_args
            .iter()
            .filter_map(|a| a.strip_prefix("-L"))
            .filter(|d| !d.is_empty())
            .map(|d| link_dir.join(d))
            .collect();
        for input in link_args {
            if let Some(name) = input.strip_prefix("-l").filter(|n| !n.is_empty()) {
                match deps.resolve_name(name, &search_dirs) {
                    Some(external) => external_dependencies.push(external),
                    // Not the toolchain's own runtime: `-lm` is not a
                    // dependency anyone converts, and the llvm toolchain
                    // brings its own.
                    None if !is_toolchain_library(name) => unresolved.push(input.clone()),
                    None => {}
                }
                continue;
            }
            if !is_library(input) {
                continue;
            }
            if let Some(external) = deps.resolve_path(&link_dir.join(input)) {
                external_dependencies.push(external);
                continue;
            }
            match declared
                .iter()
                .find(|d| basename(&d.name) == basename(input))
            {
                Some(dep) if dep.name != decl.name => dependencies.push(target_label(&dep.name)),
                Some(_) => {}
                // A library the project links but does not build — $(LIBINTL)
                // resolves to a system libintl, for instance. Collected rather
                // than dropped: it is an input the generated module cannot
                // satisfy, which is an escalation, not a silent omission.
                None => unresolved.push(input.clone()),
            }
        }
        external_dependencies.sort_by(|a, b| (&a.module, &a.target).cmp(&(&b.module, &b.target)));
        external_dependencies.dedup();
        unresolved.sort_unstable();
        unresolved.dedup();
        if !unresolved.is_empty() {
            external_links.push((target_label(&decl.name), unresolved));
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
            name: target_label(&decl.name),
            kind: match decl.primary.as_str() {
                "PROGRAMS" => TargetKind::Executable,
                _ => TargetKind::Library,
            },
            // From the PRIMARY and the DESTINATION together, not from a
            // filename. LTLIBRARIES is libtool's form and LIBRARIES a plain
            // archive — but `noinst_` overrides that: a libtool library that
            // is never installed is a CONVENIENCE archive, absorbed into
            // whatever links it, and libtool builds no `.so` for one at all.
            //
            // libidn2's libgnu.la is the case. Treating it as shared emitted
            // a cc_shared_library for it AND absorbed it into libidn2's, and
            // Bazel rejects that outright: "Two shared libraries in
            // dependencies link the same library statically."
            is_shared: decl.primary == "LTLIBRARIES" && decl.destination != "noinst",
            // Against the BUILD tree and the declaring directory: automake
            // names a target `liblzma.la` while the file is written to
            // `src/liblzma/liblzma.la`, so the build root alone does not
            // find it.
            soname: libtool_dlname(
                &build_root
                    .join(decl_dir.strip_prefix(module_root).unwrap_or(Path::new("")))
                    .join(&decl.name),
            ),
            sources,
            // `include_HEADERS` names the project's public headers, the same
            // statement CMake makes with install(FILES ... DESTINATION
            // include). A header in _SOURCES that is not installed stays
            // private.
            // Rebased like every other path-valued field. Matching happens
            // on the RAW declaration text, because `include_HEADERS` and
            // `_SOURCES` both name paths relative to the declaring
            // Makefile.am — but what is STORED must be module-relative, which
            // is the contract `model::Target` states and
            // `codegen::render_path_list` asserts. Un-rebased, a subdirectory
            // header yielded a label pointing at nothing, and a `../` one
            // panicked that assert.
            public_headers: headers
                .iter()
                .filter(|h| public_headers(db).contains(*h))
                .filter_map(|h| rebase(h, &decl_dir))
                .collect(),
            dependencies,
            external_dependencies,
            strip_include_prefix: None,
            includes,
            local_defines,
            needs_root_include,
            artifacts: vec![artifact],
            ..Default::default()
        });
        escaped.sort_unstable();
        escaped.dedup();
        if !escaped.is_empty() {
            outside_module.push((decl.name.clone(), escaped));
        }
    }

    targets.sort_by(|a, b| a.name.cmp(&b.name));
    external_links.sort_unstable();
    external_links.dedup();
    outside_module.sort_by(|a, b| a.0.cmp(&b.0));
    unbuilt.sort_unstable();
    unbuilt.dedup();

    // A source that escaped the module is the one drop that fails silently.
    // The target still renders, still links, and reports an undefined symbol
    // several steps from the cause; a system library that fails to resolve
    // announces itself at link. (A missing check_PROGRAMS used to be listed
    // here as self-announcing too. jansson disproved that: it converted to
    // one target and `tests: 0`, a positive claim rather than an absence.)
    // Escalated per target because the resolution is per target: which rule is
    // incomplete is the first thing the agent needs.
    let mut needs_attention: Vec<_> = outside_module
        .iter()
        .map(|(target, sources)| sources_outside_deliverable_needs_attention(target, sources))
        .collect();

    // Automake's TESTS, split by what each entry turns out to be. A Binary
    // names a check_PROGRAMS this module builds, so it becomes a real test; a
    // Script or an Unresolved reference cannot be expressed as a rule around
    // a target, so it is carried on `unexpressed_tests` and escalated.
    //
    // `working_directory` is the module root for all of them. automake runs a
    // test from the directory its Makefile.am declared it in, and that is
    // where the built binary already sits, so there is no CTest-style
    // WORKING_DIRECTORY to rebase.
    let (mut tests, mut unexpressed_tests) = (Vec::new(), Vec::new());
    for entry in classify_tests_per_directory(db) {
        match entry {
            TestEntry::Binary(name) => {
                let label = target_label(&name);
                // Only if the module actually builds it. A TESTS entry naming
                // a binary no target produced would render a rule pointing at
                // nothing, which fails at analysis rather than saying why.
                if targets.iter().any(|t| t.name == label) {
                    tests.push(Test {
                        name: name.clone(),
                        target: label,
                        command: name,
                        working_directory: String::new(),
                        // automake has no PASS_REGULAR_EXPRESSION analogue:
                        // the exit code alone decides, which is what None
                        // already means.
                        pass_regex: None,
                    });
                } else {
                    unexpressed_tests.push(Test {
                        name: name.clone(),
                        target: String::new(),
                        command: name,
                        working_directory: String::new(),
                        // automake has no PASS_REGULAR_EXPRESSION analogue:
                        // the exit code alone decides, which is what None
                        // already means.
                        pass_regex: None,
                    });
                }
            }
            TestEntry::Script(path) | TestEntry::Unresolved(path) => {
                unexpressed_tests.push(Test {
                    name: path.clone(),
                    target: String::new(),
                    command: path,
                    working_directory: String::new(),
                    pass_regex: None,
                });
            }
        }
    }
    // A shared library that links a static one from the same module. Bazel
    // rejects it at ANALYSIS, naming generated rules rather than anything in
    // the project, so without this the user sees a failure they cannot
    // connect to what they wrote.
    //
    // Escalated rather than resolved because the translator has the facts and
    // not the judgement: it knows automake declared the archive `noinst_` and
    // that the shared library links it, but not whether the archive is an
    // implementation detail to fold in or an interface to export. The
    // relationship is also selective — the linker takes only referenced
    // members — so no single Bazel attribute states it exactly.
    let static_libraries: HashSet<&str> = targets
        .iter()
        .filter(|t| matches!(t.kind, TargetKind::Library) && !t.is_shared)
        .map(|t| t.name.as_str())
        .collect();
    for target in targets.iter().filter(|t| t.is_shared) {
        let absorbed: Vec<String> = target
            .dependencies
            .iter()
            .filter(|d| static_libraries.contains(d.as_str()))
            .cloned()
            .collect();
        if !absorbed.is_empty() {
            needs_attention.push(shared_library_absorbs_static_needs_attention(
                &target.name,
                &absorbed,
            ));
        }
    }
    if !unexpressed_tests.is_empty() {
        needs_attention.push(ctest_command_not_a_target_needs_attention(
            &unexpressed_tests
                .iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>(),
            &unexpressed_tests
                .iter()
                .map(|t| t.command.clone())
                .collect::<Vec<_>>(),
            // Resolved against the SOURCE tree, which is what decides
            // whether the file can be copied into the module at all. A
            // `.test` wrapper automake generates into the build tree is
            // absent here, and that is the case the item must not describe
            // as "ships with the sources".
            &unexpressed_tests
                .iter()
                .map(|t| source_dir.join(&t.command).is_file())
                .collect::<Vec<_>>(),
            TestDialect::AutomakeTests,
        ));
    }

    // A library some target links that nothing converted builds. One item per
    // LIBRARY, naming every target that links it: the resolution is one action
    // (convert that library's project) however many rules wait on it, and
    // libmicrohttpd's 65 test binaries each linking libcurl would otherwise
    // be 65 copies of one instruction. Escalated rather than linked from the
    // host: `-lcurl` as a linkopt would resolve against whatever this machine
    // has installed, which is the hermeticity the llvm toolchain exists to
    // rule out (bzl-x8l).
    let mut by_library: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (target, libraries) in &external_links {
        for library in libraries {
            by_library
                .entry(library.clone())
                .or_default()
                .push(target.clone());
        }
    }
    for (library, targets) in &by_library {
        needs_attention.push(unconverted_dependency_needs_attention(library, targets));
    }
    // Not yet escalated — see bzl-yjn.5. Recorded here so the information is
    // recovered rather than discarded, and so the escalation, when it lands,
    // has something to report.
    let _ = &unbuilt;

    (
        BuildGraph {
            module: ModuleInfo {
                name: project_name.to_string(),
                version: db.root().get("VERSION").cloned(),
            },
            targets,
            tests,
            // A TESTS entry this frontend cannot express: a script that runs
            // the suite itself, or a variable reference the database does not
            // answer. Carried rather than dropped so the module still gets
            // `rules_shell` and the agent has something to write an `sh_test`
            // against — the same role these play on the CMake side.
            unexpressed_tests,
            config_headers: Vec::new(),
            // Filled by the driver, which applies the one graph-level rule
            // both frontends share; see main.rs.
            displaced_sources: Vec::new(),
            dependencies: Vec::new(),
        },
        needs_attention,
        module_root.clone(),
    )
}

/// Moves every installed header a LIBRARY target carries in `sources` into
/// its `public_headers`, and records the prefix to strip so it is reachable
/// at its INSTALLED path. A binary keeps them private: nothing depends on a
/// binary's headers.
///
/// Matched by path SUFFIX: a `*_HEADERS` value is relative to the
/// Makefile.am that declared it, whose directory the promotion
/// no longer knows, while the target's paths are module-relative — so
/// `src/greet.h` declared at the root and carried as `src/greet.h` match,
/// and so would `greet.h` declared in `src/Makefile.am`. Only headers a
/// target already carries are promoted: a declared-public header no target
/// could see is left alone rather than attached by guesswork.
fn promote_installed_headers(targets: &mut [Target], db: &VariableDatabase) {
    let installed = installed_headers(db);
    for target in targets.iter_mut().filter(|t| t.kind == TargetKind::Library) {
        let mut strips: Vec<Option<String>> = Vec::new();
        let (public, private): (Vec<String>, Vec<String>) = std::mem::take(&mut target.sources)
            .into_iter()
            .partition(|s| {
                if !is_header_file(Path::new(s)) {
                    return false;
                }
                let Some((_, subdir)) = installed
                    .iter()
                    .find(|(raw, _)| s == raw || s.ends_with(&format!("/{raw}")))
                else {
                    return false;
                };
                strips.push(strip_for(s, subdir));
                true
            });
        target.sources = private;
        for header in public {
            if !target.public_headers.contains(&header) {
                target.public_headers.push(header);
            }
        }
        target.public_headers.sort_unstable();
        // One prefix per target, so every promoted header has to agree.
        strips.sort_unstable();
        strips.dedup();
        target.strip_include_prefix = match strips.as_slice() {
            [Some(prefix)] => Some(prefix.clone()),
            _ => None,
        };
    }
}

/// The prefix to strip from `path` so it is reachable at `subdir/<basename>`,
/// its installed name: `include/event2/buffer.h` installed as
/// `event2/buffer.h` strips `include`. `None` when the path does not end in
/// the installed name (the layout is not one strip away) or when there is
/// nothing to strip.
fn strip_for(path: &str, subdir: &str) -> Option<String> {
    let installed = if subdir.is_empty() {
        basename(path)
    } else {
        format!("{subdir}/{}", basename(path))
    };
    if path == installed {
        return None;
    }
    let prefix = path.strip_suffix(&format!("/{installed}"))?;
    (!prefix.is_empty()).then(|| prefix.to_string())
}

/// Every header the project installs under its include directory, as
/// `(path as declared, subdirectory of <includedir> it lands in)`.
///
/// automake's rule: `<name>_HEADERS` installs each file's basename into
/// `$(<name>dir)`, and only an install directory under `$(includedir)` makes
/// a header PUBLIC — `include_HEADERS` lands in `<includedir>` itself, libevent's
/// `include_event2_HEADERS` in `<includedir>/event2` because it declares
/// `include_event2dir = $(includedir)/event2`. `noinst_`/`EXTRA_` are not
/// installed; `nobase_` keeps the declared path and is left alone since its
/// installed name is not one strip away. Values are expanded first, because
/// libevent declares both lists through variables.
///
/// Every scope contributes: a `_HEADERS` primary is declared per directory
/// like any other (hwloc's in `include/`, libevent's at the root), and its
/// `<name>dir` is defined in the same Makefile, so both are read from one
/// scope. Paths are as the declaring Makefile.am wrote them; the callers
/// match them by suffix against a target's module-relative sources.
fn installed_headers(db: &VariableDatabase) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (_, vars) in db.scopes() {
        out.extend(installed_headers_in(vars));
    }
    out.sort();
    out.dedup();
    out
}

/// One scope's contribution to [`installed_headers`].
fn installed_headers_in(vars: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (var, value) in vars {
        let Some(prefix) = var.strip_suffix("_HEADERS") else {
            continue;
        };
        // `dist_`/`nodist_` say whether the file ships in the tarball, not
        // where it installs.
        let prefix = prefix
            .strip_prefix("dist_")
            .or_else(|| prefix.strip_prefix("nodist_"))
            .unwrap_or(prefix);
        if prefix.starts_with("noinst")
            || prefix.starts_with("EXTRA")
            || prefix.starts_with("nobase_")
        {
            continue;
        }
        let subdir = if prefix == "include" {
            String::new()
        } else {
            // Expanded with `includedir` pinned to a marker, because the
            // database defines it (`${prefix}/include`) and a plain expansion
            // turns `$(includedir)/hwloc` into `/usr/local/include/hwloc`,
            // where nothing says which part was the include directory. Only
            // a directory UNDER includedir makes a header public.
            const MARKER: &str = "\u{1}INCLUDEDIR\u{1}";
            let mut pinned = vars.clone();
            pinned.insert("includedir".to_string(), MARKER.to_string());
            let dir = vars
                .get(&format!("{prefix}dir"))
                .map(|d| expand_references(d, &pinned))
                .unwrap_or_default();
            let Some(rest) = dir.strip_prefix(MARKER) else {
                continue;
            };
            rest.trim_matches('/').to_string()
        };
        for raw in expand_references(value, vars).split_whitespace() {
            if !raw.contains("$(") {
                out.push((raw.to_string(), subdir.clone()));
            }
        }
    }
    out
}

/// Every header the project installs, from any `*_HEADERS` primary.
fn public_headers(db: &VariableDatabase) -> Vec<String> {
    installed_headers(db)
        .into_iter()
        .map(|(raw, _)| raw)
        .collect()
}

/// The SONAME a libtool `.la` declares, from its own `dlname=` field.
///
/// A `.la` is a text control file, not a library, and it is the only place
/// the real name is written down: `liblzma.la` builds `liblzma.so.5`, and
/// nothing about the automake target name says so. Read rather than derived
/// because libtool owns the versioning scheme — `current`/`age`/`revision`
/// do not map to the filename by any rule worth reimplementing.
///
/// `None` for a static-only library, whose `dlname` is empty, and for
/// anything that is not a `.la`.
fn libtool_dlname(la_path: &Path) -> Option<String> {
    if la_path.extension().and_then(|e| e.to_str()) != Some("la") {
        return None;
    }
    let text = std::fs::read_to_string(la_path).ok()?;
    let dlname = text
        .lines()
        .find_map(|l| l.strip_prefix("dlname="))?
        .trim_matches('\'');
    (!dlname.is_empty()).then(|| dlname.to_string())
}

/// The Bazel target name for an automake target name.
///
/// Sanitises rather than prettifies: automake already gave us the name, so the
/// only job is to make it a legal Bazel label. An earlier version stripped the
/// `lib` prefix and the suffix, so `libgreet.a` read as `greet` — which is
/// nicer until a project names its library after its program. GNU hello does
/// exactly that (`bin_PROGRAMS = hello`, `noinst_LIBRARIES = lib/libhello.a`),
/// and both collapsed to `hello`: two rules with one name, and a module that
/// cannot load. That is the COMMON shape, not an edge case, and prettiness is
/// not worth a name clash.
///
/// The directory is kept for the same reason — `lib/libhello.a` and
/// `src/libhello.a` are different targets in a project with SUBDIRS.
fn target_label(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
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

/// Headers the build generated into its own tree that no config header or
/// replacement header reproduces, as `(build-relative path, the stream lines
/// that mention it)`.
///
/// Found through the compile lines' `-I` directories: one that resolves
/// into the build tree and not into the source tree exists only because
/// the build put something there. Every header under it is generated; the
/// ones a `config_header` rule already produces (by the same build-relative
/// path) are accounted for and the rest are not. The recipe lines are
/// evidence, not parsed: `parse_commands` keeps only compilers and
/// archivers, so the `sed` that made the file is invisible to the graph but
/// still in the stream, and it is the one thing an agent needs.
fn generated_headers_in_build_tree(
    commands: &[BuildCommand],
    build_root: &Path,
    source_dir: &Path,
    config_headers: &[crate::model::ConfigHeader],
    stream: &str,
) -> Vec<(String, Vec<String>)> {
    let build_root = normalize_lexically(build_root);
    let source_dir = normalize_lexically(source_dir);
    let mut dirs: Vec<PathBuf> = commands
        .iter()
        .flat_map(|cmd| {
            includes_of(&cmd.args)
                .into_iter()
                .map(move |d| cmd.dir.join(d))
        })
        .map(|d| normalize_lexically(&d))
        .filter(|d| d.starts_with(&build_root) && !d.starts_with(&source_dir))
        .collect();
    dirs.sort();
    dirs.dedup();

    let reproduced: HashSet<String> = config_headers.iter().map(|h| h.output_path()).collect();
    let mut found: Vec<String> = Vec::new();
    for dir in &dirs {
        let mut stack = vec![dir.clone()];
        while let Some(current) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if is_header_file(&path)
                    && let Ok(rel) = path.strip_prefix(&build_root)
                {
                    let rel = rel.to_string_lossy().into_owned();
                    if !reproduced.contains(&rel) && !found.contains(&rel) {
                        found.push(rel);
                    }
                }
            }
        }
    }
    found.sort();
    found
        .into_iter()
        .map(|rel| {
            let name = basename(&rel);
            let recipe: Vec<String> = stream
                .lines()
                .filter(|l| l.contains(&name) && !l.contains(" -c ") && !l.contains(" -o "))
                .map(|l| l.trim().to_string())
                .collect();
            (rel, recipe)
        })
        .collect()
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

/// The source file a compile command names, seeing through automake's VPATH
/// guard.
///
/// A subdir-objects compile does not name the source plainly. It emits
///
///   `test -f 'lzmainfo.c' || echo '<srcdir>/src/lzmainfo/'`lzmainfo.c
///
/// so the compiler finds the file whether the build is in-tree or out-of-tree.
/// Tokenised, that is SEVERAL arguments, two of which look like sources — the
/// bare `lzmainfo.c` inside the test is a decoy. The LAST argument is the real
/// one; the guard is never evaluated, because the echoed directory is already
/// the path wanted and running `test -f` would make the answer depend on the
/// filesystem the frontend happens to run against. Full account, including how
/// it surfaces, in
/// docs/lore/automake-wraps-sources-in-a-vpath-guard.md.
fn compiled_source(args: &[String]) -> Option<String> {
    // The LAST argument, always: everything before it is either flags or the
    // guard's own words. Tokenising the guard leaves the echoed directory
    // glued to the filename by the closing backtick —
    // `/src/src/lzmainfo/`lzmainfo.c` — so splitting on that backtick and
    // rejoining recovers the real path, while a plain source has no backtick
    // and passes through untouched.
    let last = args.last()?;
    let full = match last.split_once('`') {
        Some((dir, name)) => format!("{dir}{name}"),
        None => last.clone(),
    };
    is_translation_unit(&full).then_some(full)
}

/// The object files a link or archive command consumes.
///
/// Order is whatever the command used; the caller sorts, because generated
/// `srcs` are sorted for determinism regardless of where they came from.
fn link_inputs(cmd: Option<&BuildCommand>) -> Vec<String> {
    let Some(cmd) = cmd else {
        return Vec::new();
    };
    cmd.args
        .iter()
        .filter(|a| a.ends_with(".o") || a.ends_with(".lo"))
        .cloned()
        .collect()
}

/// Expresses `path` relative to `base`, walking up with `..` where it must.
///
/// `Path::strip_prefix` cannot do this — it fails outright when `path` is not
/// under `base`, and a conditional source legitimately is not: liblzma is
/// declared in `src/liblzma` and compiles `../common/tuklib_physmem.c`.
fn pathdiff(path: &Path, base: &Path) -> String {
    let mut p = path.components().peekable();
    let mut b = base.components().peekable();
    while p.peek().is_some() && p.peek() == b.peek() {
        p.next();
        b.next();
    }
    let ups = b.count();
    let rest: PathBuf = p.collect();
    let mut out = PathBuf::new();
    for _ in 0..ups {
        out.push("..");
    }
    out.push(rest);
    out.to_string_lossy().into_owned()
}

fn is_library(path: &str) -> bool {
    path.ends_with(".a") || path.ends_with(".la") || path.contains(".so")
}

/// Whether `-l<name>` names the C toolchain's own runtime rather than a
/// library some project builds. These are what every hermetic toolchain
/// supplies itself, so linking them is not a dependency on the host — and
/// escalating `-lm` on every project that uses `sqrt` would bury the real
/// unconverted dependencies. Anything not listed is treated as one.
fn is_toolchain_library(name: &str) -> bool {
    matches!(
        name,
        "c" | "m"
            | "pthread"
            | "dl"
            | "rt"
            | "util"
            | "resolv"
            | "gcc"
            | "gcc_s"
            | "stdc++"
            | "atomic"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database with no directory announcements: one scope, the root's.
    fn single_scope(database: &str) -> HashMap<String, String> {
        let db = VariableDatabase::parse(database, Path::new("/b"));
        assert_eq!(
            db.scopes().count(),
            1,
            "single_scope is for a database without directory announcements"
        );
        db.root().clone()
    }

    fn parsed(database: &str) -> VariableDatabase {
        VariableDatabase::parse(database, Path::new("/b"))
    }
    use crate::codegen;
    use crate::needs_attention::NeedsAttention;

    /// A real capture from fixture 001, trimmed of the autoconf
    /// -DPACKAGE_* block for readability. Frozen evidence: this is what
    /// automake actually emits, so the parser can be wrong against it.
    /// A real capture from fixture 001: the stdout of `make -j16
    /// --output-sync=recurse`, verbatim.
    ///
    /// The BUILD's own output, which is what the frontend now reads —
    /// not a `make -n` dry run. Frozen evidence, so the shape is the build
    /// system's and not ours: the `-DPACKAGE_*` block, the single `-I.`,
    /// the `depbase=` shell bookkeeping and libtool's `--tag=CC` are all
    /// what automake really emits. An earlier hand-written version carried
    /// an `-Isrc` automake never produces, and an assertion about
    /// per-target include attribution was passing against that invented
    /// flag.
    ///
    /// This is the IN-TREE shape, and that is a deliberate choice rather than
    /// convenience. Captured out-of-tree, the same fixture emits `-I.` AND an
    /// absolute `-I<srcdir>`, with the compiled source absolute too — which
    /// changes what `needs_root_include` and `includes` come out as. Both
    /// shapes are real; pinning one here and leaving the other to fixture
    /// conversion is what keeps this capture comparable across re-captures.
    const STREAM: &str = r#"
depbase=`echo src/greet.o | sed 's|[^/]*$|.deps/&|;s|\.o$||'`;\
gcc -DPACKAGE_NAME=\"greeter\" -DPACKAGE_TARNAME=\"greeter\" -DPACKAGE_VERSION=\"1.0\" -DPACKAGE_STRING=\"greeter\ 1.0\" -DPACKAGE_BUGREPORT=\"\" -DPACKAGE_URL=\"\" -DPACKAGE=\"greeter\" -DVERSION=\"1.0\" -DHAVE_STDIO_H=1 -DHAVE_STDLIB_H=1 -DHAVE_STRING_H=1 -DHAVE_INTTYPES_H=1 -DHAVE_STDINT_H=1 -DHAVE_STRINGS_H=1 -DHAVE_SYS_STAT_H=1 -DHAVE_SYS_TYPES_H=1 -DHAVE_UNISTD_H=1 -DSTDC_HEADERS=1 -DHAVE_DLFCN_H=1 -DLT_OBJDIR=\".libs/\" -I.     -g -O2 -MT src/greet.o -MD -MP -MF $depbase.Tpo -c -o src/greet.o src/greet.c &&\
mv -f $depbase.Tpo $depbase.Po
depbase=`echo src/main.o | sed 's|[^/]*$|.deps/&|;s|\.o$||'`;\
gcc -DPACKAGE_NAME=\"greeter\" -DPACKAGE_TARNAME=\"greeter\" -DPACKAGE_VERSION=\"1.0\" -DPACKAGE_STRING=\"greeter\ 1.0\" -DPACKAGE_BUGREPORT=\"\" -DPACKAGE_URL=\"\" -DPACKAGE=\"greeter\" -DVERSION=\"1.0\" -DHAVE_STDIO_H=1 -DHAVE_STDLIB_H=1 -DHAVE_STRING_H=1 -DHAVE_INTTYPES_H=1 -DHAVE_STDINT_H=1 -DHAVE_STRINGS_H=1 -DHAVE_SYS_STAT_H=1 -DHAVE_SYS_TYPES_H=1 -DHAVE_UNISTD_H=1 -DSTDC_HEADERS=1 -DHAVE_DLFCN_H=1 -DLT_OBJDIR=\".libs/\" -I.     -g -O2 -MT src/main.o -MD -MP -MF $depbase.Tpo -c -o src/main.o src/main.c &&\
mv -f $depbase.Tpo $depbase.Po
rm -f libgreet.a
ar cru libgreet.a src/greet.o 
ranlib libgreet.a
depbase=`echo src/shout.lo | sed 's|[^/]*$|.deps/&|;s|\.lo$||'`;\
/bin/bash ./libtool  --tag=CC   --mode=compile gcc -DPACKAGE_NAME=\"greeter\" -DPACKAGE_TARNAME=\"greeter\" -DPACKAGE_VERSION=\"1.0\" -DPACKAGE_STRING=\"greeter\ 1.0\" -DPACKAGE_BUGREPORT=\"\" -DPACKAGE_URL=\"\" -DPACKAGE=\"greeter\" -DVERSION=\"1.0\" -DHAVE_STDIO_H=1 -DHAVE_STDLIB_H=1 -DHAVE_STRING_H=1 -DHAVE_INTTYPES_H=1 -DHAVE_STDINT_H=1 -DHAVE_STRINGS_H=1 -DHAVE_SYS_STAT_H=1 -DHAVE_SYS_TYPES_H=1 -DHAVE_UNISTD_H=1 -DSTDC_HEADERS=1 -DHAVE_DLFCN_H=1 -DLT_OBJDIR=\".libs/\" -I.     -g -O2 -MT src/shout.lo -MD -MP -MF $depbase.Tpo -c -o src/shout.lo src/shout.c &&\
mv -f $depbase.Tpo $depbase.Plo
libtool: compile:  gcc -DPACKAGE_NAME=\"greeter\" -DPACKAGE_TARNAME=\"greeter\" -DPACKAGE_VERSION=\"1.0\" "-DPACKAGE_STRING=\"greeter 1.0\"" -DPACKAGE_BUGREPORT=\"\" -DPACKAGE_URL=\"\" -DPACKAGE=\"greeter\" -DVERSION=\"1.0\" -DHAVE_STDIO_H=1 -DHAVE_STDLIB_H=1 -DHAVE_STRING_H=1 -DHAVE_INTTYPES_H=1 -DHAVE_STDINT_H=1 -DHAVE_STRINGS_H=1 -DHAVE_SYS_STAT_H=1 -DHAVE_SYS_TYPES_H=1 -DHAVE_UNISTD_H=1 -DSTDC_HEADERS=1 -DHAVE_DLFCN_H=1 -DLT_OBJDIR=\".libs/\" -I. -g -O2 -MT src/shout.lo -MD -MP -MF src/.deps/shout.Tpo -c src/shout.c  -fPIC -DPIC -o src/.libs/shout.o
libtool: compile:  gcc -DPACKAGE_NAME=\"greeter\" -DPACKAGE_TARNAME=\"greeter\" -DPACKAGE_VERSION=\"1.0\" "-DPACKAGE_STRING=\"greeter 1.0\"" -DPACKAGE_BUGREPORT=\"\" -DPACKAGE_URL=\"\" -DPACKAGE=\"greeter\" -DVERSION=\"1.0\" -DHAVE_STDIO_H=1 -DHAVE_STDLIB_H=1 -DHAVE_STRING_H=1 -DHAVE_INTTYPES_H=1 -DHAVE_STDINT_H=1 -DHAVE_STRINGS_H=1 -DHAVE_SYS_STAT_H=1 -DHAVE_SYS_TYPES_H=1 -DHAVE_UNISTD_H=1 -DSTDC_HEADERS=1 -DHAVE_DLFCN_H=1 -DLT_OBJDIR=\".libs/\" -I. -g -O2 -MT src/shout.lo -MD -MP -MF src/.deps/shout.Tpo -c src/shout.c -o src/shout.o >/dev/null 2>&1
/bin/bash ./libtool  --tag=CC   --mode=link gcc  -g -O2   -o libshout.la -rpath /usr/local/lib src/shout.lo  
libtool: link: gcc -shared  -fPIC -DPIC  src/.libs/shout.o    -g -O2   -Wl,-soname -Wl,libshout.so.0 -o .libs/libshout.so.0.0.0
libtool: link: (cd ".libs" && rm -f "libshout.so.0" && ln -s "libshout.so.0.0.0" "libshout.so.0")
libtool: link: (cd ".libs" && rm -f "libshout.so" && ln -s "libshout.so.0.0.0" "libshout.so")
libtool: link: ar cr .libs/libshout.a  src/shout.o
libtool: link: ranlib .libs/libshout.a
libtool: link: ( cd ".libs" && rm -f "libshout.la" && ln -s "../libshout.la" "libshout.la" )
/bin/bash ./libtool  --tag=CC   --mode=link gcc  -g -O2   -o greeter src/main.o libgreet.a libshout.la 
libtool: link: gcc -g -O2 -o .libs/greeter src/main.o  libgreet.a ./.libs/libshout.so
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

    /// Both frozen captures are from a build rooted here, so paths in them
    /// resolve against it. An in-tree build, where the source and build roots
    /// coincide, is the simplest case that still exercises the rebasing.
    const ROOT: &str = "/src";

    fn graph_from_captures() -> BuildGraph {
        graph_only(to_graph(
            &parse_commands(STREAM, Path::new(ROOT)),
            &VariableDatabase::parse(DATABASE, Path::new(ROOT)),
            "greeter",
            Path::new(ROOT),
            Path::new(ROOT),
        ))
    }

    /// Drops the escalations `to_graph` returns alongside the graph.
    ///
    /// Only for tests asserting on graph shape. A test about what the frontend
    /// ESCALATES must read the second element instead of reaching for this.
    fn graph_only(
        result: (
            BuildGraph,
            Vec<crate::needs_attention::NeedsAttention>,
            PathBuf,
        ),
    ) -> BuildGraph {
        result.0
    }

    // bzl-yjn.7: recursive make runs each subdirectory's commands from that
    // subdirectory, so a path in the stream is relative to IT, not to the
    // build root. xz compiles ../common/tuklib_*.c from src/xz; fixture 002
    // reproduces it with ../common/util.c from app/. Emitted verbatim, that
    // is a label reaching above its own module, which codegen refuses.
    #[test]
    fn a_commands_paths_resolve_against_the_directory_it_ran_in() {
        const STREAM: &str = "\
make[1]: Entering directory '/src/app'\n\
gcc -I. -I../common -c -o main.o main.c\n\
gcc -c -o util.o ../common/util.c\n\
gcc -o tool main.o util.o\n\
make[1]: Leaving directory '/src/app'\n\
";
        let commands = parse_commands(STREAM, Path::new("/src"));
        assert!(
            commands.iter().all(|c| c.dir == Path::new("/src/app")),
            "every command between Entering and Leaving runs in that \
             directory: {commands:#?}"
        );

        let graph = graph_only(to_graph(
            &commands,
            &VariableDatabase::parse(
                "bin_PROGRAMS = tool\ntool_SOURCES = main.c ../common/util.c\n",
                Path::new("/src"),
            ),
            "sibling",
            Path::new("/src"),
            Path::new("/src"),
        ));
        let tool = &graph.targets[0];
        assert_eq!(
            tool.sources,
            vec!["app/main.c", "common/util.c"],
            "the sibling source resolves to its module-relative path, not to \
             the literal ../common/util.c the command reported:\n{tool:#?}"
        );
        assert_eq!(
            tool.includes,
            vec!["app", "common"],
            "-I. and -I../common both resolve against app/, giving app and \
             common — neither is the module root here, so nothing is \
             dropped:\n{tool:#?}"
        );
        assert!(
            !tool.needs_root_include,
            "no include dir resolves TO the module root in this stream: \
             {tool:#?}"
        );
        assert_eq!(
            tool.artifacts,
            vec!["app/tool"],
            "and the artifact is where the build actually put it — `-o tool` \
             from inside app/ is app/tool, not a bare tool that no copy can \
             find"
        );
    }

    // Both directions for the outside-module escalation, because a gate only
    // ever seen firing is indistinguishable from one wired to nothing. The
    // negative is the sibling case above: `../common/util.c` looks like it
    // escapes and does not, so a check that merely spots `..` would fire on it.
    #[test]
    fn a_source_that_escapes_the_module_is_escalated_not_dropped() {
        const STREAM: &str = "\
make[1]: Entering directory '/src/app'\n\
gcc -c -o main.o main.c\n\
gcc -c -o evil.o ../../outside/evil.c\n\
gcc -o tool main.o evil.o\n\
make[1]: Leaving directory '/src/app'\n\
";
        let (graph, escalations, _) = to_graph(
            &parse_commands(STREAM, Path::new("/src")),
            &VariableDatabase::parse(
                "bin_PROGRAMS = tool\ntool_SOURCES = main.c ../../outside/evil.c\n",
                Path::new("/src"),
            ),
            "escaper",
            Path::new("/src"),
            Path::new("/src"),
        );

        let tool = &graph.targets[0];
        assert_eq!(
            tool.sources,
            vec!["app/main.c"],
            "the escaping source cannot be a label and is left out:\n{tool:#?}"
        );
        assert_eq!(
            escalations.len(),
            1,
            "leaving it out silently makes the target fail to link with an \
             undefined symbol several steps from the cause; got: {escalations:#?}"
        );
        assert!(
            escalations[0].gap.contains("outside/evil.c") && escalations[0].title.contains("tool"),
            "the escalation must name the file AND the target now missing it, \
             because the resolution is per target; got:\n{:#?}",
            escalations[0]
        );
        // The HOW, not just the what. A raw absolute path satisfies the
        // `contains` above, which is exactly why this frontend shipped one
        // for months without any tier noticing (bzl-ti2): the assertion was
        // written to pass either way.
        assert!(
            escalations[0].gap.contains("<outside the deliverable>/")
                || escalations[0].gap.contains(".../"),
            "the path must be ANCHORED on something the reader can locate — \
             an absolute path here is a Bazel sandbox directory with a \
             run-specific number, meaningless to an agent in an unpacked \
             workspace; got:\n{}",
            escalations[0].gap
        );
        assert!(
            !escalations[0].gap.contains("/src/outside/evil.c"),
            "and specifically not the conversion's own absolute path:\n{}",
            escalations[0].gap
        );
    }

    // Three cases for the module root, and they have to be read together:
    // widening is only correct because the deliverable root caps it, and the
    // cap is only observable because widening otherwise happens.
    //
    // The escaping case here is the SAME stream as
    // `a_source_that_escapes_the_module_is_escalated_not_dropped`; what
    // differs is a deliverable root wide enough to contain the file. That the
    // two disagree is the whole point — the file's location on disk does not
    // decide this, the declared deliverable does.
    const ESCAPING_STREAM: &str = "\
make[1]: Entering directory '/deliv/proj/app'\n\
gcc -c -o main.o main.c\n\
gcc -c -o helper.o ../../shared/helper.c\n\
gcc -o tool main.o helper.o\n\
make[1]: Leaving directory '/deliv/proj/app'\n\
";

    fn escaping_graph(
        deliverable_root: &str,
    ) -> (
        BuildGraph,
        Vec<crate::needs_attention::NeedsAttention>,
        PathBuf,
    ) {
        to_graph(
            &parse_commands(ESCAPING_STREAM, Path::new("/deliv/proj")),
            &VariableDatabase::parse(
                "bin_PROGRAMS = tool\ntool_SOURCES = main.c ../../shared/helper.c\n",
                Path::new("/deliv/proj"),
            ),
            "widening",
            Path::new("/deliv/proj"),
            Path::new(deliverable_root),
        )
    }

    #[test]
    fn the_module_root_widens_to_cover_a_shipped_sibling_source() {
        let (graph, escalations, module_root) = escaping_graph("/deliv");

        assert_eq!(
            module_root,
            PathBuf::from("/deliv"),
            "a Bazel label cannot reach above its own module, so a root at \
             /deliv/proj could never name ../../shared/helper.c"
        );
        assert_eq!(
            graph.targets[0].sources,
            vec!["proj/app/main.c".to_string(), "shared/helper.c".to_string()],
            "every path is rewritten against the WIDENED root, including the \
             ones that already fitted the narrow one"
        );
        assert!(
            escalations.is_empty(),
            "a file that ships with the project is not a gap — nothing is \
             missing once the module covers it: {:#?}",
            escalations.iter().map(|e| &e.title).collect::<Vec<_>>()
        );
    }

    // The survey has to look at everything the build REFERENCES, not just
    // what it compiles: CMake surveys sources, public headers and include
    // dirs together, and an Autotools project whose only outside reference
    // was an installed header used to not widen where its CMake equivalent
    // did. Two structurally identical projects, two different modules.
    #[test]
    fn an_installed_header_outside_the_project_widens_the_root() {
        const STREAM: &str = "\
make[1]: Entering directory '/deliv/proj'\n\
gcc -c -o main.o main.c\n\
gcc -o tool main.o\n\
make[1]: Leaving directory '/deliv/proj'\n\
";
        let db = "bin_PROGRAMS = tool\n\
             tool_SOURCES = main.c\n\
             include_HEADERS = ../shared/api.h\n";
        let (_, _, module_root) = to_graph(
            &parse_commands(STREAM, Path::new("/deliv/proj")),
            &VariableDatabase::parse(db, Path::new("/deliv/proj")),
            "hdr",
            Path::new("/deliv/proj"),
            Path::new("/deliv"),
        );

        assert_eq!(
            module_root,
            PathBuf::from("/deliv"),
            "an installed header above the project must widen the root, or \
             the label naming it reaches outside the module. Deliberately \
             NOT also in _SOURCES: that would widen via the sources survey \
             and the header path would never be exercised"
        );
    }

    #[test]
    fn the_module_root_does_not_widen_past_the_deliverable_root() {
        let (graph, escalations, module_root) = escaping_graph("/deliv/proj");

        assert_eq!(
            module_root,
            PathBuf::from("/deliv/proj"),
            "the cap's whole job: a file outside the deliverable must not drag \
             the module root out to meet it"
        );
        assert_eq!(
            graph.targets[0].sources,
            vec!["app/main.c".to_string()],
            "the escaping source is left out rather than reached for"
        );
        assert_eq!(
            escalations.len(),
            1,
            "and it is escalated, because a file outside the deliverable \
             cannot be reproduced from what the project ships: {escalations:#?}"
        );
    }

    #[test]
    fn the_module_root_stays_at_the_project_when_nothing_reaches_outside() {
        const STREAM: &str = "\
make[1]: Entering directory '/deliv/proj'\n\
gcc -c -o main.o src/main.c\n\
gcc -o tool main.o\n\
make[1]: Leaving directory '/deliv/proj'\n\
";
        let (graph, _, module_root) = to_graph(
            &parse_commands(STREAM, Path::new("/deliv/proj")),
            &VariableDatabase::parse(
                "bin_PROGRAMS = tool\ntool_SOURCES = src/main.c\n",
                Path::new("/deliv/proj"),
            ),
            "narrow",
            Path::new("/deliv/proj"),
            // Deliberately WIDE: widening is driven by what the build
            // references, not by how much room it was given. A root that
            // widens to the cap regardless would ship the sibling directory
            // into every module that merely could have used it.
            Path::new("/deliv"),
        );

        assert_eq!(
            module_root,
            PathBuf::from("/deliv/proj"),
            "nothing reaches outside the project, so the root stays there even \
             though the deliverable root would have allowed /deliv"
        );
        assert_eq!(graph.targets[0].sources, vec!["src/main.c".to_string()]);
    }

    #[test]
    fn a_sibling_source_inside_the_module_is_not_escalated() {
        const STREAM: &str = "\
make[1]: Entering directory '/src/app'\n\
gcc -c -o main.o main.c\n\
gcc -c -o util.o ../common/util.c\n\
gcc -o tool main.o util.o\n\
make[1]: Leaving directory '/src/app'\n\
";
        let (_, escalations, _) = to_graph(
            &parse_commands(STREAM, Path::new("/src")),
            &VariableDatabase::parse(
                "bin_PROGRAMS = tool\ntool_SOURCES = main.c ../common/util.c\n",
                Path::new("/src"),
            ),
            "sibling",
            Path::new("/src"),
            Path::new("/src"),
        );
        assert!(
            escalations.is_empty(),
            "`../common/util.c` leaves app/ but stays inside the module, so it \
             resolves to common/util.c and nothing is missing; got: \
             {escalations:#?}"
        );
    }

    // automake's subdir-objects compiles do not name the source plainly; they
    // wrap it in a VPATH guard that tokenises into several arguments, two of
    // which look like sources. Taking the last source-LOOKING token picks the
    // bare filename inside the `test -f`, so the compile's flags attach to no
    // target and the rule renders with empty `includes` — several steps from
    // the cause. xz is full of these.
    #[test]
    fn compiled_source_sees_through_automakes_vpath_guard() {
        // As tokenize() actually yields it: the quotes are consumed, so the
        // echoed directory and the filename arrive as ONE token joined by the
        // closing backtick.
        let args: Vec<String> = [
            "-c",
            "-o",
            "lzmainfo-lzmainfo.o",
            "`test",
            "-f",
            "lzmainfo.c",
            "||",
            "echo",
            "/src/src/lzmainfo/`lzmainfo.c",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            compiled_source(&args).as_deref(),
            Some("/src/src/lzmainfo/lzmainfo.c"),
            "the echoed directory names where the file really is; the bare \
             lzmainfo.c inside the test is a decoy"
        );

        // An unguarded compile still works.
        let plain: Vec<String> = "-c -o main.o main.c"
            .split_whitespace()
            .map(str::to_string)
            .collect();
        assert_eq!(compiled_source(&plain).as_deref(), Some("main.c"));
    }

    // automake compiles conditional sources by appending `$(am__append_N)` to
    // `_SOURCES`, one per `if`, and make's database reports those references
    // VERBATIM. Taking the literal text drops every conditional source: xz's
    // liblzma declares 45 of them around an 11-file base, so the library built
    // from 11 sources and the link failed on ~70 undefined symbols, nowhere
    // near the cause.
    #[test]
    fn conditional_sources_are_recovered_from_the_command_stream() {
        const STREAM: &str = "\
make[1]: Entering directory '/src/lib'\n\
gcc -c -o base.o base.c\n\
gcc -c -o extra.o extra.c\n\
gcc -c -o sibling.o ../common/shared.c\n\
ar cru libfoo.a base.o extra.o sibling.o\n\
make[1]: Leaving directory '/src/lib'\n\
";
        // Exactly the shape make reports: the base list, plus an unexpanded
        // reference standing for everything the conditionals added.
        let db = "noinst_LIBRARIES = libfoo.a\nlibfoo_a_SOURCES = base.c $(am__append_1)\n";
        let (graph, _, _) = to_graph(
            &parse_commands(STREAM, Path::new("/src")),
            &VariableDatabase::parse(db, Path::new("/src")),
            "conditional",
            Path::new("/src"),
            Path::new("/src"),
        );

        assert_eq!(
            graph.targets[0].sources,
            vec![
                "common/shared.c".to_string(),
                "lib/base.c".to_string(),
                "lib/extra.c".to_string()
            ],
            "an unexpanded declaration cannot answer, so the command stream \
             does — including the sibling source, which is why the fallback \
             cannot simply strip_prefix the declaring directory. Sorted, like \
             every other source list"
        );
    }

    // The other direction: a declaration that IS fully expanded stays
    // authoritative. It carries headers and the automake-declared order, both
    // of which the object list loses.
    #[test]
    fn a_fully_expanded_declaration_is_not_second_guessed() {
        const STREAM: &str = "\
make[1]: Entering directory '/src/lib'\n\
gcc -c -o base.o base.c\n\
ar cru libfoo.a base.o\n\
make[1]: Leaving directory '/src/lib'\n\
";
        let (graph, _, _) = to_graph(
            &parse_commands(STREAM, Path::new("/src")),
            &VariableDatabase::parse(
                "noinst_LIBRARIES = libfoo.a\nlibfoo_a_SOURCES = base.c base.h\n",
                Path::new("/src"),
            ),
            "expanded",
            Path::new("/src"),
            Path::new("/src"),
        );

        assert_eq!(
            graph.targets[0].sources,
            vec!["lib/base.c".to_string()],
            "no `$(` in the declaration means no fallback; the header is split \
             out as before rather than being invisible to the object list"
        );
    }

    // The frontend carried its own extension list, which drifted from the
    // shared one: no lowercasing, and missing `c++`. Both gaps failed in the
    // SAME silent direction, because the partition was negated — anything not
    // recognised as a source became a header, was filtered against
    // public_headers, and if not installed was dropped from the graph with no
    // escalation. The symptom is an undefined symbol at link time.
    //
    // No fixture ships a C++ project, so this is the only tier that can catch
    // it (bzl-dc9).
    // public_headers took the raw declaration text while sources next to it
    // were rebased, so a header declared in a SUBDIRECTORY Makefile.am got a
    // label pointing at nothing — and one declared as `../common/x.h` produced
    // a `..` path that panics codegen's module-relative assert.
    //
    // Invisible to the frontend's other tests because their captures declare
    // everything at the module root, where the un-rebased string happens to
    // equal the rebased one.
    #[test]
    fn a_public_header_declared_in_a_subdirectory_is_rebased_like_its_sources() {
        const STREAM: &str = "\
make[1]: Entering directory '/src/lib'\n\
gcc -c -o greet.o greet.c\n\
ar cru libgreet.a greet.o\n\
make[1]: Leaving directory '/src/lib'\n\
";
        let db = "noinst_LIBRARIES = libgreet.a\n\
             libgreet_a_SOURCES = greet.c greet.h\n\
             include_HEADERS = greet.h\n";
        let (graph, _, _) = to_graph(
            &parse_commands(STREAM, Path::new("/src")),
            &VariableDatabase::parse(db, Path::new("/src")),
            "sub",
            Path::new("/src"),
            Path::new("/src"),
        );

        let t = &graph.targets[0];
        assert_eq!(
            t.sources,
            vec!["lib/greet.c".to_string()],
            "the source is rebased against the declaring directory"
        );
        assert_eq!(
            t.public_headers,
            vec!["lib/greet.h".to_string()],
            "and so is the header — the raw `greet.h` names no file at the \
             module root, so codegen would emit a label pointing at \
             nothing:\n{t:#?}"
        );
        for path in t.sources.iter().chain(&t.public_headers) {
            assert!(
                crate::model::is_module_relative(path),
                "every path-valued field must satisfy the contract \
                 render_path_list asserts on: {path:?}"
            );
        }
    }

    #[test]
    fn an_uppercase_or_cxx_source_is_compiled_not_silently_dropped() {
        const STREAM: &str = "\
make[1]: Entering directory '/src'\n\
gcc -c -o a.o a.C\n\
gcc -c -o b.o b.c++\n\
gcc -c -o c.o c.CPP\n\
ar cru libmix.a a.o b.o c.o\n\
make[1]: Leaving directory '/src'\n\
";
        let db = "noinst_LIBRARIES = libmix.a\nlibmix_a_SOURCES = a.C b.c++ c.CPP README\n";
        let (graph, _, _) = to_graph(
            &parse_commands(STREAM, Path::new("/src")),
            &VariableDatabase::parse(db, Path::new("/src")),
            "mixed",
            Path::new("/src"),
            Path::new("/src"),
        );

        assert_eq!(
            graph.targets[0].sources,
            vec!["a.C".to_string(), "b.c++".to_string(), "c.CPP".to_string()],
            "an extension the predicate does not know is dropped ENTIRELY — \
             not escalated — so it must know all of them:\n{:#?}",
            graph.targets[0]
        );
        assert!(
            !graph.targets[0].sources.contains(&"README".to_string())
                && !graph.targets[0]
                    .public_headers
                    .contains(&"README".to_string()),
            "automake permits a non-source in _SOURCES; it is neither compiled \
             nor a header, and the negated partition used to make it one"
        );
    }

    // GNU hello's database reports LDADD UNEXPANDED:
    //   hello_LDADD = $(LIBINTL) $(top_builddir)/lib/lib$(PACKAGE).a
    // Matching those tokens against declared names finds nothing, so taking
    // edges from the declaration dropped every dependency in silence. The
    // link line carries the resolved path, which is why edges come from the
    // command stream instead.
    #[test]
    fn dependencies_come_from_the_link_line_not_the_unexpanded_declaration() {
        const DB: &str = "\
bin_PROGRAMS = hello\n\
noinst_LIBRARIES = lib/libhello.a\n\
hello_SOURCES = src/hello.c\n\
hello_LDADD = $(LIBINTL) $(top_builddir)/lib/lib$(PACKAGE).a\n\
";
        const STREAM_WITH_RESOLVED_LINK: &str = "\
gcc -I. -c -o src/hello.o src/hello.c\n\
ar cru lib/libhello.a lib/basename.o\n\
gcc -g -O2 -o hello src/hello.o ./lib/libhello.a\n\
";
        let graph = graph_only(to_graph(
            &parse_commands(STREAM_WITH_RESOLVED_LINK, Path::new(ROOT)),
            &VariableDatabase::parse(DB, Path::new(ROOT)),
            "hello",
            Path::new(ROOT),
            Path::new(ROOT),
        ));

        let hello = graph
            .targets
            .iter()
            .find(|t| t.name == "hello")
            .expect("the program must render");
        assert_eq!(
            hello.dependencies,
            vec!["lib_libhello.a"],
            "the edge is in the link line as ./lib/libhello.a; the declaration \
             only has $(top_builddir)/lib/lib$(PACKAGE).a, which resolves to \
             nothing here:\n{hello:#?}"
        );
    }

    // GNU hello: bin_PROGRAMS = hello and noinst_LIBRARIES = lib/libhello.a.
    // Stripping `lib` and the suffix rendered BOTH as `hello`, giving a module
    // with two rules of one name that cannot load. A project naming its
    // library after its program is the common shape, so this is pinned
    // directly rather than left to the fixture, which happens not to collide.
    #[test]
    fn target_label_does_not_collapse_a_program_and_its_library() {
        assert_ne!(
            target_label("hello"),
            target_label("lib/libhello.a"),
            "a program and its own library must stay distinct targets"
        );
        // Sanitised for Bazel, not shortened: the name automake gave is kept.
        assert_eq!(target_label("lib/libhello.a"), "lib_libhello.a");
        assert_eq!(target_label("hello"), "hello");
        // The directory is part of the identity — two SUBDIRS can each
        // declare a libhello.a.
        assert_ne!(
            target_label("lib/libhello.a"),
            target_label("src/libhello.a")
        );
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

    // `TESTS` splits three ways across real projects, and the majority case
    // is NOT the one the first project onboarded showed. Surveyed six:
    //
    //   libmicrohttpd, gsl, wget   TESTS = $(check_PROGRAMS)  -> binaries
    //   jansson, xz                a shell script             -> script
    //   libgd, wget, gsl           a project-specific var     -> unresolved
    //
    // gsl alone contributes 54 binaries, so generalising from jansson's
    // scripts would have missed the common shape entirely.
    #[test]
    fn tests_naming_check_programs_are_classified_as_binaries() {
        // Exactly what `make -p` reports for libmicrohttpd: TESTS carries an
        // unexpanded reference while check_PROGRAMS beside it is expanded.
        let db = "check_PROGRAMS = test_str_compare$(EXEEXT) test_str_token$(EXEEXT)\n\
             TESTS = $(check_PROGRAMS)\n";
        let vars = single_scope(db);
        assert_eq!(
            classify_tests(&vars),
            vec![
                TestEntry::Binary("test_str_compare".to_string()),
                TestEntry::Binary("test_str_token".to_string()),
            ],
            "a one-level variable reference resolves against the same database \
             it sits in — no shell parsing, and $(EXEEXT) is stripped because \
             it is empty on every platform this converts for"
        );
    }

    #[test]
    fn tests_naming_a_script_are_classified_as_scripts() {
        let db = "check_PROGRAMS = test_array$(EXEEXT)\nTESTS = run-suites scripts/format-check\n";
        let vars = single_scope(db);
        assert_eq!(
            classify_tests(&vars),
            vec![
                TestEntry::Script("run-suites".to_string()),
                TestEntry::Script("scripts/format-check".to_string()),
            ],
            "jansson's shape: the binaries exist but TESTS names the scripts \
             that DRIVE them, so mapping TESTS onto check_PROGRAMS would \
             register the wrong thing and miss all 18"
        );
    }

    // One level of expansion was enough for jansson and libmicrohttpd's
    // TESTS, and not for what those expand INTO. libmicrohttpd's
    // check_PROGRAMS carries `$(am__EXEEXT_1)` — automake's per-conditional
    // indirection, the `$(am__append_N)` shape already handled for _SOURCES —
    // so 14 of its tests escalated as literal `$(am__EXEEXT_1)` rather than
    // resolving to test_md5.
    #[test]
    fn a_test_name_expands_exeext_the_same_way_a_target_name_does() {
        // On a Windows-targeting build EXEEXT is ".exe", and `declared_targets`
        // has always expanded it from the database — its own test says
        // "$(EXEEXT) must be expanded from the database, not assumed empty".
        // `classify_tests` used to strip it unconditionally instead, so the
        // target became `prog.exe` while the test entry became `prog`, the
        // membership check missed, and EVERY test escalated as inexpressible.
        // Nothing failed; the project just converted with tests: 0.
        let db = "EXEEXT = .exe\n\
             check_PROGRAMS = check_one$(EXEEXT)\n\
             TESTS = $(check_PROGRAMS)\n";
        let vars = single_scope(db);
        assert_eq!(
            classify_tests(&vars),
            vec![TestEntry::Binary("check_one.exe".to_string())],
            "the test entry has to carry the same name the target does, or the \
             two never match"
        );
        assert_eq!(
            declared_targets_in(Path::new("/b"), &vars)
                .iter()
                .map(|d| d.name.clone())
                .collect::<Vec<_>>(),
            vec!["check_one.exe".to_string()],
            "and that name is whatever declared_targets produces — this test \
             exists to keep the two answers identical"
        );
    }

    // Real `make -p` bytes from libmicrohttpd 1.0.1, trimmed to the three
    // directories that collide on one name. The markers are `# make[N]:`
    // COMMENT lines, which is why the directory has to be read before the
    // comment filter rather than after it.
    const COLLIDING_EXEEXT: &str = "\
# make[3]: Entering directory '/b/libmicrohttpd-1.0.1/src/microhttpd'\n\
am__EXEEXT_1 = test_md5$(EXEEXT)\n\
# make[4]: Entering directory '/b/libmicrohttpd-1.0.1/src/testcurl'\n\
am__EXEEXT_1 = \n\
check_PROGRAMS = $(am__EXEEXT_1)\n\
TESTS = $(check_PROGRAMS)\n\
# make[2]: Entering directory '/b/libmicrohttpd-1.0.1/doc/examples'\n\
am__EXEEXT_1 = basicauthentication$(EXEEXT)\n\
";

    // `am__*` is DIRECTORY-SCOPED, not accumulating — the opposite of what
    // this code assumed. libmicrohttpd defines `am__EXEEXT_1` in six
    // directories with unrelated meanings (`test_md5`, `perf_replies`,
    // `basicauthentication`, ...), and merging them dragged
    // `doc/examples`'s example programs into `src/testcurl`'s TESTS — 102
    // spurious names in one escalation, from a directory declaring no TESTS
    // at all. xz shows the same bug through `am__append_1`, defined nine
    // times.
    //
    // The merge was added because last-wins picked `src/testcurl`'s EMPTY
    // definition and lost the others. Empty was that directory's correct
    // answer; the bug was resolving one directory's TESTS against another's
    // variables.
    #[test]
    fn a_directory_scoped_am_variable_does_not_leak_into_another_directory() {
        assert!(
            classify_tests_per_directory(&VariableDatabase::parse(
                COLLIDING_EXEEXT,
                Path::new("/b/libmicrohttpd-1.0.1")
            ))
            .is_empty(),
            "src/testcurl declares TESTS = $(check_PROGRAMS) = $(am__EXEEXT_1), \
             and ITS am__EXEEXT_1 is empty. test_md5 belongs to src/microhttpd \
             and basicauthentication to doc/examples, which declares no TESTS \
             at all: {:#?}",
            classify_tests_per_directory(&VariableDatabase::parse(
                COLLIDING_EXEEXT,
                Path::new("/b/libmicrohttpd-1.0.1")
            ))
        );
    }

    // The other direction: a directory that DOES declare tests still gets
    // them. Without this, emptying the result would pass the test above.
    #[test]
    fn a_directory_that_declares_tests_still_reports_them() {
        const DB: &str = "\
# make[3]: Entering directory '/b/proj/src/microhttpd'\n\
am__EXEEXT_1 = test_md5$(EXEEXT)\n\
check_PROGRAMS = $(am__EXEEXT_1)\n\
TESTS = $(check_PROGRAMS)\n\
# make[2]: Entering directory '/b/proj/doc/examples'\n\
am__EXEEXT_1 = basicauthentication$(EXEEXT)\n\
";
        assert_eq!(
            classify_tests_per_directory(&VariableDatabase::parse(DB, Path::new("/b/proj"))),
            vec![TestEntry::Binary("test_md5".to_string())],
            "src/microhttpd's own TESTS resolves against its own \
             am__EXEEXT_1, and doc/examples contributes nothing"
        );
    }

    // The brace form is the same variable. Only one of the two implementations
    // handled it, so a project writing ${EXEEXT} got a target literally named
    // `greeter${EXEEXT}`.
    #[test]
    fn the_brace_form_of_exeext_expands_too() {
        let db = "EXEEXT = .exe\ncheck_PROGRAMS = tool${EXEEXT}\nTESTS = $(check_PROGRAMS)\n";
        let vars = single_scope(db);
        assert_eq!(
            classify_tests(&vars),
            vec![TestEntry::Binary("tool.exe".to_string())],
            "${{EXEEXT}} and $(EXEEXT) are one variable written two ways"
        );
    }

    #[test]
    // gnulib generates a replacement for a system header by sed-substituting
    // its own .in.h template. Both the template and every substituted VALUE
    // are in the V=1 stream, which is why this needs no second input — the
    // variable database cannot answer it (it has no STRING_H at all, and
    // sets ICONV_H for a header the build never generates).
    #[test]
    fn a_replacement_header_is_recovered_from_the_generation_recipe() {
        const STREAM: &str = "\
make[1]: Entering directory '/build/gl'\n\
/bin/sed -e 's|@''GUARD_PREFIX''@|GL|g' \\\n\
      -e 's|@''NEXT_STRING_H''@|<string.h>|g' \\\n\
      -e 's/@''GNULIB_STRDUP''@/0/g' \\\n\
      < /src/gl/string.in.h > string.h-t1\n\
mv string.h-t1 string.h\n\
make[1]: Leaving directory '/build/gl'\n\
";
        let found = parse_replacement_headers(STREAM, Path::new("/build"));

        assert_eq!(found.len(), 1, "one header generated: {found:#?}");
        assert_eq!(found[0].output, "string.h");
        assert_eq!(found[0].template, "/src/gl/string.in.h");
        assert_eq!(
            found[0].values,
            vec![
                ("GNULIB_STRDUP".to_string(), "0".to_string()),
                ("GUARD_PREFIX".to_string(), "GL".to_string()),
                ("NEXT_STRING_H".to_string(), "<string.h>".to_string()),
            ],
            "both sed delimiter forms are used in one recipe — `s|...|` for \
             values that may contain a slash, `s/.../` for the rest — so a \
             parser that knows only one silently drops half the substitutions"
        );
    }

    // gnulib writes atomically: the sed redirects to `<name>.h-t` and a
    // later `mv` renames it. Keying on the redirect yields `limits.h-t1`,
    // which is not a header anyone includes — the rename is what names it.
    #[test]
    fn the_header_is_named_by_the_rename_not_the_redirect() {
        const STREAM: &str = "\
make[1]: Entering directory '/build/gl'\n\
/bin/sed -e 's|@''X''@|1|g' < /src/gl/limits.in.h > limits.h-t1\n\
mv limits.h-t1 limits.h\n\
make[1]: Leaving directory '/build/gl'\n\
";
        let found = parse_replacement_headers(STREAM, Path::new("/build"));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].output, "limits.h", "not limits.h-t1");
        assert_eq!(found[0].template, "/src/gl/limits.in.h");
    }

    // libidn2 writes SEVEN of its eleven recipes this way — the template as
    // a positional argument rather than on stdin. Handling only the `<` form
    // found four of eleven and looked like it worked.
    #[test]
    fn a_template_passed_positionally_is_found_too() {
        const STREAM: &str = "\
make[1]: Entering directory '/build/gl'\n\
  -e 's|@''HAVE_ALLOCA_H''@|1|g' \\\n\
  /src/gl/alloca.in.h > alloca.h-t\n\
mv alloca.h-t alloca.h\n\
make[1]: Leaving directory '/build/gl'\n\
";
        let found = parse_replacement_headers(STREAM, Path::new("/build"));
        assert_eq!(
            found.len(),
            1,
            "no `<` redirect, but still a recipe: {found:#?}"
        );
        assert_eq!(found[0].output, "alloca.h");
        assert_eq!(found[0].template, "/src/gl/alloca.in.h");
    }

    // The macros a generated header DECLARES often live in a separate file,
    // spliced in by sed's `r` command rather than substituted. Nothing about
    // this looks like `s|@VAR@|value|`, so a recipe reader that knows only
    // substitutions drops it in silence — and the header still generates,
    // just without the macros, failing later in every consumer.
    #[test]
    fn a_spliced_helper_file_is_recovered_from_the_recipe() {
        const STREAM: &str = "\
make[1]: Entering directory '/build/gl'\n\
/bin/sed -e 's|@''GUARD_PREFIX''@|GL|g' \\\n\
      -e '/definitions of _GL_FUNCDECL_RPL/r ./c++defs.h' \\\n\
      -e '/definition of _GL_ARG_NONNULL/r ./arg-nonnull.h' \\\n\
      < /src/gl/string.in.h > string.h-t1\n\
mv string.h-t1 string.h\n\
make[1]: Leaving directory '/build/gl'\n\
";
        let found = parse_replacement_headers(STREAM, Path::new("/build"));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(
            found[0].splices,
            vec![
                (
                    "definitions of _GL_FUNCDECL_RPL".to_string(),
                    "./c++defs.h".to_string()
                ),
                (
                    "definition of _GL_ARG_NONNULL".to_string(),
                    "./arg-nonnull.h".to_string()
                ),
            ],
            "both `r` splices, in recipe order: {:#?}",
            found[0]
        );
    }

    // Order is the whole content of the field. sed applies each `r`
    // independently at the line its pattern matched, so reordering them puts
    // the text somewhere else in the header — and a set or a map would lose
    // that while still looking populated.
    #[test]
    fn splices_keep_recipe_order_and_are_not_deduplicated() {
        const STREAM: &str = "\
make[1]: Entering directory '/build/gl'\n\
/bin/sed -e '/second marker/r ./shared.h' \\\n\
      -e '/first marker/r ./shared.h' \\\n\
      < /src/gl/x.in.h > x.h-t\n\
mv x.h-t x.h\n\
make[1]: Leaving directory '/build/gl'\n\
";
        let found = parse_replacement_headers(STREAM, Path::new("/build"));
        assert_eq!(
            found[0].splices,
            vec![
                ("second marker".to_string(), "./shared.h".to_string()),
                ("first marker".to_string(), "./shared.h".to_string()),
            ],
            "one file spliced at two markers fires twice, in the recipe's \
             order — not once, and not sorted: {:#?}",
            found[0]
        );
    }

    // Real `autoconf --trace=AC_DEFINE:$1|$2` shapes, from xz and
    // libmicrohttpd. Captured rather than invented: the five mutually
    // exclusive cpucores backends and the duplicate _MHD_EXTERN records are
    // both things a hand-written sample would have got wrong.
    const TRACE: &str = concat!(
        "SPEC___THREAD|__thread\n",
        // xz picks ONE of these in a shell `case`. Each is defined exactly
        // once, so no per-name filter can tell them apart.
        "TUKLIB_CPUCORES_SCHED_GETAFFINITY|1\n",
        "TUKLIB_CPUCORES_CPUSET|1\n",
        "TUKLIB_CPUCORES_SYSCTL|1\n",
        // Three platform spellings of one name.
        "_MHD_EXTERN|__declspec(dllexport)\n",
        "_MHD_EXTERN|extern\n",
        // A probe supplies the value, so `$2` is empty.
        "HAVE_CPUID_H|\n",
    );

    // THE REGRESSION THIS EXISTS FOR. Using the trace alone shipped and was
    // reverted: it emitted all five of xz's TUKLIB_CPUCORES_* variants when
    // configure picked one, and the module then compiled sys/systemcfg.h —
    // an AIX-only header — and xz stopped building.
    //
    // `config.status`'s D[] table is the only source that knows which branch
    // ran, which is why the two are intersected rather than either used
    // alone.
    #[test]
    fn a_branch_this_build_did_not_take_is_not_resolved() {
        let traced = parse_traced_defines(TRACE);
        // What configure actually decided.
        let selected = HashMap::from([
            (
                "TUKLIB_CPUCORES_SCHED_GETAFFINITY".to_string(),
                "1".to_string(),
            ),
            ("SPEC___THREAD".to_string(), "__thread".to_string()),
        ]);
        let resolved = intersect_traced_with_selected(traced, &selected);

        assert!(
            resolved.contains_key("TUKLIB_CPUCORES_SCHED_GETAFFINITY"),
            "the branch configure took resolves: {resolved:#?}"
        );
        assert!(
            !resolved.contains_key("TUKLIB_CPUCORES_CPUSET")
                && !resolved.contains_key("TUKLIB_CPUCORES_SYSCTL"),
            "the branches it did NOT take must not — each is defined once, \
             so only D[] can distinguish them: {resolved:#?}"
        );
        assert_eq!(
            resolved.get("SPEC___THREAD").map(String::as_str),
            Some("__thread"),
            "a project literal still carries its value: {resolved:#?}"
        );
    }

    // Still excluded after intersecting, for reasons D[] cannot supply: a
    // macro defined differently per platform has no single answer, and an
    // empty `$2` means a probe supplies the value.
    #[test]
    fn conditional_and_probe_supplied_defines_stay_out_of_the_trace_map() {
        let traced = parse_traced_defines(TRACE);
        assert!(
            !traced.contains_key("_MHD_EXTERN"),
            "two platform spellings, no single answer: {traced:#?}"
        );
        assert!(
            !traced.contains_key("HAVE_CPUID_H"),
            "an empty value means AC_CHECK_HEADERS supplies it: {traced:#?}"
        );
    }

    // Real `configure --help` output from xz 5.4.5, trimmed to the shapes
    // that matter. Captured rather than invented: the indentation, the
    // wrapped continuation line and autoconf's own `--enable-FEATURE`
    // placeholder are all things a hand-written sample would have missed.
    const XZ_HELP: &str = concat!(
        "Optional Features:\n",
        "  --disable-option-checking  ignore unrecognized --enable/--with options\n",
        "  --disable-FEATURE       do not include FEATURE (same as --enable-FEATURE=no)\n",
        "  --enable-FEATURE[=ARG]  include FEATURE [ARG=yes]\n",
        "  --enable-encoders=LIST  Comma-separated list of encoders to build.\n",
        "  --disable-lzip-decoder  Disable decompression support for .lz files.\n",
        "  --enable-threads=METHOD Supported METHODS are `yes', `no', `posix'.\n",
        "  --disable-doc           do not install documentation files\n",
        "Optional Packages:\n",
        "  --with-pic              try to use only PIC objects\n",
    );

    // `bool_flag` can express an on/off switch and nothing else, so the
    // shape has to be read rather than assumed. autoconf states it: an
    // `=ARG` suffix means the flag takes a VALUE, and a valued flag either
    // toggles many macros at once (--enable-encoders drives 12) or changes
    // one macro's value (--enable-assume-ram=256 moves ASSUME_RAM from 128).
    #[test]
    fn a_valued_flag_is_distinguished_from_a_boolean_one() {
        let flags = parse_configure_flags(XZ_HELP);
        let by_name = |n: &str| flags.iter().find(|f| f.name == n).cloned();

        assert_eq!(
            by_name("--enable-encoders").map(|f| f.valued),
            Some(true),
            "`--enable-encoders=LIST` takes a value: {flags:#?}"
        );
        assert_eq!(
            by_name("--enable-threads").map(|f| f.valued),
            Some(true),
            "so does `--enable-threads=METHOD`: {flags:#?}"
        );
        assert_eq!(
            by_name("--enable-lzip-decoder").map(|f| f.valued),
            Some(false),
            "`--disable-lzip-decoder` is a plain switch: {flags:#?}"
        );
        assert_eq!(
            by_name("--enable-doc").map(|f| f.valued),
            Some(false),
            "and so is `--disable-doc`: {flags:#?}"
        );
    }

    // A `--disable-X` and an `--enable-X` are ONE option, not two: autoconf
    // treats the first as `--enable-X=no`. Recording both would emit two
    // flags for one project choice.
    #[test]
    fn the_disable_spelling_is_normalised_to_the_enable_one() {
        let flags = parse_configure_flags(XZ_HELP);
        assert!(
            flags.iter().any(|f| f.name == "--enable-lzip-decoder"),
            "the positive spelling names the pair: {flags:#?}"
        );
        assert!(
            !flags.iter().any(|f| f.name.starts_with("--disable-")),
            "no flag should keep the negative spelling: {flags:#?}"
        );
    }

    // autoconf's preamble documents the SYNTAX with `--enable-FEATURE` and
    // `--with-PACKAGE`. Those are not options this project exposes, and
    // emitting a flag for them would put a knob in every module.
    #[test]
    fn autoconfs_own_placeholders_are_not_project_options() {
        let flags = parse_configure_flags(XZ_HELP);
        assert!(
            !flags
                .iter()
                .any(|f| f.name.contains("FEATURE") || f.name.contains("PACKAGE")),
            "the generic help is not an option: {flags:#?}"
        );
        // Nor are autoconf's real-but-generic switches, which every
        // generated configure carries: --with-pic, --enable-shared,
        // --enable-option-checking. A knob for those would appear in every
        // converted module and belong to none of them.
        assert!(
            !flags
                .iter()
                .any(|f| f.name == "--with-pic" || f.name == "--enable-option-checking"),
            "autoconf's own switches are not this project's options: {flags:#?}"
        );
        assert!(
            flags.iter().any(|f| f.name == "--enable-lzip-decoder"),
            "but the project's own are kept: {flags:#?}"
        );
    }

    // A recipe that runs one level UP from the directory it generates into
    // writes `mv gl/x.h-t gl/x.h`. `config_header_output` re-adds the
    // directory from `shadow_dir`, so keeping the path here doubled it —
    // `gl/gl/x.h`, generated two levels from where its include path points.
    // gnulib's own recipes cd into `gl/` first and so never showed this.
    #[test]
    fn a_path_qualified_rename_yields_just_the_filename() {
        const STREAM: &str = "\
make[1]: Entering directory '/build'\n\
/bin/sed -e 's|@''X''@|1|g' < /src/gl/mystring.in.h > gl/mystring.h-t\n\
mv gl/mystring.h-t gl/mystring.h\n\
make[1]: Leaving directory '/build'\n\
";
        let found = parse_replacement_headers(STREAM, Path::new("/build"));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(
            found[0].output, "mystring.h",
            "the directory comes from shadow_dir, so carrying it here too \
             doubles it"
        );
    }

    // A third recipe form: sed's `w` writes the output, so there is no `>`
    // redirect at all (`-n` suppresses the default output). Keying on the
    // redirect missed exactly libidn2's uniconv.h, unistr.h and unitypes.h —
    // 3 of its 14 unistring headers, and the three the build then failed on.
    #[test]
    fn a_recipe_that_writes_with_sed_w_is_found() {
        const STREAM: &str = "\
make[1]: Entering directory '/build/unistring'\n\
sed -e 1h -e '1s,.*,/* GENERATED */,' -e 1G -n -e 'w uniconv.h-t' /src/unistring/uniconv.in.h\n\
mv uniconv.h-t uniconv.h\n\
make[1]: Leaving directory '/build/unistring'\n\
";
        let found = parse_replacement_headers(STREAM, Path::new("/build"));
        assert_eq!(found.len(), 1, "no `>` in this recipe: {found:#?}");
        assert_eq!(found[0].output, "uniconv.h");
        assert_eq!(
            found[0].template, "/src/unistring/uniconv.in.h",
            "the template is the trailing positional argument, after the \
             whole script"
        );
    }

    // The widening that made form 3 work must not make a bare mention of a
    // template into a recipe: `make` announces prerequisites and `cp`s them
    // around, and a line that names a .in.h without writing anything is not
    // a generation recipe.
    #[test]
    fn a_line_that_only_mentions_a_template_is_not_a_recipe() {
        const STREAM: &str = "\
make[1]: Entering directory '/build/gl'\n\
make: Nothing to be done for '/src/gl/string.in.h'\n\
mv string.h-t string.h\n\
make[1]: Leaving directory '/build/gl'\n\
";
        assert!(
            parse_replacement_headers(STREAM, Path::new("/build")).is_empty(),
            "a rename with no recipe that WROTE anything is not a \
             replacement header"
        );
    }

    // The recipe names a spliced file relative to its own directory in an
    // IN-TREE build and absolutely in an out-of-tree one — and the translator
    // always builds out of tree, so the absolute form is the one that ships.
    // Handling only the relative form emitted `gl/<absolute path>`, a
    // concatenation naming nothing, for all 14 of libidn2's splices. Every
    // unit test still passed, because they were all written in the relative
    // form I had measured by hand.
    #[test]
    fn an_absolutely_named_splice_is_rebased_onto_the_module() {
        let rebased = rebase_splices(
            &[("marker".to_string(), "/src/gl/c++defs.h".to_string())],
            "gl",
            Path::new("/src"),
        );
        assert_eq!(
            rebased,
            vec![("marker".to_string(), "gl/c++defs.h".to_string())],
            "an absolute path is made module-relative, never concatenated \
             onto the shadow dir"
        );
    }

    #[test]
    fn a_relatively_named_splice_is_joined_onto_the_shadow_dir() {
        let rebased = rebase_splices(
            &[("marker".to_string(), "./c++defs.h".to_string())],
            "gl",
            Path::new("/src"),
        );
        assert_eq!(
            rebased,
            vec![("marker".to_string(), "gl/c++defs.h".to_string())],
            "the relative form is named against the recipe's own directory, \
             which `shadow_dir` already holds in module terms"
        );
    }

    // The negative: a recipe with no `r` must not acquire one. Without this
    // the field could be populated by anything and the positive test above
    // would still pass.
    #[test]
    fn a_recipe_without_a_splice_has_none() {
        const STREAM: &str = "\
make[1]: Entering directory '/build/gl'\n\
/bin/sed -e 's|@''X''@|1|g' < /src/gl/limits.in.h > limits.h-t\n\
mv limits.h-t limits.h\n\
make[1]: Leaving directory '/build/gl'\n\
";
        let found = parse_replacement_headers(STREAM, Path::new("/build"));
        assert!(
            found[0].splices.is_empty(),
            "no `r` in the recipe: {:#?}",
            found[0]
        );
    }

    // A redirect with no rename is some other recipe, not a header.
    #[test]
    fn a_redirect_without_a_rename_yields_nothing() {
        const STREAM: &str = "\
make[1]: Entering directory '/build/gl'\n\
/bin/sed -e 's|@''X''@|1|g' < /src/gl/limits.in.h > scratch.txt\n\
make[1]: Leaving directory '/build/gl'\n\
";
        assert!(
            parse_replacement_headers(STREAM, Path::new("/build")).is_empty(),
            "only an atomic rename to a .h completes a replacement header"
        );
    }

    #[test]
    fn a_conditional_test_variable_resolves_through_its_indirection() {
        let db = "am__EXEEXT_1 = test_md5$(EXEEXT)\n\
             am__EXEEXT_2 = test_sha1$(EXEEXT) test_sha256$(EXEEXT)\n\
             check_PROGRAMS = test_base$(EXEEXT) $(am__EXEEXT_1) $(am__EXEEXT_2)\n\
             TESTS = $(check_PROGRAMS)\n";
        let vars = single_scope(db);
        assert_eq!(
            classify_tests(&vars),
            vec![
                TestEntry::Binary("test_base".to_string()),
                TestEntry::Binary("test_md5".to_string()),
                TestEntry::Binary("test_sha1".to_string()),
                TestEntry::Binary("test_sha256".to_string()),
            ],
            "a reference inside an expansion has to resolve too, or a project \
             using automake conditionals reports its tests as the literal text \
             `$(am__EXEEXT_1)`"
        );
    }

    // Being `#include`d does NOT mean a file is not also compiled.
    // libmicrohttpd's test_postprocessor_md.c includes internal.c, mhd_str.c
    // and mhd_panic.c, AND its `_SOURCES` declares all three, and automake
    // compiles each into a per-target object. Treating the include as proof
    // they are textual-only removed them from `srcs`, and the target failed
    // to link on the symbols they define.
    #[test]
    fn a_source_that_is_both_included_and_compiled_stays_compiled() {
        let mut targets = vec![Target {
            name: "prog".to_string(),
            kind: TargetKind::Executable,
            sources: vec!["main.c".to_string(), "helper.c".to_string()],
            ..Default::default()
        }];
        // main.c includes helper.c, but helper.c is a declared source of the
        // same target, so it is compiled in its own right.
        let dir =
            std::env::temp_dir().join(format!("bzlf_bothroles_{}_{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.c"), "#include \"helper.c\"\n").unwrap();
        std::fs::write(dir.join("helper.c"), "int helper(void) { return 1; }\n").unwrap();

        inject_textual_includes(&mut targets, &dir);

        assert!(
            targets[0].sources.contains(&"helper.c".to_string()),
            "a declared source has its own compile command, so it must stay in \
             srcs however many files include it — dropping it is an undefined \
             symbol at link: {:?}",
            targets[0].sources
        );
        assert!(
            !targets[0].textual_sources.contains(&"helper.c".to_string()),
            "and it must not ALSO be textual_hdrs, which would stage the same \
             file twice: {:?}",
            targets[0].textual_sources
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // libmicrohttpd defines `am__EXEEXT_1` FIVE times — once per directory
    // that has a first conditional, including one directory where it is
    // EMPTY. Last-wins kept whichever came last and lost the rest, so its
    // tests escalated as the literal text `$(am__EXEEXT_1)`. Same reason
    // primaries and TESTS accumulate: make never sees these together.
    #[test]
    fn a_shared_library_absorbing_a_static_one_escalates() {
        // Bazel rejects this at ANALYSIS with an error naming generated
        // rules, so without an escalation the user sees a failure they
        // cannot connect to anything they wrote.
        const STREAM: &str = "\
make[1]: Entering directory '/src/gl'\n\
gcc -c -o helper.o helper.c\n\
libtool --tag=CC --mode=link gcc helper.o -o libgnu.la\n\
make[1]: Leaving directory '/src/gl'\n\
make[1]: Entering directory '/src/lib'\n\
gcc -c -o main.o main.c\n\
libtool --tag=CC --mode=link gcc main.o ../gl/libgnu.la -o libthing.la\n\
make[1]: Leaving directory '/src/lib'\n\
";
        let db = "noinst_LTLIBRARIES = gl/libgnu.la\nlibgnu_la_SOURCES = helper.c\n\
             lib_LTLIBRARIES = lib/libthing.la\nlibthing_la_SOURCES = main.c\n";
        let (_, escalations, _) = to_graph(
            &parse_commands(STREAM, Path::new("/src")),
            &VariableDatabase::parse(db, Path::new("/src")),
            "conv",
            Path::new("/src"),
            Path::new("/src"),
        );

        let item = escalations
            .iter()
            .find(|e| e.kind == "shared_library_absorbs_static")
            .unwrap_or_else(|| {
                panic!("a shared library linking a static one must escalate: {escalations:#?}")
            });
        assert!(
            item.gap.contains("libgnu"),
            "and must name the absorbed archive: {}",
            item.gap
        );
    }

    // The other direction, so the check cannot be wired to always fire: a
    // shared library with no static dependency is the ordinary case and must
    // stay silent. xz, expat and jansson are all this shape.
    #[test]
    fn a_shared_library_with_no_static_dependency_does_not_escalate() {
        const STREAM: &str = "\
make[1]: Entering directory '/src/lib'\n\
gcc -c -o main.o main.c\n\
libtool --tag=CC --mode=link gcc main.o -o libthing.la\n\
make[1]: Leaving directory '/src/lib'\n\
";
        let (_, escalations, _) = to_graph(
            &parse_commands(STREAM, Path::new("/src")),
            &VariableDatabase::parse(
                "lib_LTLIBRARIES = lib/libthing.la\nlibthing_la_SOURCES = main.c\n",
                Path::new("/src"),
            ),
            "conv",
            Path::new("/src"),
            Path::new("/src"),
        );

        assert!(
            !escalations
                .iter()
                .any(|e| e.kind == "shared_library_absorbs_static"),
            "no static dependency, nothing to escalate: {escalations:#?}"
        );
    }

    #[test]
    fn a_noinst_libtool_library_is_not_shared() {
        // `noinst_` means built but never installed — libtool makes it a
        // static CONVENIENCE archive to be absorbed into whatever links it,
        // and produces no .so at all. libidn2's libgnu.la is one; the real
        // build emits libgnu.a and nothing else, while lib_LTLIBRARIES
        // libidn2.la does produce libidn2.so.
        //
        // Treating every LTLIBRARIES as shared made the module emit a
        // cc_shared_library for libgnu AND absorb it into libidn2's, which
        // Bazel rejects outright: "Two shared libraries in dependencies link
        // the same library statically."
        const STREAM: &str = "\
make[1]: Entering directory '/src/gl'\n\
gcc -c -o helper.o helper.c\n\
libtool --tag=CC --mode=link gcc helper.o -o libgnu.la\n\
make[1]: Leaving directory '/src/gl'\n\
";
        let (graph, _, _) = to_graph(
            &parse_commands(STREAM, Path::new("/src")),
            &VariableDatabase::parse(
                "noinst_LTLIBRARIES = libgnu.la\nlibgnu_la_SOURCES = helper.c\n",
                Path::new("/src"),
            ),
            "conv",
            Path::new("/src"),
            Path::new("/src"),
        );

        assert!(
            !graph.targets.is_empty(),
            "the convenience library is still a target"
        );
        assert!(
            !graph.targets[0].is_shared,
            "a noinst_ libtool library is a static convenience archive, not a \
             shared library: {:#?}",
            graph.targets[0]
        );
    }

    // The other direction, so the destination check cannot be wired to
    // always-false: an INSTALLED libtool library really is shared.
    #[test]
    fn a_lib_libtool_library_is_still_shared() {
        const STREAM: &str = "\
make[1]: Entering directory '/src/lib'\n\
gcc -c -o a.o a.c\n\
libtool --tag=CC --mode=link gcc a.o -o libthing.la\n\
make[1]: Leaving directory '/src/lib'\n\
";
        let (graph, _, _) = to_graph(
            &parse_commands(STREAM, Path::new("/src")),
            &VariableDatabase::parse(
                "lib_LTLIBRARIES = libthing.la\nlibthing_la_SOURCES = a.c\n",
                Path::new("/src"),
            ),
            "conv",
            Path::new("/src"),
            Path::new("/src"),
        );

        assert!(
            graph.targets.first().is_some_and(|t| t.is_shared),
            "lib_LTLIBRARIES installs a .so and must stay shared: {:#?}",
            graph.targets
        );
    }

    // libmicrohttpd's shape: `am__EXEEXT_1` means `test_md5` in one
    // directory, nothing in the next (its conditional was false) and
    // `perf_replies` in a third, and each directory's `TESTS` resolves
    // against its own definition. An empty definition contributes nothing
    // and erases nothing.
    #[test]
    fn each_directory_resolves_its_conditional_tests_against_its_own_definition() {
        const DB: &str = "\
# make[1]: Entering directory '/b/src/microhttpd'\n\
am__EXEEXT_1 = test_md5$(EXEEXT)\n\
check_PROGRAMS = $(am__EXEEXT_1)\n\
TESTS = $(check_PROGRAMS)\n\
# make[1]: Leaving directory '/b/src/microhttpd'\n\
# make[1]: Entering directory '/b/src/testcurl'\n\
am__EXEEXT_1 = \n\
check_PROGRAMS = $(am__EXEEXT_1)\n\
TESTS = $(check_PROGRAMS)\n\
# make[1]: Leaving directory '/b/src/testcurl'\n\
# make[1]: Entering directory '/b/src/perf'\n\
am__EXEEXT_1 = perf_replies$(EXEEXT)\n\
check_PROGRAMS = $(am__EXEEXT_1)\n\
TESTS = $(check_PROGRAMS)\n\
# make[1]: Leaving directory '/b/src/perf'\n\
";
        assert_eq!(
            classify_tests_per_directory(&parsed(DB)),
            vec![
                TestEntry::Binary("test_md5".to_string()),
                TestEntry::Binary("perf_replies".to_string()),
            ],
            "the empty definition must not erase the real ones, and every \
             directory's contribution has to survive"
        );
    }

    // A REPEAT is not a cycle. `make -p` reports a directory's database once
    // per sub-make that reads it, so accumulation gives libidn2
    // `TESTS = $(am__EXEEXT_2) $(dist_check_SCRIPTS) $(am__EXEEXT_2)
    // $(dist_check_SCRIPTS)` — the same reference twice. Marking a name
    // followed on first use made the second occurrence report Unresolved,
    // and the literal text `$(am__EXEEXT_2)` shipped in the escalation
    // alongside the very tests it expands to.
    #[test]
    fn a_reference_repeated_by_accumulation_still_expands() {
        let db = "am__EXEEXT_2 = test_a$(EXEEXT)\n\
             check_PROGRAMS = $(am__EXEEXT_2)\n\
             TESTS = $(am__EXEEXT_2) $(am__EXEEXT_2)\n";
        let vars = single_scope(db);
        assert_eq!(
            classify_tests(&vars),
            vec![TestEntry::Binary("test_a".to_string())],
            "the second occurrence is the same declaration seen twice, not a \
             cycle — it must expand and then dedup, never escalate as literal \
             make syntax"
        );
    }

    // The bound has to exist, because make's own expander is not being
    // reimplemented: a variable that refers to itself must terminate rather
    // than recurse forever, and it must not silently yield nothing.
    #[test]
    fn a_self_referential_variable_terminates_as_unresolved() {
        let vars = single_scope("LOOP = $(LOOP)\nTESTS = $(LOOP)\n");
        assert_eq!(
            classify_tests(&vars),
            vec![TestEntry::Unresolved("$(LOOP)".to_string())],
            "cycles are the reason expansion is bounded; the honest answer at \
             the bound is that we could not resolve it"
        );
    }

    // The branch that is load-bearing rather than a fallback: 2 of the 6
    // surveyed projects have a TESTS entry no database lookup resolves.
    #[test]
    fn a_tests_entry_that_resolves_to_nothing_is_unresolved_not_guessed() {
        let vars = single_scope("check_PROGRAMS = test_a$(EXEEXT)\nTESTS = $(libgd_tests)\n");
        assert_eq!(
            classify_tests(&vars),
            vec![TestEntry::Unresolved("$(libgd_tests)".to_string())],
            "libgd's shape. The variable is defined nowhere make -p reports, so \
             the honest answer is that we do not know — escalate rather than \
             silently drop it or guess it is a binary"
        );
    }

    // `TESTS` is per-DIRECTORY under recursive make. jansson declares
    // `run-suites` in test/ and `scripts/clang-format-check` at the root;
    // a flattened last-wins kept whichever came second and reported half
    // the suite. Every scope's `TESTS` is a test, qualified by the
    // directory that declared it.
    #[test]
    fn tests_declared_in_several_directories_all_survive() {
        const DB: &str = "\
# make[1]: Entering directory '/b/test'\n\
TESTS = run-suites\n\
# make[1]: Leaving directory '/b/test'\n\
TESTS = scripts/format-check\n\
";
        assert_eq!(
            classify_tests_per_directory(&parsed(DB)),
            vec![
                TestEntry::Script("scripts/format-check".to_string()),
                TestEntry::Script("test/run-suites".to_string()),
            ],
            "both declarations survive — make never sees them together, so \
             combining them is ours to do, by name, above the scopes — with \
             the root's first"
        );
    }

    // The same declaration arrives more than once: automake recurses into
    // `.` for a `SUBDIRS = . sub` Makefile, so `make -p` prints that
    // directory's database twice, and jansson's five `TESTS = ` lines are
    // really two distinct tests. Shipped un-deduplicated, the escalation
    // listed `run-suites` twice and `clang-format-check` three times, which
    // reads as five separate problems to whoever has to resolve it.
    #[test]
    fn a_test_declared_once_is_not_reported_several_times() {
        const DB: &str = "\
# make[1]: Entering directory '/b/test'\n\
TESTS = run-suites\n\
# make[1]: Leaving directory '/b/test'\n\
TESTS = scripts/format-check\n\
# make[1]: Entering directory '/b/test'\n\
TESTS = run-suites\n\
# make[1]: Leaving directory '/b/test'\n\
TESTS = scripts/format-check\n\
";
        assert_eq!(
            classify_tests_per_directory(&parsed(DB)),
            vec![
                TestEntry::Script("scripts/format-check".to_string()),
                TestEntry::Script("test/run-suites".to_string()),
            ],
            "deduplicated, and in scope order — a repeat is the same \
             declaration seen twice, not a second test"
        );
    }

    #[test]
    fn a_project_with_no_tests_variable_classifies_nothing() {
        let vars = single_scope("check_PROGRAMS = helper$(EXEEXT)\n");
        assert!(
            classify_tests(&vars).is_empty(),
            "gzip declares check_PROGRAMS and no TESTS at all: building a \
             helper binary is not the same as declaring a test, and inventing \
             one would report a test the project never ran"
        );
    }

    // Identity comes from the primaries, not from artifact paths. The
    // destination prefix is carried because it is where the public/private
    // signal lives — noinst_ means built but never installed.
    #[test]
    // libevent's shape: every library behind one variable, every program
    // behind automake's `$(am__EXEEXT_N)` chain, in one directory.
    #[test]
    fn primaries_declared_through_variables_expand() {
        const DATABASE: &str = "\
EXEEXT = \n\
am__append_1 = libevent_pthreads.la\n\
LIBEVENT_LIBS_LA = libevent.la libevent_core.la $(am__append_1) $(am__append_3)\n\
lib_LTLIBRARIES = $(LIBEVENT_LIBS_LA)\n\
am__EXEEXT_3 = sample/hello-world$(EXEEXT) $(am__EXEEXT_2)\n\
am__EXEEXT_4 = $(am__EXEEXT_3)\n\
noinst_PROGRAMS = $(am__EXEEXT_4)\n\
";
        let names: Vec<String> = declared_targets_in(Path::new("/b"), &single_scope(DATABASE))
            .into_iter()
            .map(|d| format!("{}:{}", d.destination, d.name))
            .collect();
        assert_eq!(
            names,
            vec![
                "lib:libevent.la",
                "lib:libevent_core.la",
                "lib:libevent_pthreads.la",
                "noinst:sample/hello-world",
            ],
            "three names through two levels of reference; an undefined \
             $(am__append_3) — a false conditional — contributes nothing"
        );
    }

    // hwloc: `tests/hwloc` says `am__EXEEXT_2 = shmem` and `utils/hwloc`
    // reuses the name for something else. Each directory's check_PROGRAMS
    // must expand in its OWN scope, or one of the two targets vanishes —
    // shmem did, built by `make check` and declared by nothing.
    #[test]
    fn each_directory_expands_its_own_primaries() {
        const DATABASE: &str = "\
# make[1]: Entering directory '/b/utils'\n\
am__EXEEXT_2 = hwloc-ps\n\
bin_PROGRAMS = hwloc-calc $(am__EXEEXT_2)\n\
# make[1]: Leaving directory '/b/utils'\n\
# make[1]: Entering directory '/b/tests'\n\
am__EXEEXT_2 = shmem\n\
check_PROGRAMS = hwloc_bitmap $(am__EXEEXT_1) $(am__EXEEXT_2)\n\
TESTS = $(check_PROGRAMS)\n\
# make[1]: Leaving directory '/b/tests'\n\
";
        let names: Vec<String> = parsed(DATABASE)
            .declared_targets()
            .into_iter()
            .map(|d| format!("{}:{}", d.destination, d.name))
            .collect();
        assert_eq!(
            names,
            vec![
                "bin:hwloc-calc",
                "bin:hwloc-ps",
                "check:hwloc_bitmap",
                "check:shmem",
            ],
            "both directories' conditional targets survive, each from its own \
             definition of am__EXEEXT_2"
        );

        // And the test list: the undefined $(am__EXEEXT_1) is a false
        // conditional (Windows-only), not an unresolved reference.
        let entries = classify_tests_per_directory(&parsed(DATABASE));
        assert_eq!(
            entries,
            vec![
                TestEntry::Binary("hwloc_bitmap".to_string()),
                TestEntry::Binary("shmem".to_string()),
            ],
            "{entries:#?}"
        );

        // A project's OWN undefined variable is still escalated.
        let entries = classify_tests(&single_scope("TESTS = $(MY_SUITE)\n"));
        assert_eq!(
            entries,
            vec![TestEntry::Unresolved("$(MY_SUITE)".to_string())]
        );
    }

    // A program under two primaries is ONE target — libmicrohttpd declares
    // authorization_example as noinst_ and EXTRA_ — and the one that says
    // more wins, so it renders once, as the noinst binary it is.
    #[test]
    fn a_name_under_two_primaries_is_declared_once_with_the_more_specific_destination() {
        let vars = single_scope(
            "noinst_PROGRAMS = authorization_example\nEXTRA_PROGRAMS = authorization_example demo\ncheck_PROGRAMS = demo\n",
        );
        let names: Vec<String> = declared_targets_in(Path::new("/b"), &vars)
            .iter()
            .map(|d| format!("{}:{}", d.destination, d.name))
            .collect();
        assert_eq!(names, vec!["noinst:authorization_example", "check:demo"]);
    }

    #[test]
    fn declared_targets_recovers_names_destinations_and_primaries() {
        let declared = declared_targets_in(Path::new("/b"), &single_scope(DATABASE));
        assert_eq!(
            declared,
            vec![
                DeclaredTarget {
                    name: "greeter".to_string(),
                    destination: "bin".to_string(),
                    primary: "PROGRAMS".to_string(),
                    directory: PathBuf::from("/b"),
                },
                DeclaredTarget {
                    name: "libgreet.a".to_string(),
                    destination: "noinst".to_string(),
                    primary: "LIBRARIES".to_string(),
                    directory: PathBuf::from("/b"),
                },
                DeclaredTarget {
                    name: "libshout.la".to_string(),
                    destination: "lib".to_string(),
                    primary: "LTLIBRARIES".to_string(),
                    directory: PathBuf::from("/b"),
                },
            ],
            "$(EXEEXT) must be expanded from the database, not assumed empty"
        );
    }

    // The shape of a real `make -p -n` on xz 5.4.7: recursive make prints one
    // database per subdirectory, so the SAME primary is defined in four of
    // them. Flattened with last-wins, this dropped the project's namesake
    // binary while reporting success.
    const RECURSIVE_PRIMARIES: &str = "\
# make[2]: Entering directory '/b/src/liblzma'
lib_LTLIBRARIES = liblzma.la
EXEEXT =
# make[2]: Leaving directory '/b/src/liblzma'
# make[2]: Entering directory '/b/src/xz'
bin_PROGRAMS = xz$(EXEEXT)
EXEEXT =
xz_SOURCES = src/xz/main.c
# make[2]: Leaving directory '/b/src/xz'
# make[2]: Entering directory '/b/src/lzmainfo'
bin_PROGRAMS = lzmainfo$(EXEEXT)
EXEEXT =
lzmainfo_SOURCES = src/lzmainfo/lzmainfo.c
# make[2]: Leaving directory '/b/src/lzmainfo'
bin_PROGRAMS = $(am__EXEEXT_1) $(am__EXEEXT_2)
EXEEXT =
";

    #[test]
    fn a_primary_defined_in_several_subdirectories_keeps_every_target() {
        let names: Vec<String> = parsed(RECURSIVE_PRIMARIES)
            .declared_targets()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "liblzma.la".to_string(),
                "lzmainfo".to_string(),
                "xz".to_string()
            ],
            "recursive make defines bin_PROGRAMS once per subdirectory, and every \
             scope's declaration is a target. Also: the top level's `$(am__EXEEXT_N)` \
             references are false conditionals there, not target names."
        );
    }

    // Within one scope the last definition wins for EVERY name, primaries
    // included: `make -p` prints a variable's final value, so a second
    // definition in one scope is a restatement, never a second directory.
    // The flattened model merged primaries here; this is the assertion that
    // would go red if that merge crept back in.
    #[test]
    fn within_one_scope_the_last_definition_wins_for_every_name() {
        let vars = single_scope("xz_SOURCES = first.c\nxz_SOURCES = second.c\n");
        assert_eq!(
            vars.get("xz_SOURCES").map(String::as_str),
            Some("second.c"),
            "a per-target variable keeps make's last-assignment-wins"
        );
        let names: Vec<String> = parsed("bin_PROGRAMS = a\nbin_PROGRAMS = b\n")
            .declared_targets()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(
            names,
            vec!["b".to_string()],
            "and so does a primary: nothing accumulates inside a scope"
        );
    }

    #[test]
    fn parses_the_real_command_stream_into_a_graph() {
        let graph = graph_from_captures();
        let names: Vec<&str> = graph.targets.iter().map(|t| t.name.as_str()).collect();
        // automake's own names, sanitised for Bazel but not shortened —
        // see target_label on why stripping `lib` collides.
        assert_eq!(
            names,
            vec!["greeter", "libgreet.a", "libshout.la"],
            "{graph:#?}"
        );

        let by_name = |n: &str| {
            graph
                .targets
                .iter()
                .find(|t| t.name == n)
                .unwrap_or_else(|| panic!("missing {n}"))
        };

        // noinst_LIBRARIES -> a static archive built by `ar`.
        let greet = by_name("libgreet.a");
        assert_eq!(greet.kind, TargetKind::Library);
        assert!(!greet.is_shared, "an ar archive is not shared");
        assert_eq!(greet.sources, vec!["src/greet.c"]);

        // lib_LTLIBRARIES -> a libtool library. This is the concept with no
        // CMake analogue, and the one the whole frontend was worth writing to
        // find out about.
        let shout = by_name("libshout.la");
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
            vec!["libgreet.a", "libshout.la"],
            "both libraries are link inputs"
        );

        // `-I.` resolves TO the module root, which is recorded as
        // needs_root_include rather than as an includes entry; codegen turns
        // it into `includes = ["."]`. Same decision the CMake frontend makes,
        // for the same reason.
        //
        // `-I.` is the ONLY include this fixture's compiles carry, so both
        // lists are empty. An earlier version of this capture invented an
        // `-Isrc` and asserted `greet.includes == ["src"]`; the assertion held
        // only because the input had been written to satisfy it. Per-target
        // include attribution is genuinely exercised by
        // `a_commands_paths_resolve_against_the_directory_it_ran_in`, whose
        // stream carries two different `-I` values.
        assert!(
            greet.includes.is_empty(),
            "-I. is the module root, not an includes entry: {greet:#?}"
        );
        assert!(greet.needs_root_include, "-I. must set it: {greet:#?}");
        assert!(greeter.includes.is_empty(), "{greeter:#?}");
        assert!(greeter.needs_root_include, "{greeter:#?}");

        // include_HEADERS is automake's statement that a header is public —
        // the same claim CMake makes with install(FILES ... DESTINATION
        // include). greet.h is listed in libgreet_a_SOURCES AND installed, so
        // it is public; nothing else is.
        assert_eq!(greet.public_headers, vec!["src/greet.h"], "{greet:#?}");
        assert!(greeter.public_headers.is_empty(), "{greeter:#?}");

        // configure puts its own substitutions on every compile line, and they
        // carry QUOTES. This is where the value that motivated
        // `codegen::escape_starlark` actually comes from, so the capture is
        // the honest place to pin its shape: emitted raw into a Starlark
        // string, `PACKAGE_NAME="greeter"` closes that string early and the
        // generated module stops parsing.
        assert!(
            greet
                .local_defines
                .contains(&"PACKAGE_NAME=\"greeter\"".to_string()),
            "the define reaches the model with its quotes intact, unescaped — \
             escaping is codegen's job and happens at render time: {:#?}",
            greet.local_defines
        );
    }

    // The point of this frontend: codegen has only ever seen CMake. If the
    // model boundary is real, an Autotools graph renders through it with no
    // change to codegen at all.
    #[test]
    fn an_autotools_graph_renders_through_unmodified_codegen() {
        let graph = graph_from_captures();
        let rendered = codegen::render(&graph).build_bazel;

        assert!(
            rendered.contains("cc_library(\n    name = \"libgreet.a\","),
            "the static archive becomes a cc_library:\n{rendered}"
        );
        assert!(
            rendered.contains("cc_shared_library(\n    name = \"libshout.la_shared\","),
            "and the libtool library gets the shared-library treatment zlib \
             drove, with no autotools-specific codegen:\n{rendered}"
        );
        assert!(
            rendered.contains("cc_binary(\n    name = \"greeter\","),
            "the program becomes a cc_binary:\n{rendered}"
        );
        assert!(
            rendered.contains("\":libgreet.a\",") && rendered.contains("\":libshout.la\","),
            "with both library edges:\n{rendered}"
        );
    }

    /// A converted dependency as its conversion leaves it, plus the merged
    /// sysroot a dependent's conversion would see. Real files, because the
    /// resolution keys on paths that exist.
    fn converted_greet(tag: &str) -> (PathBuf, Dependencies) {
        let root = std::env::temp_dir().join(format!("bzlf_xmod_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let module = root.join("greet_module");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(
            module.join("MODULE.bazel"),
            "module(\n    name = \"greet\",\n    version = \"1.2\",\n)\n",
        )
        .unwrap();
        std::fs::write(
            module.join("TARGETS"),
            "library libgreet_la libgreet shared\n",
        )
        .unwrap();
        let lib = root.join("greet_install/usr/local/lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("libgreet.so"), "elf").unwrap();
        std::fs::write(lib.join("libgreet.la"), "# libtool").unwrap();
        let sysroot = root.join("sysroot");
        let deps = Dependencies::load(
            &[format!(
                "{}:{}",
                module.display(),
                root.join("greet_install").display()
            )],
            &sysroot,
        )
        .unwrap();
        (root, deps)
    }

    fn app_graph(stream: &str, deps: &Dependencies) -> (BuildGraph, Vec<NeedsAttention>) {
        let database = "bin_PROGRAMS = app\napp_SOURCES = src/main.c\n";
        let (graph, escalations, _) = to_graph_with_dependencies(
            &parse_commands(stream, Path::new("/proj")),
            &VariableDatabase::parse(database, Path::new("/proj")),
            "greet-user",
            Path::new("/proj"),
            Path::new("/proj"),
            deps,
        );
        (graph, escalations)
    }

    // The shape pkg-config produces: `-L<sysroot lib> -lgreet`. With the
    // dependency supplied it is a cross-module edge and nothing escalates;
    // without it the same line escalates the library by name. Both
    // directions, because a resolver that resolved everything and one that
    // resolved nothing each pass one of them.
    #[test]
    fn a_library_from_a_converted_dependency_is_an_edge_and_otherwise_an_escalation() {
        let (root, deps) = converted_greet("edge");
        let libdir = root.join("sysroot/usr/local/lib");
        let stream = format!(
            "make[1]: Entering directory '/proj'\n\
             gcc -c -o src/app-main.o src/main.c\n\
             gcc -o app src/app-main.o -L{} -lgreet -lm\n\
             make[1]: Leaving directory '/proj'\n",
            libdir.display()
        );

        let (graph, escalations) = app_graph(&stream, &deps);
        assert_eq!(
            graph.targets[0].external_dependencies,
            vec![ExternalDependency {
                module: "greet".to_string(),
                target: "libgreet_la".to_string(),
                shared: true,
            }],
            "{:#?}",
            graph.targets[0]
        );
        assert!(
            escalations
                .iter()
                .all(|e| e.kind != "unconverted_dependency"),
            "resolved, so nothing to escalate — and -lm is the toolchain's: {escalations:#?}"
        );

        let (graph, escalations) = app_graph(&stream, &Dependencies::default());
        assert!(graph.targets[0].external_dependencies.is_empty());
        let item = escalations
            .iter()
            .find(|e| e.kind == "unconverted_dependency")
            .unwrap_or_else(|| {
                panic!("the same line must escalate without the dependency: {escalations:#?}")
            });
        assert_eq!(
            item.subject, "-lgreet",
            "one item per library, named as the link line spelled it"
        );
        assert!(
            item.gap.contains("- `app`"),
            "and it names the rule that waits on it:\n{}",
            item.gap
        );
        assert!(
            !escalations.iter().any(|e| e.subject == "-lm"),
            "the toolchain's own runtime is not a dependency: {escalations:#?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    // A libtool-linked program names the dependency by the absolute path
    // of its `.la` (what `--with-foo-extra-libs=` and a resolved
    // `$(FOO_LIBS)` produce) rather than by `-L`/`-l`; both spellings are
    // the same edge. The wrapper invocation is what the parser reads — the
    // `libtool: link:` echo that follows it is not a command.
    #[test]
    fn an_absolute_path_into_the_sysroot_resolves_the_same_way() {
        let (root, deps) = converted_greet("abs");
        let la = root.join("sysroot/usr/local/lib/libgreet.la");
        let stream = format!(
            "make[1]: Entering directory '/proj'\n\
             gcc -c -o src/app-main.o src/main.c\n\
             /bin/bash ./libtool --tag=CC --mode=link gcc -o app src/app-main.o {}\n\
             make[1]: Leaving directory '/proj'\n",
            la.display()
        );
        let (graph, escalations) = app_graph(&stream, &deps);
        assert_eq!(
            graph.targets[0].external_dependencies.len(),
            1,
            "{:#?}",
            graph.targets[0]
        );
        assert!(
            escalations
                .iter()
                .all(|e| e.kind != "unconverted_dependency")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    // automake's `include_HEADERS` names no target, so the header a target
    // can see is promoted where it sits, and the install layout is recorded
    // so a consumer's `<greet.h>` resolves. libevent's shape too: a custom
    // install directory under includedir, declared through variables.
    #[test]
    fn installed_headers_are_promoted_to_hdrs_with_their_install_prefix() {
        let vars = parsed(
            "include_HEADERS = src/greet.h src/util.h\n\
             EVENT2_EXPORT = include/event2/buffer.h include/event2/event.h\n\
             prefix = /usr/local\n\
             includedir = ${prefix}/include\n\
             include_event2dir = $(includedir)/event2\n\
             include_event2_HEADERS = $(EVENT2_EXPORT)\n\
             noinst_HEADERS = src/private.h\n",
        );
        let mut targets = vec![
            Target {
                name: "libgreet_la".to_string(),
                kind: TargetKind::Library,
                sources: vec![
                    "src/greet.c".to_string(),
                    "src/greet.h".to_string(),
                    "src/util.h".to_string(),
                    "src/private.h".to_string(),
                ],
                ..Default::default()
            },
            Target {
                name: "demo".to_string(),
                kind: TargetKind::Executable,
                sources: vec!["src/demo.c".to_string(), "src/greet.h".to_string()],
                ..Default::default()
            },
            Target {
                name: "libevent_core_la".to_string(),
                kind: TargetKind::Library,
                sources: vec![
                    "event.c".to_string(),
                    "include/event2/buffer.h".to_string(),
                    "include/event2/event.h".to_string(),
                    "event-internal.h".to_string(),
                ],
                ..Default::default()
            },
        ];
        promote_installed_headers(&mut targets, &vars);
        assert_eq!(
            targets[0].public_headers,
            vec!["src/greet.h".to_string(), "src/util.h".to_string()]
        );
        assert_eq!(
            targets[0].sources,
            vec!["src/greet.c".to_string(), "src/private.h".to_string()],
            "an uninstalled header stays private"
        );
        assert_eq!(targets[0].strip_include_prefix.as_deref(), Some("src"));
        assert!(
            targets[1].public_headers.is_empty() && targets[1].sources.len() == 2,
            "a binary exports nothing: {:#?}",
            targets[1]
        );
        assert_eq!(
            targets[2].public_headers,
            vec![
                "include/event2/buffer.h".to_string(),
                "include/event2/event.h".to_string()
            ],
            "declared through a variable, in a custom include subdirectory"
        );
        assert_eq!(
            targets[2].strip_include_prefix.as_deref(),
            Some("include"),
            "installed as event2/buffer.h, so only `include` is stripped"
        );
        assert_eq!(
            targets[2].sources,
            vec!["event.c".to_string(), "event-internal.h".to_string()]
        );

        // No prefix when there is nothing to strip, or no single answer.
        assert_eq!(strip_for("greet.h", ""), None);
        assert_eq!(strip_for("a/x.h", "").as_deref(), Some("a"));
        assert_eq!(
            strip_for("include/event2/x.h", "event2").as_deref(),
            Some("include")
        );
        assert_eq!(
            strip_for("src/x.h", "event2"),
            None,
            "not one strip away from event2/x.h"
        );
        let mut split = vec![Target {
            name: "lib".to_string(),
            kind: TargetKind::Library,
            sources: vec!["a/x.h".to_string(), "b/y.h".to_string()],
            ..Default::default()
        }];
        promote_installed_headers(&mut split, &parsed("include_HEADERS = a/x.h b/y.h\n"));
        assert_eq!(
            split[0].strip_include_prefix, None,
            "two directories cannot both be the prefix; better none than half right"
        );
    }

    // libevent: `sed -f make-event-config.sed < config.h > include/event2/
    // event-config.hT` at make time, reached by `-I./include`. Nothing in
    // the graph produces it, so the module would compile against nothing;
    // the item has to name it and carry the recipe. config.h in the same
    // build tree IS produced (a config_header) and must not be listed.
    #[test]
    fn a_header_the_build_generated_and_nothing_reproduces_is_escalated_with_its_recipe() {
        let root = std::env::temp_dir().join(format!("bzlf_genhdr_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let build = root.join("build");
        let src = root.join("src");
        std::fs::create_dir_all(build.join("include/event2")).unwrap();
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(build.join("include/event2/event-config.h"), "").unwrap();
        std::fs::write(build.join("config.h"), "").unwrap();
        std::fs::write(build.join("event.o"), "").unwrap();
        let stream = format!(
            "/usr/bin/sed -f {src}/make-event-config.sed < config.h > include/event2/event-config.hT\n\
             mv include/event2/event-config.hT include/event2/event-config.h\n\
             gcc -I. -I./include -I{src} -c -o event.o {src}/event.c\n",
            src = src.display()
        );
        let config_h = crate::model::ConfigHeader {
            output: "config.h".to_string(),
            template: "config.h.in".to_string(),
            template_source: None,
            catalog_probes: vec![],
            values: vec![],
            options: Vec::new(),
            splices: Vec::new(),
            unresolved: Vec::new(),
            dialect: crate::model::ConfigDialect::Undef,
            shadow_dir: None,
        };
        let generated = generated_headers_in_build_tree(
            &parse_commands(&stream, &build),
            &build,
            &src,
            std::slice::from_ref(&config_h),
            &stream,
        );
        assert_eq!(generated.len(), 1, "{generated:#?}");
        assert_eq!(generated[0].0, "include/event2/event-config.h");
        assert!(
            generated[0].1.iter().any(|l| l.contains("sed -f")),
            "the recipe is the evidence an agent needs: {:#?}",
            generated[0].1
        );
        let item = generated_headers_needs_attention(&generated);
        assert_eq!(item.kind, "generated_headers");
        assert!(
            item.gap.contains("include/event2/event-config.h") && item.gap.contains("sed -f"),
            "{}",
            item.gap
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    // hwloc: utils/hwloc builds `hwloc-bind` from hwloc-bind.c and tests/hwloc
    // builds `hwloc_bind` from hwloc_bind.c; both own `hwloc_bind_SOURCES`.
    // The flattened map cannot hold both, and taking the last handed the
    // utility a file that does not exist in its directory.
    #[test]
    fn a_per_target_variable_is_read_from_the_directory_that_built_the_target() {
        const DATABASE: &str = "\
# make[1]: Entering directory '/b/utils'\n\
# Variables\n\
bin_PROGRAMS = hwloc-bind\n\
hwloc_bind_SOURCES = hwloc-bind.c\n\
# make[1]: Leaving directory '/b/utils'\n\
# make[1]: Entering directory '/b/tests'\n\
# Variables\n\
check_PROGRAMS = hwloc_bind\n\
hwloc_bind_SOURCES = hwloc_bind.c\n\
# make[1]: Leaving directory '/b/tests'\n\
";
        const STREAM: &str = "\
make[1]: Entering directory '/b/utils'\n\
gcc -c -o hwloc-bind.o /s/utils/hwloc-bind.c\n\
gcc -o hwloc-bind hwloc-bind.o\n\
make[1]: Leaving directory '/b/utils'\n\
make[1]: Entering directory '/b/tests'\n\
gcc -c -o hwloc_bind.o /s/tests/hwloc_bind.c\n\
gcc -o hwloc_bind hwloc_bind.o\n\
make[1]: Leaving directory '/b/tests'\n\
";
        let db = parsed(DATABASE);
        assert_eq!(
            db.scope(Path::new("/b/utils")).unwrap()["hwloc_bind_SOURCES"],
            "hwloc-bind.c"
        );
        assert_eq!(
            db.scope(Path::new("/b/tests")).unwrap()["hwloc_bind_SOURCES"],
            "hwloc_bind.c"
        );

        let (graph, _, _) = to_graph(
            &parse_commands(STREAM, Path::new("/b")),
            &VariableDatabase::parse(DATABASE, Path::new("/b")),
            "hwloc",
            Path::new("/s"),
            Path::new("/s"),
        );
        let sources: Vec<(String, Vec<String>)> = graph
            .targets
            .iter()
            .map(|t| (t.name.clone(), t.sources.clone()))
            .collect();
        assert_eq!(
            sources,
            vec![
                (
                    "hwloc-bind".to_string(),
                    vec!["utils/hwloc-bind.c".to_string()]
                ),
                (
                    "hwloc_bind".to_string(),
                    vec!["tests/hwloc_bind.c".to_string()]
                ),
            ],
            "each target reads its own directory's declaration"
        );
    }

    // `make -p` prints a directory's database AFTER its sub-makes return, so
    // the parent's TESTS follow the child's `Leaving` line. hwloc's
    // utils/hwloc has nine TESTS and a SUBDIRS child with one; splitting on
    // `Entering` alone gave the child all ten.
    #[test]
    fn a_database_printed_after_a_child_returns_belongs_to_the_parent() {
        const DATABASE: &str = "\
# make[1]: Entering directory '/b/utils'\n\
# make[2]: Entering directory '/b/utils/sub'\n\
# Variables\n\
TESTS = child.sh\n\
# make[2]: Leaving directory '/b/utils/sub'\n\
# Variables\n\
TESTS = parent-a.sh parent-b.sh\n\
# make[1]: Leaving directory '/b/utils'\n\
# Variables\n\
TESTS = top.sh\n\
";
        let db = parsed(DATABASE);
        let tests_of = |dir: &str| -> Vec<String> {
            db.scope(Path::new(dir))
                .and_then(|vars| vars.get("TESTS"))
                .cloned()
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_string)
                .collect()
        };
        assert_eq!(tests_of("/b/utils/sub"), vec!["child.sh"]);
        assert_eq!(tests_of("/b/utils"), vec!["parent-a.sh", "parent-b.sh"]);
        assert_eq!(
            tests_of("/b"),
            vec!["top.sh"],
            "after every Leaving, the top-level make's own, keyed by the build root"
        );
        assert_eq!(db.root()["TESTS"], "top.sh");

        let entries = classify_tests_per_directory(&parsed(DATABASE));
        let names: Vec<String> = entries
            .iter()
            .map(|e| match e {
                TestEntry::Script(s) | TestEntry::Binary(s) | TestEntry::Unresolved(s) => s.clone(),
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "top.sh",
                "utils/sub/child.sh",
                "utils/parent-a.sh",
                "utils/parent-b.sh",
            ],
            "each script qualified by the directory that declared it, once; \
             the root's lead even though make prints its database last"
        );
    }
}
