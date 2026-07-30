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
        title,
        gap,
        context,
        expected_output,
    } = item;
    format!(
        "# {title}\n\n\
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
        gap: format!(
            "Target '{target_name}' compiles {} source file(s) that the translator could not \
             place inside the generated module:\n\n{}\n\nThey were left out of the generated \
             rule's `srcs`. The module's root is derived, not assumed: it is the deepest \
             directory containing both the CMake project and everything the build references \
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
             `deliverable_root` attribute on `convert_cmake_project`) and defaults to the \
             CMake project directory, which is why a sibling directory falls outside it \
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
             the generated `BUILD.bazel`. Either way, do NOT edit the project's CMakeLists.txt \
             to move or inline the files."
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
pub fn unmapped_config_macros_needs_attention(
    output: &str,
    template: &str,
    macros: &[String],
) -> NeedsAttention {
    let title = format!("Config header '{output}' references names not in the shared catalog");
    NeedsAttention {
        gap: format!(
            "The config header '{output}' is generated from the template `{template}` and the \
             translator reproduced it with a `config_header` rule wired to `@cc_config` probes \
             — but these name(s) it references have no probe in the shared catalog and no value \
             the translator could resolve, so the generated header would be wrong:\n\n{}\n\nEach \
             is referenced either as a `#cmakedefine` (which would be silently left undefined) \
             or as a plain `@VAR@` substitution (which would be left LITERAL in the output — an \
             `@NAME@` token in the header that breaks every source that includes it). The \
             catalog covers the common autoconf facts (`HAVE_<header>`, `HAVE_<symbol>`, \
             `SIZEOF_<type>`); a name absent from it is one the translator will not guess a \
             probe for — including a project-specific alias of a catalog fact (e.g. a \
             `<PROJECT>_HAVE_FOO` that the project sets from the standard `HAVE_FOO`), which is \
             deliberately NOT matched to the lookalike catalog entry.",
            macros
                .iter()
                .map(|m| format!("- `{m}`"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        context: format!(
            "Decide, for each name, what it should resolve to under the CONSUMER's toolchain \
             (not this conversion host's). Each will be one of:\n\n\
             - a common toolchain fact the catalog should simply carry — add it to \
             `cc_config/catalog/BUILD.bazel` (a one-line `check_include_file` / \
             `check_symbol_exists` / `check_type_size`) and keep the translator's \
             `CATALOG_DEFINES` in sync (see `cc_config/check_catalog_sync.py`); then it maps \
             automatically here and for every later project;\n\
             - an alias of a fact the catalog already has (the `{output}` name differs only by \
             a project prefix from a catalog `HAVE_*`/`SIZEOF_*`): wire the aliased define to \
             that existing `@cc_config//catalog:` probe in the generated `config_header`;\n\
             - a project value, not a toolchain probe — an option, a version, or a plain \
             `@VAR@` the project computes itself (e.g. an umbrella-include guard that gates \
             whether one header pulls in another): supply it as a `values` entry on the \
             `config_header` (a Bazel config knob or a fixed default), since it does not depend \
             on the consumer's toolchain.\n\n\
             Do NOT copy this host's generated header, and do NOT edit the project's \
             CMakeLists.txt."
        ),
        expected_output: format!(
            "Resolve every listed name so '{output}' is complete — no undefined `#cmakedefine` \
             and no literal `@VAR@` left in the output: extend the catalog (and \
             `CATALOG_DEFINES`) for a genuine new fact, point an alias at the existing catalog \
             probe, or add a `values` entry for a project value — in the GENERATED output, or \
             the catalog, only. The header must end up correct for whatever toolchain builds \
             the converted module, not baked from the conversion host."
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
             builds with, not captured from the conversion host. The intended design is a \
             shared Bazel-native probing module that reproduces CMake's `check_include_file` / \
             `check_symbol_exists` / `check_type_size` as rules resolving the consumer's \
             toolchain — see docs/architecture/configure-file-and-toolchain-probes.md. That \
             capability is not built yet.\n\n\
             Either way '{target_name}' will not compile until the header it includes is \
             available."
        ),
        expected_output: format!(
            "Produce the config header(s) for '{target_name}' in a way that stays correct for \
             the consumer's toolchain — not by copying the conversion host's generated file. \
             Until the probing capability above exists, that means reproducing the substitution \
             in the generated `BUILD.bazel` (a rule that resolves the needed values against the \
             build's own toolchain and expands the project's `.in`/`.cmakein` template), and \
             wiring the result into '{target_name}'. Do NOT edit the project's CMakeLists.txt, \
             and do NOT vendor the host-generated header."
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
pub fn header_visibility_needs_attention(target_name: &str) -> NeedsAttention {
    let title = format!("Library '{target_name}' has headers with no public declaration");
    NeedsAttention {
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
        assert!(guidance.contains("no mapping in the translator yet"));
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
            item.context.contains("catalog") && item.context.contains("check_catalog_sync"),
            "the escalation must offer extending the catalog (kept in sync) as a resolution:\n{}",
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
            title: "Library 'greet' has no public headers".to_string(),
            gap: "gap text".to_string(),
            context: "context text".to_string(),
            expected_output: "expected text".to_string(),
        };
        let rendered = render(&item);
        assert!(rendered.starts_with("# Library 'greet' has no public headers\n"));
        assert!(rendered.contains("## Gap\n\ngap text\n"));
        assert!(rendered.contains("## Context\n\ncontext text\n"));
        assert!(rendered.contains("## Expected output\n\nexpected text\n"));
    }

    #[test]
    fn slugifies_titles() {
        assert_eq!(
            slugify("Library 'greet' has no public headers"),
            "library-greet-has-no-public-headers"
        );
    }
}
