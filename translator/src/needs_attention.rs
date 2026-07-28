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
    let title = format!("Library '{target_name}' has headers but no public FILE_SET");
    NeedsAttention {
        gap: format!(
            "Target '{target_name}' is a library with at least one other target depending \
             on it, and has header-like files among its sources, but none of them are \
             declared as a public `FILE_SET` (`target_sources({target_name} PUBLIC FILE_SET \
             ... TYPE HEADERS ...)`). The CMake File API does not report which plain-source \
             headers are meant for consumers vs. internal-only use, so the translator cannot \
             confidently populate `hdrs` for this target's generated `cc_library` — see \
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

        assert!(item.title.contains("gen_docs"));
        assert!(item.title.contains("UTILITY"));
        // Type-specific guidance, not a generic "unsupported" message.
        assert!(item.context.contains("add_custom_target"));
        assert!(
            item.context.contains("No other target"),
            "an unreferenced target should say so explicitly:\n{}",
            item.context
        );
        assert!(item.expected_output.contains("do NOT edit"));
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
