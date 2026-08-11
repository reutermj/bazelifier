//! The translator → agent handoff: per-conversion, agent-actionable
//! descriptions of a gap the translator could not confidently resolve for
//! THIS project, rendered as `needs_attention/<NNN>-<slug>.md`.
//!
//! Holds both halves — the rendering, and the text of every escalation the
//! translator can emit. The escalations are pure functions of the facts the
//! frontend hands them, so they live here rather than in the frontend that
//! detects the gaps.
//!
//! The text is output and goes stale like any other output; escalations
//! giving substantive guidance carry a test asserting on that guidance. See
//! docs/architecture/needs-attention-interface.md.

use std::collections::HashMap;

/// A gap the translator could not confidently resolve for a specific
/// conversion — written into the output tree's `needs_attention/` for
/// whoever picks up this converted project to address. See
/// docs/architecture/needs-attention-interface.md.
///
/// Deliberately not part of `model::BuildGraph`: the graph is what the
/// conversion *did* produce, and an escalation is what it couldn't. Codegen
/// never reads these, so they ride alongside the graph on the frontend's
/// `Discovery` rather than inside it — and they live here, with the text
/// and rendering that are the only things that ever touch them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsAttention {
    /// What KIND of gap this is, as a stable machine key.
    ///
    /// One value per constructor in this module, and the only field a tool
    /// may key on. The title cannot serve: it is prose, it is reworded
    /// routinely, and the `<NNN>-<slug>` filename is derived from it — so
    /// anything keyed on either silently re-partitions when someone improves
    /// the wording. Constructors, by contrast, have only ever been ADDED.
    ///
    /// Renaming one is therefore a deliberate schema change, not an edit.
    pub kind: &'static str,
    /// What the gap is ABOUT — the target, header, or file — so two items of
    /// the same kind in one conversion are distinguishable.
    ///
    /// Best-effort by design: an item covering several things (every inert
    /// convenience target, say) names the set rather than inventing an
    /// identifier for it.
    pub subject: String,
    pub title: String,
    pub gap: String,
    pub context: String,
    pub expected_output: String,
}

/// Renders one escalation as the fixed section structure every
/// `needs_attention/<NNN>-<slug>.md` follows. See
/// docs/architecture/needs-attention-interface.md.
pub fn render(item: &NeedsAttention) -> String {
    let NeedsAttention {
        kind,
        subject,
        title,
        gap,
        context,
        expected_output,
    } = item;
    // The subject is flattened onto one line. It carries values from the
    // project — a file path, a CMake type, a joined list — and a newline in it
    // would end the header early, silently turning the rest into body text
    // that a parser then reads as prose. Quoted for the same reason a leading
    // `[` or a `: ` would otherwise change how it parses.
    let subject = subject.replace(['\n', '\r'], " ");
    let subject = subject.replace('"', "'");
    format!(
        "---\n\
         kind: {kind}\n\
         subject: \"{subject}\"\n\
         ---\n\n\
         # {title}\n\n\
         ## Gap\n\n{gap}\n\n\
         ## Context\n\n{context}\n\n\
         ## Expected output\n\n{expected_output}\n"
    )
}

/// Slugifies `title` into a filesystem-safe name (lowercase, `-`
/// separated), for use in `needs_attention/<NNN>-<slug>.md`.
pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

/// Escalates sources the translator could not place inside the generated
/// module. The question that decides the resolution is whether the file is
/// part of the source deliverable — see the tier discussion in
/// docs/architecture/cmake-frontend.md.
pub fn sources_outside_deliverable_needs_attention(
    target_name: &str,
    outside_deliverable: &[String],
) -> NeedsAttention {
    let title = format!("Target '{target_name}' has sources the module cannot reach");
    NeedsAttention {
        kind: "sources_outside_deliverable",
        subject: target_name.to_string(),
        gap: format!(
            "Target '{target_name}' compiles {} source file(s) that the translator could not \
             place inside the generated module:\n\n{}\n\nThey were left out of the generated \
             rule's `srcs`. The module's root is derived, not assumed: it is the deepest \
             directory containing both the project and everything the build references \
             from inside the declared DELIVERABLE ROOT. These files sit outside that \
             deliverable root, so the module was not widened to cover them — and a Bazel \
             label cannot refer to anything above its own module root.",
            outside_deliverable.len(),
            outside_deliverable
                .iter()
                .map(|p| format!("- `{p}`"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        context: format!(
            "The question that decides what to do here is NOT where the file sits on this \
             machine — it is whether the file is part of the source deliverable being \
             converted (the tarball, checkout, or directory the project ships as its \
             sources).\n\n\
             If it IS part of the deliverable — typically a sibling directory like \
             `../shared/` that ships alongside the project — then nothing is wrong with the \
             project, and nothing is wrong with the translator either: the deliverable root \
             was simply declared too narrowly. It is set by `--deliverable-root` (or the \
             `deliverable_root` attribute on the conversion rule) and defaults to the \
             project's own source directory, which is why a sibling directory falls outside \
             it \
             unless you say otherwise. Re-running the conversion with a root that contains \
             both the project and these files widens the module to cover them, rewrites every \
             path relative to the new root, and emits no escalation at all. That is the \
             intended resolution here, and it requires no edit to the generated output. \
             Failing that — if the files ship separately rather than in one deliverable — \
             convert the directory that owns them into its own Bazel module and depend on it, \
             which is what the validation workspace's cross-module `bazel_dep` wiring exists \
             to support (see docs/architecture/build-verification.md).\n\n\
             If it is NOT part of the deliverable — an absolute path into a system location, \
             a checkout that only exists on the machine that ran the conversion, a prebuilt \
             artifact — then the gap is real: this build has an input that cannot be \
             reproduced from what the project ships, and no conversion can be faithful while \
             that is true. Vendoring the file is then the only honest fix.\n\n\
             Either way '{target_name}' is missing whatever those files contribute, so it \
             will fail to link if anything references their symbols."
        ),
        expected_output: format!(
            "State which of the two cases above applies for each file. For the first, re-run \
             the conversion with a deliverable root wide enough to contain the files and \
             confirm the escalation is gone — do not hand-patch the generated `BUILD.bazel` \
             to paper over a root that was declared too narrowly. For the second, make each \
             file reachable from '{target_name}' by a relative Bazel label — vendored into \
             this module, or supplied by a `deps` edge on another module — and wire it into \
             the generated `BUILD.bazel`. Either way, do NOT edit the project's own build \
             files (`CMakeLists.txt`, `Makefile.am`, `configure.ac`) to move or inline the \
             files."
        ),
        title,
    }
}

/// Escalates a `configure_file` config header that the translator did
/// reproduce, except for some `#cmakedefine`s whose macro the shared
/// `cc_config` catalog does not cover — so the emitted header would be
/// missing those defines. Unlike `generated_config_header_needs_attention`,
/// the mechanism (probing module, template wiring) is in place; the gap is
/// specific macros with no probe.
///
/// The translator does not guess: a macro is escalated on anything but an
/// exact catalog match, so a project's aliased/prefixed macro
/// (json-c's `JSON_C_HAVE_INTTYPES_H`, an alias of the catalog's
/// `HAVE_INTTYPES_H`) lands here rather than being silently mapped to a
/// lookalike. See docs/architecture/configure-file-and-toolchain-probes.md.
/// Which build system produced the template, for the two facts in this
/// escalation that differ by dialect.
///
/// Passed in rather than guessed, because the item ships to an agent who
/// cannot see the project's build files and has no way to catch us being
/// wrong: xz received an item naming `#cmakedefine`, `@VAR@` and a
/// `CMakeLists.txt`, for an autoconf project whose `config.h.in` is 153
/// `#undef` lines and which has no `CMakeLists.txt` at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConfigDialect {
    /// `#cmakedefine NAME` and `@VAR@`, from a `configure_file()` template.
    CMake,
    /// `#undef NAME`, from an autoconf `config.h.in`.
    Autoconf,
}

impl ConfigDialect {
    /// How an unresolved name manifests in the generated header — the reason
    /// the agent has to care, and the thing the pass criterion is stated in.
    fn unresolved_forms(self) -> &'static str {
        match self {
            Self::CMake => {
                "Each is referenced either as a `#cmakedefine` (which would be silently left \
                 undefined) or as a plain `@VAR@` substitution (which would be left LITERAL in \
                 the output — an `@NAME@` token in the header that breaks every source that \
                 includes it)."
            }
            Self::Autoconf => {
                "Each is referenced as an `#undef NAME` line that `configure` would normally \
                 rewrite into a `#define` (or leave commented out). Left unresolved the name \
                 stays undefined, so every `#ifdef` on it silently takes the wrong branch — a \
                 miscompile rather than a build failure, which is the harder direction."
            }
        }
    }

    /// The project's own build files, which a resolution must never edit.
    fn build_files(self) -> &'static str {
        match self {
            Self::CMake => "`CMakeLists.txt`",
            Self::Autoconf => "`configure.ac` or `Makefile.am`",
        }
    }

    /// What "complete" means, in the vocabulary of this dialect.
    fn completion_criterion(self) -> &'static str {
        match self {
            Self::CMake => "no undefined `#cmakedefine` and no literal `@VAR@` left in the output",
            Self::Autoconf => "no `#undef` left for a name that should have been defined",
        }
    }
}

/// Which build system registered the tests an escalation is about.
///
/// Same reason `ConfigDialect` exists, for the other escalation two
/// frontends share: `ctest_command_not_a_target_needs_attention` is called
/// by BOTH `ctest.rs` and `autotools.rs`, and its text used to open "CMake
/// registered these tests with `add_test()`" and close "do NOT edit the
/// project's CMakeLists.txt" — shipped verbatim into six modules holding
/// only `configure.ac` and `Makefile.am`. An agent in an unpacked workspace
/// was told to look for a file that is not there.
///
/// Named for the REGISTRATION MECHANISM rather than the build system, so a
/// third frontend picks the variant that describes what it read rather than
/// the one named after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestDialect {
    /// CMake's `add_test()`, read from `ctest --show-only=json-v1`.
    AddTest,
    /// automake's `TESTS`, read from make's variable database.
    AutomakeTests,
}

impl TestDialect {
    /// How the project registered these tests — the opening claim, and the
    /// one that was false for every Autotools project.
    fn registration(self) -> &'static str {
        match self {
            Self::AddTest => "CMake registered these tests with `add_test()`",
            Self::AutomakeTests => "automake registered these tests in `TESTS`",
        }
    }

    /// The project's own build files, which a resolution must never edit.
    fn build_files(self) -> &'static str {
        match self {
            Self::AddTest => "`CMakeLists.txt`",
            Self::AutomakeTests => "`Makefile.am` or `configure.ac`",
        }
    }

    /// Where the test's working directory came from, which differs by
    /// mechanism and decides what the agent has to reconstruct.
    fn working_directory(self) -> &'static str {
        match self {
            Self::AddTest => {
                "CTest's `WORKING_DIRECTORY` for these tests often points into the CMake                  BUILD tree, which has no counterpart in the converted module"
            }
            Self::AutomakeTests => {
                "automake runs each test from the directory its `Makefile.am` lives in, and                  the scripts typically expect `$srcdir` to be set"
            }
        }
    }
}

pub fn unmapped_config_macros_needs_attention(
    output: &str,
    template: &str,
    macros: &[String],
    dialect: ConfigDialect,
    // What `configure` resolved each name to ON THE CONVERSION HOST, where
    // it recorded one. EVIDENCE for the agent's decision, never the decision
    // — see the note rendered beside the list. Empty for the CMake frontend,
    // which has no `config.status` to read.
    resolved: &HashMap<String, String>,
) -> NeedsAttention {
    let title = format!("Config header '{output}' references names not in the shared catalog");
    NeedsAttention {
        kind: "unmapped_config_macros",
        subject: output.to_string(),
        gap: format!(
            "The config header '{output}' is generated from the template `{template}` and the \
             translator reproduced it with a `config_header` rule wired to `@cc_config` probes \
             — but these name(s) it references have no probe in the shared catalog and no value \
             the translator could resolve, so the generated header would be wrong:\n\n{}\n\n{} \
             The \
             catalog covers the common autoconf facts (`HAVE_<header>`, `HAVE_<symbol>`, \
             `SIZEOF_<type>`); a name absent from it is one the translator will not guess a \
             probe for — including a project-specific alias of a catalog fact (e.g. a \
             `<PROJECT>_HAVE_FOO` that the project sets from the standard `HAVE_FOO`), which is \
             deliberately NOT matched to the lookalike catalog entry.",
            macros
                .iter()
                .map(|m| match resolved.get(m) {
                    Some(value) => format!("- `{m}` — configure resolved this to `{value}`"),
                    None => format!("- `{m}`"),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            dialect.unresolved_forms()
        ),
        context: format!(
            "Some names above carry what `configure` resolved them to. That is EVIDENCE from \
             the conversion host, not the answer: it says what THIS machine decided, and the \
             question is whether that holds for whoever builds the converted module. A project \
             option (a feature switch the project chose, a version) carries over as a `values` \
             entry; a TOOLCHAIN FACT does not, however project-specific its name looks — \
             `BYTEORDER` resolved to `1234` here and is wrong on a big-endian consumer, and a \
             `<PROJECT>_CPUCORES_SCHED_GETAFFINITY` is Linux-only. The name does not tell you \
             which kind it is, which is exactly why this escalates rather than being resolved \
             for you.\n\n\
             Decide, for each name, what it should resolve to under the CONSUMER's toolchain \
             (not this conversion host's). Each will be one of:\n\n\
             - a common toolchain fact the catalog should simply carry — add it to \
             `cc_config/catalog/BUILD.bazel` (a one-line `check_include_file` / \
             `check_symbol_exists` / `check_type_size`) and keep the translator's \
             `CATALOG_DEFINES` in sync (the `catalog_sync_check` test fails if you \
             update one and not the other); then it maps \
             automatically here and for every later project. NOTE this is the one \
             resolution you cannot carry out from inside this module: `cc_config` \
             is not shipped in it, and is supplied at build time by \
             `--override_module=cc_config=<bazelifier checkout>/cc_config` (see \
             the root `MODULE.bazel`). Take this branch only if you have that \
             checkout; otherwise use one of the two below, which need nothing \
             outside this module;\n\
             - an alias of a fact the catalog already has (the `{output}` name differs only by \
             a project prefix from a catalog `HAVE_*`/`SIZEOF_*`): wire the aliased define to \
             that existing `@cc_config//catalog:` probe in the generated `config_header`;\n\
             - a project value, not a toolchain probe — an option, a version, or a name the \
             project computes itself (e.g. an umbrella-include guard that gates whether one \
             header pulls in another): supply it as a `values` entry on the \
             `config_header` (a Bazel config knob or a fixed default), since it does not depend \
             on the consumer's toolchain.\n\n\
             Do NOT copy this host's generated header, and do NOT edit the project's {}.",
            dialect.build_files()
        ),
        expected_output: format!(
            "Resolve every listed name so '{output}' is complete — {}: extend the catalog (and \
             `CATALOG_DEFINES`) for a genuine new fact, point an alias at the existing catalog \
             probe, or add a `values` entry for a project value — in the GENERATED output, or \
             the catalog, only. The header must end up correct for whatever toolchain builds \
             the converted module, not baked from the conversion host.",
            dialect.completion_criterion()
        ),
        title,
    }
}

/// Escalates a header that a target compiles against but which exists only
/// in the CMake build directory — the output of a `configure_file` (or
/// similar build-time generation) rather than a file in the project tree.
///
/// Distinct from `sources_outside_deliverable_needs_attention`: that one is
/// for a source under a *sibling directory* the deliverable root was drawn
/// too narrowly to include, and its resolutions (widen the root, vendor the
/// file) are actively wrong here. Widening can't reach a file that is in no
/// source tree at all, and vendoring means copying *this machine's*
/// generated header — baking in the host's feature-detection results, the
/// opposite of a portable module. So this escalation names the
/// `configure_file` case explicitly and points at the design for it. See
/// docs/lore/cmake-configure-file-generated-headers.md and
/// docs/architecture/configure-file-and-toolchain-probes.md.
pub fn generated_config_header_needs_attention(
    target_name: &str,
    build_dir_outputs: &[String],
) -> NeedsAttention {
    let title = format!("Target '{target_name}' compiles against build-generated headers");
    NeedsAttention {
        kind: "generated_config_header",
        subject: target_name.to_string(),
        gap: format!(
            "Target '{target_name}' compiles against {} header(s) that exist only in the CMake \
             build directory, not in the project's source tree:\n\n{}\n\nThese are the output of \
             `configure_file` (or similar build-time generation) — CMake substitutes \
             feature-detection results and `#cmakedefine`/`@VAR@` values into a template (a \
             `.in`/`.cmakein` file that IS in the source tree) to produce them. The File API \
             does not flag them as generated sources, so they reached this point as absolute \
             build-directory paths and were left out of the generated rule.",
            build_dir_outputs.len(),
            build_dir_outputs
                .iter()
                .map(|p| format!("- `{p}`"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        context: format!(
            "Do NOT resolve this the way an outside-the-deliverable source is resolved. Widening \
             the deliverable root cannot help: the header is in no source tree under any root. \
             And vendoring the generated header as-is copies THIS build machine's \
             feature-detection results (`HAVE_*`, `SIZEOF_*`, ...) into the module, which then \
             builds correctly only on a host like the one that converted it — the opposite of \
             the portable, hermetic module the pipeline is supposed to produce.\n\n\
             The values in these headers are facts about the target platform and toolchain, so \
             they must be computed against whatever toolchain the CONSUMER of this module \
             builds with, not captured from the conversion host. That capability EXISTS and \
             this module already depends on it: `cc_config` is a Bazel-native probing module \
             reproducing CMake's `check_include_file` / `check_symbol_exists` / \
             `check_type_size` as rules that resolve the consumer's toolchain, and its \
             `config_header` rule expands a template against them. See this module's \
             `MODULE.bazel` for the `bazel_dep`, and `resolutions/` for a worked recipe.\n\n\
             Either way '{target_name}' will not compile until the header it includes is \
             available."
        ),
        expected_output: format!(
            "Produce the config header(s) for '{target_name}' in a way that stays correct for \
             the consumer's toolchain — not by copying the conversion host's generated file. \
             That means a `@cc_config//cc_config:config_header.bzl` `config_header` rule in the \
             generated `BUILD.bazel`: it expands the project's template against catalog probes \
             resolved by the build's own toolchain. Wire its output into '{target_name}'s \
             `srcs`. `resolutions/generated-config-header.md` in this module carries a worked \
             example, including what to do when the template itself is not in the source tree. \
             Do NOT edit the project's CMakeLists.txt, and do NOT vendor the host-generated \
             header."
        ),
        title,
    }
}

/// Escalates sources CMake produces during the build rather than reading
/// from the project tree. Kept out of the generated `srcs` — see the
/// `is_generated` handling in `to_target`.
pub fn generated_sources_needs_attention(
    target_name: &str,
    generated: &[String],
) -> NeedsAttention {
    let title = format!("Target '{target_name}' consumes generated sources");
    NeedsAttention {
        kind: "generated_sources",
        subject: target_name.to_string(),
        gap: format!(
            "CMake reports {} source(s) for target '{target_name}' that it generates during \
             the build rather than reading from the project tree:\n\n{}\n\nThey were left out \
             of the generated `cc_*` rule's `srcs`. The File API reports them as absolute \
             paths into the CMake build directory and does not say what produces them, so the \
             translator has nothing it could point `srcs` at.",
            generated.len(),
            generated
                .iter()
                .map(|p| format!("- `{p}`"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        context: format!(
            "This is a translator capability gap, not a problem with the project. A generated \
             file is a perfectly legitimate build input: it is reproducible, because the \
             recipe that produces it ships with the sources. What the translator cannot yet do \
             is translate that recipe, so it has nothing to point `srcs` at. Nothing here \
             needs to be removed or worked around — the recipe needs expressing in \
             Bazel.\n\n\
             Two common causes: an `add_custom_command()` that produces a source, which maps \
             to a `genrule` whose output feeds this target's `srcs`; or a \
             `$<TARGET_OBJECTS:...>` expansion from an `OBJECT_LIBRARY`, in which case the \
             paths above are `.o` files and the real fix is translating that library — look \
             for a separate needs_attention item naming it, and resolving that one likely \
             resolves this too.\n\n\
             '{target_name}' is missing whatever these files contribute. Note the build may \
             still LINK if nothing references the missing symbols, so a green build does not \
             by itself mean this was handled."
        ),
        expected_output: format!(
            "Identify what produces each file above and express it in the generated \
             `BUILD.bazel` — typically a `genrule` (or a `cc_library` replacing the \
             `OBJECT_LIBRARY`) whose output is wired into '{target_name}'. Resolve this in \
             the GENERATED output only — do NOT edit the project's CMakeLists.txt."
        ),
        title,
    }
}

/// Type-specific guidance for an untranslatable target. A generic "this
/// type isn't supported" tells an agent nothing it couldn't read off the
/// title; what's actually useful is the shape of the Bazel answer for that
/// particular CMake construct.
fn unsupported_type_guidance(cmake_type: &str) -> &'static str {
    match cmake_type {
        "UTILITY" => {
            "`UTILITY` is what `add_custom_target()` produces — a named build step, not a \
             compiled artifact. Decide first whether it affects the converted output at all: \
             many utility targets are developer conveniences (docs, formatting, linting) with \
             no place in a Bazel build, in which case the correct resolution is to confirm \
             that and emit nothing. If it does produce a file something else consumes, it \
             maps to a `genrule` (or a custom rule) declaring that file as an output."
        }
        "OBJECT_LIBRARY" => {
            "`OBJECT_LIBRARY` has no direct Bazel equivalent: it exists in CMake to compile a \
             set of sources once and splice the resulting objects into several targets, \
             which is a job Bazel's `cc_library` already does. The usual resolution is a \
             plain `cc_library` with the same `srcs`, depended on normally — Bazel decides \
             object reuse and static/dynamic linking itself."
        }
        "MODULE_LIBRARY" => {
            "`MODULE_LIBRARY` is a plugin loaded at runtime via `dlopen()`, never linked \
             against. The closest Bazel equivalent is `cc_binary(linkshared = True)` with the \
             expected filename, rather than a `cc_library`."
        }
        "INTERFACE_LIBRARY" => {
            "`INTERFACE_LIBRARY` carries no compiled sources — only usage requirements \
             (include dirs, defines, link flags) for its consumers. It maps to a `cc_library` \
             with `hdrs`/`includes` and no `srcs`."
        }
        _ => {
            "This CMake target type has no mapping in the translator yet. Determine what the \
             target contributes to the build and express that with the closest native Bazel \
             rule."
        }
    }
}

/// Escalates a target whose CMake type the translator has no Bazel rule for.
/// The conversion continues without it — see where `read_codemodel_reply`
/// handles `target_kind` returning `None`, and
/// docs/architecture/cmake-frontend.md.
pub fn unsupported_target_needs_attention(
    target_name: &str,
    cmake_type: &str,
    dependents: &[String],
) -> NeedsAttention {
    let title = format!("Target '{target_name}' has unsupported CMake type '{cmake_type}'");

    let dependents_context = if dependents.is_empty() {
        "No other target in this project depends on it, so no dependency edges were lost."
            .to_string()
    } else {
        format!(
            "These targets declared a dependency on '{target_name}': {}. That edge was \
             DROPPED from their generated `deps` — keeping it would emit a label pointing at \
             a target that was never generated, which fails at Bazel analysis time with an \
             error far removed from this cause. If '{target_name}' turns out to contribute \
             symbols or generated files, those targets are incomplete until the edge is \
             restored alongside whatever rule replaces it.",
            dependents.join(", ")
        )
    };

    NeedsAttention {
        kind: "unsupported_target",
        subject: target_name.to_string(),
        gap: format!(
            "Target '{target_name}' has CMake type '{cmake_type}', which the translator has \
             no Bazel rule for — only `EXECUTABLE`, `STATIC_LIBRARY`, and `SHARED_LIBRARY` \
             are mapped today. No rule was generated for it. The rest of the project WAS \
             converted: an unrecognized target is escalated here rather than failing the \
             whole conversion, so the remaining targets are still usable and this gap stays \
             scoped to the one construct that caused it."
        ),
        context: format!(
            "{}\n\n{dependents_context}",
            unsupported_type_guidance(cmake_type)
        ),
        expected_output: format!(
            "Decide what '{target_name}' should become in Bazel and add it to the generated \
             `BUILD.bazel` — including restoring any dependency edge listed above, if the \
             replacement rule warrants one. If the correct answer is that it has no Bazel \
             equivalent, say so explicitly in the resolution rather than silently dropping \
             it; a deliberate omission and an overlooked one are indistinguishable in the \
             output otherwise. Resolve this in the GENERATED output only — do NOT edit the \
             project's CMakeLists.txt."
        ),
        title,
    }
}

/// Escalates a *group* of the project's own inert convenience targets
/// (UTILITY-shaped: no build artifact, nothing depends on them) in ONE item
/// rather than one apiece. See the `inert_convenience` collection in
/// `read_codemodel_reply`, and
/// docs/lore/cmake-include-ctest-injects-utility-targets.md.
///
/// This is the fallback for targets a CMake *module* did not inject (those
/// are dropped silently by provenance): a project's hand-written `docs` /
/// `format` / `lint` targets, which usually have no Bazel equivalent but
/// occasionally do (a codegen step something consumes). Aggregating keeps a
/// project with a dozen of them from burying the substantive gaps, while
/// still surfacing them once so the drop is a decision, not an oversight.
pub fn inert_convenience_targets_needs_attention(target_names: &[String]) -> NeedsAttention {
    let title = format!(
        "{} project convenience target(s) with no artifact and no dependents",
        target_names.len()
    );
    NeedsAttention {
        kind: "inert_convenience_targets",
        subject: target_names.join(", "),
        gap: format!(
            "These targets are defined by the project but produce no build artifact and have \
             no other target depending on them: {}. Each is a CMake `add_custom_target()` \
             (type `UTILITY`) — a named build step, not a compiled artifact. The translator \
             has no Bazel rule for `UTILITY` and, because these are inert (nothing to build, \
             nothing that consumes them), it grouped them here instead of emitting a separate \
             item for each. They are NOT the CMake-provided dashboard targets from \
             `include(CTest)` or similar modules — those are recognized by provenance and \
             dropped without an item; everything named here was written in the project's own \
             CMake files.",
            target_names.join(", ")
        ),
        context: format!(
            "Convenience targets like these are usually developer tooling — generating docs, \
             running a formatter or linter — with no place in a Bazel build, in which case the \
             correct resolution is to confirm that and emit nothing for them. The reason they \
             are surfaced at all, rather than dropped silently, is that a UTILITY target CAN \
             produce a file another target consumes (a generated header, say); the translator \
             cannot tell a pure convenience step from a load-bearing one that merely happens to \
             have no *declared* artifact. Check each of ({}) against what it actually runs. \
             (Any UTILITY target that already has a dependent or a declared artifact was NOT \
             grouped here — it gets its own escalation — so everything in this list looked \
             inert.)",
            target_names.join(", ")
        ),
        expected_output: "For each of these targets, decide whether it has a Bazel equivalent. \
             Most will not: confirm that in the resolution and emit nothing for them — but say so \
             explicitly, because a deliberate omission and an overlooked one are \
             indistinguishable in the output otherwise. For any that DOES produce a file the \
             build consumes, add a `genrule` (or custom rule) to the generated `BUILD.bazel` \
             declaring that output and wire its consumers to it. Resolve this in the GENERATED \
             output only — do NOT edit the project's CMakeLists.txt."
            .to_string(),
        title,
    }
}

/// Escalates CTest tests whose command is not a target the module builds.
///
/// Grouped into one item rather than one per test: a project that drives its
/// suite this way does it for every test (json-c: all 28), and 28 items
/// repeating one paragraph is worse to triage than one naming 28 tests.
///
/// `commands` is parallel to `test_names` — the command as CTest reported it,
/// which is the evidence an agent needs and cannot recover from the test name.
pub fn ctest_command_not_a_target_needs_attention(
    test_names: &[String],
    commands: &[String],
    // Which of `commands` the module actually CARRIES, decided by the caller
    // against the module tree rather than asserted here. The item used to
    // tell every agent "those scripts ship with the sources"; measured
    // across the corpus that was false for 178 of 181 commands, and the
    // instruction that depends on it — point `srcs` at the project's own
    // script — is unfollowable when the file is absent. Which case a command
    // is in changes the resolution, so it is stated per command.
    staged: &[bool],
    dialect: TestDialect,
) -> NeedsAttention {
    let title = format!(
        "{} registered test(s) run a command the translator did not build",
        test_names.len()
    );
    let listing = test_names
        .iter()
        .zip(commands)
        .zip(staged)
        .map(|((name, command), present)| {
            if *present {
                format!("- `{name}` runs `{command}` — present in this module")
            } else {
                format!("- `{name}` runs `{command}` — NOT in this module")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    NeedsAttention {
        // Still `ctest_`-prefixed although the item now speaks both
        // dialects, and deliberately: `kind` is the ONE stable key a tool
        // may group by (needs-attention-interface.md), and `tools/sweep`
        // keys `escalations_by_kind` on it. Renaming would silently split
        // every historical row in metrics/history.jsonl into two series
        // that look like a regression. The prose an agent reads is what had
        // to stop lying; the machine key is a name, not a claim.
        kind: "ctest_command_not_a_target",
        subject: test_names.join(", "),
        gap: format!(
            "{registration}, but each one's command is not \
             an executable this module builds:\n\n{listing}\n\nThe translator emits an \
             `sh_test` for a registered test by wrapping the `cc_binary` the test runs — \
             that is the only shape it can express, because it knows how to point a label \
             at a target it emitted itself. A command that is anything else (a checked-in \
             shell script, an interpreter invoked on a script, a system tool) has no such \
             target to wrap, so NO `sh_test` was generated for the tests above and they are \
             simply absent from the converted module.\n\nThe most common shape by far is a \
             shell script that lives in the project's SOURCE tree and drives a built binary \
             indirectly — comparing its output against a checked-in `.expected` file, \
             looping over fixtures, or setting environment the binary needs.",
            registration = dialect.registration()
        ),
        context: format!(
            "This is a translator capability gap, not a problem with the project. Driving a \
             test suite through scripts is ordinary practice, and what the translator cannot \
             yet do is express 'run this script' as a Bazel test.\n\nEach command above is \
             marked with whether this module CARRIES it, because that changes what you can \
             do and the two cases look identical in the list:\n\n\
             - **present in this module** — the script is checked into the project's source \
               tree and was copied in, so an `sh_test` can point `srcs` at it directly.\n\
             - **NOT in this module** — the file is not here. Usually it is GENERATED by the \
               project's own build (automake rewrites a `.test` wrapper from a `.in`, or \
               configure substitutes it), so there is no checked-in file to copy and the \
               translator did not invent one. Do NOT write an `sh_test` whose `srcs` names \
               it; that fails at analysis with 'missing input file'. Either reproduce what \
               the wrapper does around the binary this module DOES build, or record that \
               this test is not reproduced.\n\nThree things are \
             worth checking before writing the replacement, because each is a way the \
             obvious translation goes wrong:\n\n\
             - **What the script actually runs.** It usually invokes a binary this module DOES \
               build, so the Bazel test should depend on that `cc_binary` target rather than \
               on whatever path the script hardcodes (typically a build-directory path that \
               does not exist here).\n\
             - **Its working directory.** {working_directory} — so it could not be rebased \
               and is not carried in the generated output. A script that locates its data \
               relative to `$0`, `$srcdir`, or the current directory will not find it \
               unless the test sets that up explicitly.\n\
             - **Files it reads.** Anything the script consumes — `.expected` files, JSON \
               fixtures, a sourced helper like `test-defs.sh` — has to be listed in the test's \
               `data`, or it will not be present when the test runs.\n\n\
             A test that is absent is invisible: unlike a build failure, nothing reports it. \
             Whatever these tests were checking is currently unchecked in the converted \
             module.\n\nResolve this in the GENERATED output only. Do NOT edit the \
             project's {build_files}.",
            working_directory = dialect.working_directory(),
            build_files = dialect.build_files()
        )
        .to_string(),
        expected_output: format!(
            "For each test listed above, add a test to the generated `BUILD.bazel` that \
             reproduces what it checked — typically an `sh_test` whose `srcs` is the project's \
             own script, with the `cc_binary` it exercises and every data file it reads in \
             `data`. Where a script only wraps a binary whose exit code is the real pass \
             criterion, a direct test of that binary is an equally good resolution: reproduce \
             the CHECK, not necessarily the script.\n\nIf a test genuinely has no Bazel \
             equivalent, say so explicitly in the resolution rather than leaving it out \
             silently — a deliberate omission and an overlooked one are indistinguishable in \
             the output otherwise. Resolve this in the GENERATED output only — do NOT edit the \
             project's {build_files} or its test scripts.",
            build_files = dialect.build_files()
        ),
        title,
    }
}

/// Escalates a library whose headers CMake never declared public, so the
/// translator has no basis for populating `hdrs` — see the
/// `has_unclassified_headers` check in `to_target`, and
/// docs/architecture/cmake-frontend.md on why the split can't be guessed.
///
/// Unlike the other escalations here, this one flags a conversion that
/// almost certainly still builds and runs correctly: Bazel does not enforce
/// the `hdrs`/`srcs` split, so consumers can include the header either way.
/// The item exists because the gap is invisible in a green build, which is
/// the case most needing explicit triage rather than least.
/// Escalates a shared library that absorbs a static one from the same
/// module — automake's `noinst_LTLIBRARIES` convenience archive, `LIBADD`ed
/// into an installed `lib_LTLIBRARIES`.
///
/// The translator has the FACTS and not the JUDGEMENT, which is why this
/// escalates rather than picking an attribute. It knows the archive is not
/// installed, that the shared library links it, and that Bazel rejects the
/// combination. What it cannot know is whether the archive is an
/// implementation detail to fold in or an interface to export — and the
/// relationship is subtler than any single Bazel attribute expresses, since
/// the linker takes only the members actually referenced.
pub fn shared_library_absorbs_static_needs_attention(
    shared: &str,
    absorbed: &[String],
) -> NeedsAttention {
    let title = format!(
        "Shared library '{shared}' absorbs {} static librar(y/ies)",
        absorbed.len()
    );
    NeedsAttention {
        kind: "shared_library_absorbs_static",
        subject: shared.to_string(),
        gap: format!(
            "The shared library '{shared}' links these static libraries from this same \
             module:\n\n{}\n\nBazel does not accept that as written. A `cc_library` \
             reached through a `cc_shared_library` is linked INTO it, and Bazel refuses \
             to do so unless told explicitly how, failing at ANALYSIS with one of:\n\n\
             - \"Two shared libraries in dependencies link the same library statically\"\n\
             - \"The following libraries were linked statically by different \
             cc_shared_libraries but not exported\"\n\n\
             That error names generated rules rather than anything in the project, which \
             is why it is escalated here instead of left for you to decode.\n\n\
             The project's own build expresses this with automake's \
             `noinst_LTLIBRARIES` — a CONVENIENCE ARCHIVE, built but never installed, \
             whose object files are meant to be pulled into whatever links it. libtool \
             produces no shared object for one at all.",
            absorbed
                .iter()
                .map(|a| format!("- `{a}`"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        context: format!(
            "Three resolutions, and which is right depends on what the archive IS to \
             this project. The translator cannot tell them apart, which is why you are \
             reading this.\n\n\
             - **Fold it in.** If the archive exists only to organise the shared \
             library's own sources — the usual case for a `noinst_` convenience \
             library — move its `srcs` and `hdrs` into '{shared}' directly and delete \
             the separate rule. Simplest, and correct whenever nothing else links the \
             archive.\n\
             - **Name it in the `cc_shared_library`'s own `deps`,** alongside the \
             library itself. The archive is then absorbed into this shared library \
             and its symbols stay LOCAL to it, which is what a `noinst_` archive is \
             for. Right when several targets link the archive and you want one \
             definition of its sources. Note this is a WHOLE-LIBRARY claim.\n\
             - **Add it to `exports_filter`.** Genuinely different: this asserts the \
             shared library EXPORTS those symbols, which is only true if the sources \
             carry visibility attributes or the link uses a version script. For a \
             `noinst_` archive they usually are not exported, so prefer `deps` unless \
             you can point at the thing that makes them visible.\n\n\
             (An older Bazel spelled the SECOND option `static_deps`. That attribute \
             is now a hard error — \"a no-op and its usage is forbidden after \
             cc_shared_library is no longer experimental\" — and `roots` was renamed \
             to `deps`. If you find either in older material, the current spelling is \
             `deps`.)\n\n\
             One thing to know before choosing, because it makes `deps` an \
             over-statement: the absorption is SELECTIVE. A static archive contributes \
             only the members something actually references, so the shared object ends \
             up with some of the archive's objects and not others. If you check the \
             original build you will typically find only the referenced ones present. \
             `deps` says the whole archive is absorbed. The link works either way and \
             the result is slightly larger than the project's own; say so in your \
             resolution rather than leaving the difference unrecorded.\n\n\
             The relationship can also be TRANSITIVE — the archive may be reached \
             through another library rather than named directly by '{shared}' — so \
             check which target actually references its symbols before folding \
             anything in.\n\n\
             Resolve this in the GENERATED output only. Do NOT edit the project's \
             `Makefile.am` or `configure.ac`."
        ),
        expected_output: format!(
            "'{shared}' and the librar(y/ies) it absorbs build without a Bazel analysis \
             error, and the shared object still provides the symbols consumers expect. \
             If you folded an archive in, its separate rule is gone rather than left \
             orphaned; if you named it in `deps`, every target that links the archive \
             agrees. A resolution that merely deletes the archive's rule and drops its \
             sources is wrong — the symbols would go missing at link, far from here."
        ),
        title,
    }
}

pub fn header_visibility_needs_attention(target_name: &str) -> NeedsAttention {
    let title = format!("Library '{target_name}' has headers with no public declaration");
    NeedsAttention {
        kind: "header_visibility",
        subject: target_name.to_string(),
        gap: format!(
            "Target '{target_name}' is a library with at least one other target depending \
             on it, and has header-like files among its sources, but none of them are \
             declared public by either signal the translator trusts: a `target_sources` \
             `FILE_SET` (`target_sources({target_name} PUBLIC FILE_SET ... TYPE HEADERS ...)`) \
             or an `install(FILES ... TYPE INCLUDE)` / target `INCLUDES DESTINATION` rule. \
             Absent both, the CMake File API does not report which plain-source headers are \
             meant for consumers vs. internal-only use, so the translator cannot confidently \
             populate `hdrs` for this target's generated `cc_library` — see \
             docs/architecture/cmake-frontend.md."
        ),
        context: format!(
            "'{target_name}' has at least one dependent target, meaning some other target \
             likely needs to #include one or more of this library's headers. Its generated \
             `cc_library` currently has an empty `hdrs` (all header-like sources were placed \
             in `srcs`). Note this conversion very likely still BUILDS: Bazel does not enforce \
             the hdrs/srcs split by default — a header listed in a dependency's `srcs` is \
             still propagated as an input to dependents' compile actions, so consumers can \
             #include it regardless. (`includes` only supplies the -I search path that \
             determines how the #include is spelled; it is not what exposes the file.) This \
             matches CMake's own looser semantics, where a consumer can #include any header \
             in an include directory whether or not it's the library's 'real' public \
             interface. So the gap here is weaker encapsulation and an unclear public/private \
             boundary, not necessarily a build failure — which is exactly why it needs \
             explicit triage rather than being inferred from a green build. See \
             docs/architecture/build-verification.md's 'Header visibility is not enforced by \
             default'."
        ),
        expected_output: format!(
            "Determine which of '{target_name}''s header files are actually part of its \
             public interface (consumed by dependents via #include) and move those from \
             `srcs` to `hdrs` in the generated `BUILD.bazel`. Resolve this in the GENERATED \
             output only — do NOT edit the project's CMakeLists.txt. The source build files \
             are the input being translated: adding a `FILE_SET` upstream would make this \
             particular project convert cleanly while leaving the translator just as unable \
             to handle the next project that has the same shape."
        ),
        title,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // bzl-fxa.23: this text told the agent the cc_config probing capability
    // "is not built yet" long after it shipped, steering them toward
    // hand-rolling or vendoring — the very thing the same item forbids two
    // paragraphs earlier. A capability landing must not leave a shipped
    // escalation claiming it is missing.
    #[test]
    fn generated_config_header_escalation_points_at_cc_config_not_at_a_future_capability() {
        let item = generated_config_header_needs_attention("mylib", &["/build/config.h".into()]);
        let text = format!("{}\n{}", item.context, item.expected_output);

        assert!(
            text.contains("cc_config"),
            "the escalation must name the mechanism that resolves it:\n{text}"
        );
        assert!(
            text.contains("config_header"),
            "and the rule to use:\n{text}"
        );
        for stale in ["is not built yet", "Until the probing capability"] {
            assert!(
                !text.contains(stale),
                "escalation still claims the probing capability is missing ({stale:?}); \
                 cc_config exists and this module already depends on it:\n{text}"
            );
        }
        assert!(
            text.contains("do NOT vendor") || text.contains("Do NOT vendor"),
            "the do-not-vendor guidance is still load-bearing and must survive:\n{text}"
        );
    }
    #[test]
    fn unsupported_target_escalation_names_type_and_target() {
        let item = unsupported_target_needs_attention("gen_docs", "UTILITY", &[]);

        assert!(item.title.contains("gen_docs"), "{}", item.title);
        assert!(item.title.contains("UTILITY"), "{}", item.title);
        // Type-specific guidance, not a generic "unsupported" message.
        assert!(
            item.context.contains("add_custom_target"),
            "no UTILITY-specific guidance:\n{}",
            item.context
        );
        assert!(
            item.context.contains("No other target"),
            "an unreferenced target should say so explicitly:\n{}",
            item.context
        );
        assert!(
            item.expected_output.contains("do NOT edit"),
            "{}",
            item.expected_output
        );
    }

    #[test]
    fn unsupported_target_escalation_records_dropped_dependency_edges() {
        let item = unsupported_target_needs_attention(
            "obj",
            "OBJECT_LIBRARY",
            &["app".to_string(), "app2".to_string()],
        );

        // The agent has to know which targets were left incomplete.
        assert!(item.context.contains("app, app2"), "{}", item.context);
        assert!(item.context.contains("DROPPED"), "{}", item.context);
        assert!(item.context.contains("cc_library"), "{}", item.context);
    }

    #[test]
    fn unsupported_type_guidance_falls_back_for_unknown_types() {
        let guidance = unsupported_type_guidance("SOMETHING_NEW");
        assert!(
            guidance.contains("no mapping in the translator yet"),
            "an unknown type falls back to generic guidance:\n{guidance}"
        );
    }

    // The escalation has to name the knob that resolves the common case.
    // Its guidance long outlived the limitation it described: it told the
    // agent module roots were "not yet derived from the referenced file
    // set" and that vendoring was the only fix, for several commits after
    // derived module roots and --deliverable-root landed. Nothing caught it
    // because no test read the text.
    #[test]
    fn sources_outside_deliverable_escalation_points_at_the_deliverable_root() {
        let item = sources_outside_deliverable_needs_attention(
            "app",
            &["/elsewhere/vendor/blob.cpp".to_string()],
        );

        assert!(
            item.context.contains("deliverable-root"),
            "the resolution for a file that ships with the project is to widen the \
             deliverable root; the escalation must say so:\n{}",
            item.context
        );
        assert!(
            !item.context.contains("not yet derived"),
            "describes a limitation the translator no longer has:\n{}",
            item.context
        );
    }

    // A configure_file-generated header must NOT get the sources-outside
    // guidance (widen the root / vendor the file) — both are wrong for a
    // header that exists in no source tree, and vendoring the host's copy
    // bakes in this machine's feature detection. The escalation has to name
    // the configure_file case and steer AWAY from vendoring. This is the
    // misdiagnosis bzl-fxa.3 fixes, pinned so it can't silently regress.
    #[test]
    fn generated_config_header_escalation_names_configure_file_and_forbids_vendoring() {
        let item = generated_config_header_needs_attention(
            "json-c",
            &["/abs/_build/json_config.h".to_string()],
        );

        assert!(
            item.gap.contains("configure_file"),
            "the escalation must name the configure_file case:\n{}",
            item.gap
        );
        assert!(
            item.gap.contains("json_config.h"),
            "the escalation must name the header it dropped:\n{}",
            item.gap
        );
        // The whole point: it must steer away from the two resolutions the
        // sources-outside escalation offers, which are wrong here.
        assert!(
            item.context.contains("toolchain") && item.context.contains("consumer"),
            "the escalation must explain the config is toolchain/consumer-specific, not \
             host-capturable:\n{}",
            item.context
        );
        assert!(
            item.expected_output
                .to_lowercase()
                .contains("do not vendor")
                || item.expected_output.contains("do NOT vendor"),
            "the escalation must explicitly forbid vendoring the host-generated header:\n{}",
            item.expected_output
        );
    }

    // The unmapped-macro escalation is distinct from the one above: the header
    // IS reproduced, the gap is specific macros with no catalog probe. Its
    // guidance must name the macros and the three real resolutions (extend the
    // catalog, wire an alias to an existing probe, or supply a value) — and
    // must not tell the agent to bake in the host's config.
    #[test]
    fn unmapped_config_macros_escalation_names_macros_and_the_resolutions() {
        let item = unmapped_config_macros_needs_attention(
            "json_config.h",
            "cmake/json_config.h.in",
            &["JSON_C_HAVE_INTTYPES_H".to_string()],
            ConfigDialect::CMake,
            &HashMap::new(),
        );

        assert!(
            item.gap.contains("JSON_C_HAVE_INTTYPES_H"),
            "the escalation must name the unmapped macro:\n{}",
            item.gap
        );
        assert!(
            item.gap.contains("json_config.h") && item.gap.contains("cmake/json_config.h.in"),
            "the escalation must name the header and its template:\n{}",
            item.gap
        );
        // The alias case (JSON_C_HAVE_* deliberately not auto-mapped) must be
        // called out so the agent knows what it's looking at.
        assert!(
            item.gap.contains("alias"),
            "the escalation must explain the not-guessing-an-alias behavior:\n{}",
            item.gap
        );
        // The three resolution paths: extend the catalog, wire an alias, or a
        // value. Check the two that are load-bearing guidance.
        assert!(
            item.context.contains("catalog") && item.context.contains("catalog_sync_check"),
            "the escalation must offer extending the catalog as a resolution AND name the \
             check that enforces keeping CATALOG_DEFINES in step — an agent told to edit one \
             file will not discover the other on its own:\n{}",
            item.context
        );
        assert!(
            item.context.contains("consumer") && item.context.contains("Do NOT copy this host"),
            "the escalation must keep the resolution consumer-toolchain-correct and forbid \
             host-capture:\n{}",
            item.context
        );
    }

    #[test]
    fn unmapped_config_macros_escalation_calls_out_the_literal_var_build_breaker() {
        // bzl-fxa.9: a plain @VAR@ the translator can't resolve is left LITERAL
        // in the output (an `@NAME@` token the compiler chokes on), unlike an
        // unresolved #cmakedefine which merely goes undefined. The escalation
        // ships to an agent with no access to this repo, so it must say that
        // an unresolved reference can be a literal @VAR@ — otherwise the agent
        // hunts for a missing #define and never looks for the token.
        let item = unmapped_config_macros_needs_attention(
            "json.h",
            "json.h.cmakein",
            &["JSON_H_JSON_PATCH".to_string()],
            ConfigDialect::CMake,
            &HashMap::new(),
        );

        assert!(
            item.gap.contains("LITERAL") && item.gap.contains("`@VAR@`"),
            "the escalation must warn that an unresolved @VAR@ is left literal in the output:\n{}",
            item.gap
        );
        assert!(
            item.expected_output.contains("literal `@VAR@`"),
            "the expected-output must require no literal @VAR@ remains, not just a defined \
             #cmakedefine:\n{}",
            item.expected_output
        );
    }

    // The same escalation ships from both frontends, and three of its facts
    // are dialect-specific: how an unresolved name manifests, which build
    // files must not be edited, and what "complete" means. xz received an
    // item naming `#cmakedefine`, `@VAR@` and a `CMakeLists.txt` — for an
    // autoconf project with none of the three, whose own shipped
    // `resolutions/` recipe simultaneously said `configure.ac`/`Makefile.am`.
    // The NEGATIVE half is the half that failed: the item read plausibly, and
    // only naming a construct the project does not contain gave it away.
    #[test]
    fn the_absorbed_static_escalation_names_both_ends_and_the_bazel_error() {
        let item = shared_library_absorbs_static_needs_attention(
            "libidn2.la",
            &["libgnu.la".to_string(), "libunistring.la".to_string()],
        );

        assert!(
            item.gap.contains("libidn2.la")
                && item.gap.contains("libgnu.la")
                && item.gap.contains("libunistring.la"),
            "the agent has to know WHICH shared library and WHICH archives; \
             the resolution differs per archive:\n{}",
            item.gap
        );
        // The failure the user actually sees is a Bazel analysis error naming
        // rules nobody wrote. An item that does not quote it leaves them
        // unable to connect the two.
        assert!(
            item.gap.contains("linked statically by different")
                || item.gap.contains("link the same"),
            "the item must quote the Bazel error it explains, or the reader \
             cannot tell this item is about the failure in front of \
             them:\n{}",
            item.gap
        );
        // Three genuinely different resolutions, and which is right depends
        // on whether the archive is an implementation detail.
        assert!(
            item.context.to_lowercase().contains("fold it in")
                && item.context.contains("`deps`")
                && item.context.contains("exports_filter"),
            "all three options must be offered, since the translator cannot \
             choose between them — that is why this escalates:\n{}",
            item.context
        );
        // The advice has to be FOLLOWABLE. An earlier version recommended
        // `static_deps`, which rules_cc rejects outright: "a no-op and its
        // usage is forbidden after cc_shared_library is no longer
        // experimental". An agent following the item could not resolve the
        // item. It may still be MENTIONED, to explain the error someone hits
        // reading older material, but never as an option to take.
        //
        // Checked over the WHOLE shipped item, not just `context`. The first
        // version of this assertion split `context` at the deprecation note
        // and looked no further, so the correction that landed in `context`
        // and the recipe left `expected_output` still naming `static_deps`
        // as the success criterion — the item contradicting itself, past the
        // reach of the test written to prevent exactly that.
        let mentions: Vec<&str> = [item.context.as_str(), item.expected_output.as_str()]
            .iter()
            .flat_map(|s| s.split("(An older Bazel"))
            .enumerate()
            // Everything except the deprecation note itself, which is the
            // one place the forbidden spelling belongs.
            .filter(|(i, _)| *i != 1)
            .map(|(_, part)| part)
            .filter(|part| part.contains("static_deps"))
            .collect();
        assert!(
            mentions.is_empty(),
            "`static_deps` is forbidden by rules_cc and must not be offered \
             as a resolution, in the context OR the expected output:\n{:?}",
            mentions
        );
        assert!(
            item.context.to_lowercase().contains("selective"),
            "and the item must say the absorption is SELECTIVE, because a \
             reader who assumes whole-library semantics overstates what \
             `deps` claims:\n{}",
            item.context
        );
    }

    // The test escalation is called by BOTH frontends, and its text used to
    // be CMake's unconditionally: "CMake registered these tests with
    // `add_test()`" and "do NOT edit the project's CMakeLists.txt", shipped
    // into six modules that hold only `configure.ac` and `Makefile.am`. An
    // agent reading it in an unpacked workspace was told to look for a file
    // that does not exist, by an item that also misnamed how the tests were
    // registered.
    //
    // Both directions, because either alone is satisfiable by a constant.
    // The value configure resolved is quoted as EVIDENCE beside each name.
    // Without it the agent has to re-run configure by hand to learn what the
    // translator already read — 151 escalated names across four projects
    // have a definitive value sitting unused (bzl-yjn.10).
    #[test]
    fn a_resolved_value_is_quoted_as_evidence_beside_its_name() {
        let resolved = HashMap::from([
            ("GNULIB_TEST_U8_MBTOUC".to_string(), "1".to_string()),
            ("BYTEORDER".to_string(), "1234".to_string()),
        ]);
        let item = unmapped_config_macros_needs_attention(
            "config.h",
            "config.h.in",
            &[
                "GNULIB_TEST_U8_MBTOUC".to_string(),
                "BYTEORDER".to_string(),
                "SOMETHING_UNRESOLVED".to_string(),
            ],
            ConfigDialect::Autoconf,
            &resolved,
        );

        assert!(
            item.gap
                .contains("`GNULIB_TEST_U8_MBTOUC` — configure resolved this to `1`"),
            "the value belongs beside the name, not in a separate list:\n{}",
            item.gap
        );
        // A name with no recorded value still ships, bare. Otherwise the
        // evidence pass would silently shorten the list it is annotating.
        assert!(
            item.gap.contains("- `SOMETHING_UNRESOLVED`"),
            "a name configure did not record must still be escalated:\n{}",
            item.gap
        );
        // The load-bearing half: the value must NOT read as the answer. It
        // is this host's, and `BYTEORDER 1234` is wrong on a big-endian
        // consumer — the failure this whole escalation exists to prevent.
        assert!(
            item.context.contains("EVIDENCE") && item.context.contains("BYTEORDER"),
            "the item must say what the value is and is not, with the case \
             that shows why:\n{}",
            item.context
        );
    }

    #[test]
    fn a_frontend_with_no_resolved_values_ships_the_bare_list() {
        let item = unmapped_config_macros_needs_attention(
            "config.h",
            "config.h.in",
            &["HAVE_THING".to_string()],
            ConfigDialect::CMake,
            &HashMap::new(),
        );
        assert!(
            item.gap.contains("- `HAVE_THING`") && !item.gap.contains("configure resolved"),
            "the CMake frontend has no config.status, so nothing is quoted:\n{}",
            item.gap
        );
    }

    #[test]
    fn an_automake_test_escalation_speaks_automake_not_ctest() {
        let item = ctest_command_not_a_target_needs_attention(
            &["check_direct".to_string()],
            &["/build/check_direct".to_string()],
            &[false],
            TestDialect::AutomakeTests,
        );
        // The WHOLE item, expected_output included. The morning's
        // static_deps defect was a correction that reached `context` and
        // not `expected_output`, and this constructor had the same shape:
        // its expected_output said "do NOT edit the project's
        // CMakeLists.txt" independently of the gap.
        let whole = format!(
            "{} {} {} {}",
            item.title, item.gap, item.context, item.expected_output
        );
        assert!(
            whole.contains("automake registered these tests in `TESTS`"),
            "the item must name the mechanism the project actually used:\n{whole}"
        );
        assert!(
            whole.contains("`Makefile.am`"),
            "and the build files it must not edit are automake's:\n{whole}"
        );
        assert!(
            !whole.contains("CMake") && !whole.contains("CTest") && !whole.contains("CMakeLists"),
            "an Autotools project has no CMake anything — this item ships to \
             a module with no CMakeLists.txt in it:\n{whole}"
        );
    }

    #[test]
    fn a_ctest_escalation_still_speaks_ctest() {
        let item = ctest_command_not_a_target_needs_attention(
            &["json_parse".to_string()],
            &["tests/parse.test".to_string()],
            &[false],
            TestDialect::AddTest,
        );
        let whole = format!(
            "{} {} {} {}",
            item.title, item.gap, item.context, item.expected_output
        );
        assert!(
            whole.contains("CMake registered these tests with `add_test()`")
                && whole.contains("`CMakeLists.txt`")
                && whole.contains("CTest's `WORKING_DIRECTORY`"),
            "the CMake half must keep its own vocabulary:\n{whole}"
        );
    }

    // The item used to tell EVERY agent "those scripts ship with the
    // sources", which was false for 178 of the corpus's 181 escalated
    // commands — json-c names 28 `.test` wrappers automake generates into
    // the build tree, and 0 of them are in the module. The instruction that
    // depends on it (point `srcs` at the project's own script) fails at
    // analysis with "missing input file", so an agent following the item
    // lands on an error that says nothing about why.
    //
    // Both directions, because a claim that is never false is not a claim.
    #[test]
    fn an_absent_test_command_is_not_described_as_shipping() {
        let item = ctest_command_not_a_target_needs_attention(
            &["test1".to_string(), "test2".to_string()],
            &["tests/test1.test".to_string(), "tests/run.sh".to_string()],
            &[false, true],
            TestDialect::AutomakeTests,
        );
        let whole = format!(
            "{} {} {} {}",
            item.title, item.gap, item.context, item.expected_output
        );

        assert!(
            whole.contains("- `test1` runs `tests/test1.test` — NOT in this module"),
            "an absent command must say so where it is NAMED, not only in \
             the prose below:\n{whole}"
        );
        assert!(
            whole.contains("- `test2` runs `tests/run.sh` — present in this module"),
            "and a checked-in script must still say it IS here, or the \
             marking carries no information:\n{whole}"
        );
        assert!(
            !whole.contains("those scripts ship with the sources"),
            "the unconditional claim is what made the item false:\n{whole}"
        );
    }

    #[test]
    fn an_autoconf_config_header_escalation_speaks_autoconf_not_cmake() {
        let item = unmapped_config_macros_needs_attention(
            "config.h",
            "config.h.in",
            &["HAVE_IMMINTRIN_H".to_string()],
            ConfigDialect::Autoconf,
            &HashMap::new(),
        );
        let whole = format!("{}\n{}\n{}", item.gap, item.context, item.expected_output);

        assert!(
            item.gap.contains("#undef"),
            "an autoconf item must say how an unresolved name manifests in ITS dialect — as an \
             `#undef` configure would have rewritten:\n{}",
            item.gap
        );
        assert!(
            item.context.contains("configure.ac") && item.context.contains("Makefile.am"),
            "an autoconf item must name the build files a resolution must not edit, and this \
             project has no CMakeLists.txt to name:\n{}",
            item.context
        );
        for cmake_only in ["#cmakedefine", "@VAR@", "CMakeLists.txt"] {
            assert!(
                !whole.contains(cmake_only),
                "an autoconf item must not mention `{cmake_only}` — the project contains no \
                 such construct, so the instruction is unfollowable and contradicts the \
                 `resolutions/` recipe shipped beside it:\n{whole}"
            );
        }
    }

    // The other direction, so the dialect switch cannot be wired to a
    // constant: a CMake item must keep saying `#cmakedefine`/`@VAR@`, which
    // the assertions above would otherwise happily accept being deleted.
    #[test]
    fn a_cmake_config_header_escalation_still_speaks_cmake() {
        let item = unmapped_config_macros_needs_attention(
            "config.h",
            "config.h.cmakein",
            &["HAVE_UNISTD_H".to_string()],
            ConfigDialect::CMake,
            &HashMap::new(),
        );
        let whole = format!("{}\n{}\n{}", item.gap, item.context, item.expected_output);

        assert!(
            item.gap.contains("#cmakedefine") && item.gap.contains("`@VAR@`"),
            "a CMake item must still name both CMake unresolved forms:\n{}",
            item.gap
        );
        assert!(
            item.context.contains("CMakeLists.txt"),
            "a CMake item must still name CMakeLists.txt as the file not to edit:\n{}",
            item.context
        );
        assert!(
            !whole.contains("#undef"),
            "a CMake item must not describe autoconf's `#undef` dialect:\n{whole}"
        );
    }

    // The agent cannot act on "a source was dropped" alone: what it needs is
    // the Bazel shape to reach for, and the warning that this gap is one a
    // green build does not clear. Both were unpinned prose until this test.
    #[test]
    fn generated_sources_escalation_names_the_bazel_shape_and_the_silent_failure() {
        let item = generated_sources_needs_attention(
            "app",
            &["/abs/build/CMakeFiles/obj.dir/lib.cpp.o".to_string()],
        );

        assert!(
            item.gap.contains("/abs/build/CMakeFiles/obj.dir/lib.cpp.o"),
            "the escalation must name the file that was dropped:\n{}",
            item.gap
        );
        assert!(
            item.context.contains("genrule"),
            "the resolution shape for a produced source is a genrule; the escalation \
             must say so:\n{}",
            item.context
        );
        assert!(
            item.context.contains("OBJECT_LIBRARY"),
            "the other common cause is an OBJECT_LIBRARY expansion, whose real fix is a \
             different item:\n{}",
            item.context
        );
        assert!(
            item.context.contains("green build"),
            "this gap can link cleanly when nothing references the missing symbols; \
             without that warning an agent reads a passing build as a resolution:\n{}",
            item.context
        );
    }

    // The one escalation whose conversion still builds and runs correctly, so
    // its whole job is explaining why it is worth triaging anyway. If that
    // explanation is ever trimmed to "populate hdrs", the item becomes
    // indistinguishable from a false positive and gets dismissed as one.
    #[test]
    fn header_visibility_escalation_explains_why_a_green_build_proves_nothing() {
        let item = header_visibility_needs_attention("greet");

        assert!(
            item.context.contains("does not enforce"),
            "the non-obvious fact is that Bazel does not enforce the hdrs/srcs split:\n{}",
            item.context
        );
        assert!(
            item.context.contains("not what exposes the file"),
            "propagation comes from the header being in some target's srcs/hdrs, NOT from \
             `includes` — the intuitive model has this backwards, see \
             docs/lore/bazel-does-not-enforce-hdrs-vs-srcs.md:\n{}",
            item.context
        );
        assert!(
            item.expected_output.contains("do NOT edit"),
            "adding a FILE_SET upstream is not a resolution:\n{}",
            item.expected_output
        );
    }

    #[test]
    fn renders_expected_sections() {
        let item = NeedsAttention {
            kind: "header_visibility",
            subject: "greet".to_string(),
            title: "Library 'greet' has no public headers".to_string(),
            gap: "gap text".to_string(),
            context: "context text".to_string(),
            expected_output: "expected text".to_string(),
        };
        let rendered = render(&item);
        // The title now follows the machine-readable header rather than
        // opening the file; `renders_a_machine_readable_header_before_the_prose`
        // pins that ordering.
        assert!(
            rendered.contains("\n# Library 'greet' has no public headers\n"),
            "{rendered}"
        );
        // The section headings are the schema an agent reads, so each is
        // asserted with its body attached — a heading present but empty is
        // the failure worth catching.
        assert!(rendered.contains("## Gap\n\ngap text\n"), "{rendered}");
        assert!(
            rendered.contains("## Context\n\ncontext text\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("## Expected output\n\nexpected text\n"),
            "{rendered}"
        );
    }

    // The header is what a tool keys on, so its exact shape is the contract —
    // and it comes FIRST, before the title, so a parser can read kind and
    // subject without scanning prose.
    #[test]
    fn renders_a_machine_readable_header_before_the_prose() {
        let rendered = render(&NeedsAttention {
            kind: "header_visibility",
            subject: "greet".to_string(),
            title: "Library 'greet' has no public headers".to_string(),
            gap: "gap text".to_string(),
            context: "context text".to_string(),
            expected_output: "expected text".to_string(),
        });
        assert!(
            rendered
                .starts_with("---\nkind: header_visibility\nsubject: \"greet\"\n---\n\n# Library"),
            "the header opens the file and the title follows it:\n{rendered}"
        );
    }

    // A subject carries values from the PROJECT — a path, a CMake type, a
    // joined list — so it cannot be trusted to be one clean line. A newline
    // would close the header early and silently turn the rest of the item into
    // body text a parser reads as prose.
    #[test]
    fn a_subject_cannot_break_out_of_the_header() {
        let rendered = render(&NeedsAttention {
            kind: "unsupported_target",
            subject: "weird\nname: with\r\"quotes\"".to_string(),
            title: "t".to_string(),
            gap: "g".to_string(),
            context: "c".to_string(),
            expected_output: "e".to_string(),
        });
        let header: Vec<&str> = rendered
            .lines()
            .skip(1)
            .take_while(|l| *l != "---")
            .collect();
        assert_eq!(
            header,
            vec![
                "kind: unsupported_target",
                "subject: \"weird name: with 'quotes'\""
            ],
            "the header stays two lines whatever the subject contains:\n{rendered}"
        );
    }

    // The header is ADDITIVE. Everything below it ships to an agent working in
    // an unpacked workspace with no access to this repo, and CLAUDE.md forbids
    // churning that text — so adding a machine key must not reflow, reorder or
    // reword a single byte of it.
    #[test]
    fn the_header_does_not_disturb_the_prose_below_it() {
        let item = header_visibility_needs_attention("greet");
        let rendered = render(&item);
        let (header, prose) = rendered.split_once("---\n\n").expect("header delimiter");
        assert_eq!(
            header.lines().count(),
            3,
            "header is `---`, kind, subject; the closing `---` is the split \
             point:\n{header}"
        );
        assert_eq!(
            prose,
            format!(
                "# {}\n\n## Gap\n\n{}\n\n## Context\n\n{}\n\n## Expected output\n\n{}\n",
                item.title, item.gap, item.context, item.expected_output
            ),
            "the prose must be exactly what it was before the header existed"
        );
    }

    // Every constructor must carry a kind, and no two may share one: the kind
    // is the ONLY stable key a metric can group by (a title is prose and gets
    // reworded, and the NNN-slug filename is derived from the title). A
    // duplicate would silently merge two gap types into one bucket.
    #[test]
    fn every_escalation_kind_is_distinct_and_non_empty() {
        let items = vec![
            sources_outside_deliverable_needs_attention("t", &["a.c".to_string()]),
            unmapped_config_macros_needs_attention(
                "config.h",
                "config.h.in",
                &["X".to_string()],
                ConfigDialect::CMake,
                &HashMap::new(),
            ),
            generated_config_header_needs_attention("t", &["gen.h".to_string()]),
            generated_sources_needs_attention("t", &["gen.c".to_string()]),
            unsupported_target_needs_attention("t", "OBJECT_LIBRARY", &["dep".to_string()]),
            inert_convenience_targets_needs_attention(&["t".to_string()]),
            ctest_command_not_a_target_needs_attention(
                &["x".to_string()],
                &["cmd".to_string()],
                &[false],
                TestDialect::AddTest,
            ),
            header_visibility_needs_attention("t"),
        ];
        let mut kinds: Vec<&str> = items.iter().map(|i| i.kind).collect();
        let total = kinds.len();
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(
            kinds.len(),
            total,
            "two escalations share a kind, so a metric would merge them: {kinds:?}"
        );
        for item in &items {
            assert!(
                !item.kind.is_empty() && !item.subject.is_empty(),
                "every item needs a kind and a subject: {item:#?}"
            );
            assert!(
                item.kind
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_'),
                "a kind is a machine key, so it stays snake_case: {:?}",
                item.kind
            );
        }
    }

    #[test]
    fn slugifies_titles() {
        assert_eq!(
            slugify("Library 'greet' has no public headers"),
            "library-greet-has-no-public-headers"
        );
    }
}
