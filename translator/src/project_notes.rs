//! Per-project notes shipped into a converted module's `project_notes/`.
//!
//! Anything project-specific the agent stage needs and cannot derive. What
//! general guidance cannot reach, because it is true of ONE project:
//!
//! - an oddity where the obvious answer is wrong and the input actively
//!   misleads — json-c's `apps/CMakeLists.txt` writes `set(VAR)` under a
//!   comment saying "we know we have this", which reads as an assertion and
//!   unsets the variable;
//! - a convention the project inherited that its build system does not
//!   explain — a CMake project carrying an autotools-era shell harness;
//! - a bug in the project's OWN build, as an `## Upstream` section of the
//!   same note. The maintainer's fix and the agent's instruction are about
//!   one line and are usually opposite (a conversion reproduces what the
//!   build DOES), so keeping them in one file is what stops them drifting.
//!
//! Distinct from an escalation, which says what the translator could not
//! resolve and is generated per conversion. A note is human knowledge about
//! a project, written once and true until the project changes. It is also
//! not a resolution: it supplies the fact and leaves the decision where the
//! escalation put it.
//!
//! Held as `include_str!` of real markdown rather than as string literals so
//! a note reads and reviews as prose. `compile_data` in
//! `translator/BUILD.bazel` declares them to Bazel; nothing is read at
//! runtime, so the binary still works when run directly on a checkout.
//!
//! Keyed by the module name the frontend derived, which is what the
//! generated `MODULE.bazel` already calls the project. A project with no
//! notes ships no directory — an empty one would read as "checked, nothing
//! found" when it means "nobody has looked".

/// One note: its filename inside `project_notes/`, and its content.
pub struct Note {
    pub filename: &'static str,
    pub body: &'static str,
}

const JSON_C_SET_WITH_NO_VALUE: &str =
    include_str!("../project_notes/json-c/001-set-with-no-value-defeats-a-feature-assertion.md");

/// Notes for `module_name`, empty when there are none.
///
/// Matched on the module name rather than the directory the sources came
/// from: the directory is a Bazel staging path (`external/+git_repository+…`)
/// that changes with how the project was fetched, while the module name is
/// what the project calls itself.
pub fn for_project(module_name: &str) -> Vec<Note> {
    match module_name {
        "json-c" => vec![Note {
            filename: "001-set-with-no-value-defeats-a-feature-assertion.md",
            body: JSON_C_SET_WITH_NO_VALUE,
        }],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The note exists to be READ, so the thing worth pinning is that it
    // reaches the project it is about and no other. A note shipped into
    // every module would be the uniformity that `resolutions/` was deleted
    // for.
    #[test]
    fn a_note_reaches_only_the_project_it_describes() {
        let notes = for_project("json-c");
        assert_eq!(notes.len(), 1, "json-c has exactly one note today");
        assert!(
            notes[0]
                .body
                .contains("set(HAVE_JSON_TOKENER_GET_PARSE_END)"),
            "the note must quote the construct it is about, or a reader \
             cannot match it to what they are looking at"
        );
        assert!(
            for_project("zlib").is_empty(),
            "a project with no notes gets none — this is per-project by \
             construction, not a shared directory with a filter"
        );
    }

    // The failure this guards against is silent: `include_str!` of a file
    // that was emptied still compiles, and the module then ships a note
    // saying nothing.
    #[test]
    fn every_note_has_content() {
        for name in ["json-c"] {
            for note in for_project(name) {
                assert!(
                    note.body.len() > 200,
                    "{name}'s {} is too short to be a real note",
                    note.filename
                );
                assert!(
                    note.filename.ends_with(".md"),
                    "notes are markdown: {}",
                    note.filename
                );
            }
        }
    }
}
