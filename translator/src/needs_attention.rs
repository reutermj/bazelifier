//! Renders `needs_attention/<NNN>-<slug>.md` files: per-conversion,
//! agent-actionable descriptions of a gap the translator could not
//! confidently resolve for THIS project. See
//! docs/architecture/runbook-interface.md for how this differs from
//! bazelifier's own docs/runbooks/ (which document the translator's
//! general escalation contract, not a specific conversion's follow-ups).

use crate::model::NeedsAttention;

const TEMPLATE: &str = "# {title}

## Gap

{gap}

## Context

{context}

## Expected output

{expected_output}
";

pub fn render(item: &NeedsAttention) -> String {
    TEMPLATE
        .replace("{title}", &item.title)
        .replace("{gap}", &item.gap)
        .replace("{context}", &item.context)
        .replace("{expected_output}", &item.expected_output)
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

#[cfg(test)]
mod tests {
    use super::*;

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
