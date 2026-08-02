//! autoconf's config headers: `AC_CONFIG_HEADERS` recovered from
//! `config.status`, and the plan for reproducing one.
//!
//! Its own module because it reads a THIRD input. The rest of the Autotools
//! frontend interrogates the build — its stdout for the resolved command
//! stream, `make -p` for the variable database — while this parses
//! `config.status`, a script `configure` wrote, and the template it names off
//! disk.
//!
//! That is the same argument that gave `configure_file.rs` its own module on
//! the CMake side, where the config headers appear in no File API reply and
//! have to come from the configure trace. The two are deliberately NOT
//! merged: they consume different inputs, resolve values against different
//! sources, and run at different points — the CMake side after the codemodel
//! walk, because it rebases template paths against a root derived during it,
//! while this runs before the graph is built and takes its template from the
//! source tree unconditionally. What they genuinely share is already shared:
//! `catalog_label`, the `cc_config` expander's dual dialect handling, and the
//! `unmapped_config_macros` escalation.

use std::collections::HashMap;

use crate::configure_file::catalog_label;
use crate::model::ConfigHeader;

/// Whether a substituted value is a bare number, and so must NOT be quoted.
///
/// autoconf's own rule: `AC_DEFINE([SIZEOF_LONG], [8])` emits `8`, while
/// `AC_DEFINE([PACKAGE], ["xz"])` emits `"xz"`. Getting this backwards fails
/// in opposite directions — an unquoted string becomes stray identifiers, a
/// quoted number becomes a type error in `#if`.
fn is_numeric_literal(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

/// Builds the `config_header` plan for an autoconf template, and the macros
/// it references that the shared catalog cannot resolve.
///
/// The catalog is the SAME one the CMake frontend maps `#cmakedefine` names
/// through — a `HAVE_UNISTD_H` probe is the same fact whichever build system
/// asked for it, and reusing the catalog is a large part of why Autotools was
/// the right second frontend rather than an excuse to build a parallel one.
///
/// Package metadata (`PACKAGE`, `VERSION`, ...) is resolved from make's
/// variable database rather than probed: those are values `configure`
/// substituted, not facts about the toolchain, so probing for them would be
/// both wrong and impossible.
pub(crate) fn plan_config_header(
    output: &str,
    template: &str,
    template_text: &str,
    vars: &HashMap<String, String>,
) -> (ConfigHeader, Vec<String>) {
    let mut probes = Vec::new();
    let mut values = Vec::new();
    let mut unmapped = Vec::new();

    for name in undef_names(template_text) {
        if let Some(label) = catalog_label(&name) {
            probes.push(label);
        } else if let Some(value) = vars.get(&name).filter(|v| !v.is_empty()) {
            // A value configure substituted — PACKAGE, VERSION and friends.
            //
            // The emptiness check is load-bearing, not defensive. `configure`
            // seeds make's database with EVERY macro it knows, most of them
            // empty, so a bare `vars.get` matches names it has no value for:
            // hello has `BITSIZEOF_PTRDIFF_T = ` in its database, and taking
            // that as a substitution emitted `"BITSIZEOF_PTRDIFF_T": """"` —
            // an unclosed string literal that would not parse. An empty
            // variable is the absence of a value, so the macro falls through
            // to the escalation below where it belongs.
            //
            // Quoted the way autoconf itself would. `AC_DEFINE([PACKAGE_NAME],
            // ["XZ Utils"])` puts a C STRING LITERAL in config.h — quotes
            // included — while make's variable database carries the bare text.
            // Passing that through raw emitted `#define PACKAGE_NAME XZ Utils`,
            // which the preprocessor splices into `printf("xz (" XZ Utils ")")`
            // and clang rejects as an undeclared identifier, several files from
            // the config header that caused it.
            //
            // A purely numeric value is left bare, matching autoconf: SIZEOF_*
            // and ASSUME_RAM are used in arithmetic and `#if`, where a string
            // literal is a type error rather than a quoting nicety.
            //
            // Note this is C quoting, not Starlark quoting — codegen escapes
            // the result again on the way into the BUILD file.
            let value = if is_numeric_literal(value) {
                value.clone()
            } else {
                format!("\"{value}\"")
            };
            values.push((name, value));
        } else {
            // Neither a catalog fact nor a substituted value. Escalated
            // rather than guessed at, exactly as the CMake frontend refuses
            // to match a project-prefixed alias to a lookalike catalog entry.
            unmapped.push(name);
        }
    }

    probes.sort_unstable();
    probes.dedup();
    values.sort_unstable();
    values.dedup();
    unmapped.sort_unstable();
    unmapped.dedup();

    (
        ConfigHeader {
            output: output.to_string(),
            template: template.to_string(),
            template_source: None,
            catalog_probes: probes,
            values,
        },
        unmapped,
    )
}

/// Every name an autoconf template declares with a bare `#undef`.
///
/// Line-anchored for the same reason the expander is: a mid-line `#undef` is
/// ordinary C undefining a macro, not a declaration to resolve.
fn undef_names(template_text: &str) -> Vec<String> {
    template_text
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('#')?.trim_start().strip_prefix("undef")?;
            let name = rest.trim();
            (!name.is_empty() && !name.contains(char::is_whitespace))
                .then(|| name.to_string())
        })
        .collect()
}

/// The config header a project declares with `AC_CONFIG_HEADERS`, as
/// (output, template) — `("config.h", "config.in")` for GNU hello.
///
/// Read from `config.status` rather than `configure.ac`, for the same reason
/// the rest of this frontend reads resolved output: `configure.ac` is the
/// input, and its macro arguments can be variables. `config.status` is what
/// `configure` actually decided, and it records BOTH sides:
///
/// ```text
/// config_headers=" config.h:config.in"
/// ```
///
/// The template is not always `<output>.in`. hello declares
/// `AC_CONFIG_HEADERS([config.h:config.in])` to keep the filename 8.3-safe,
/// so assuming the name would find nothing. Autoconf defaults the template to
/// `<output>.in` when only one side is given, which is handled here rather
/// than by the caller.
pub(crate) fn parse_config_headers(config_status: &str) -> Vec<(String, String)> {
    let Some(line) = config_status
        .lines()
        .find_map(|l| l.trim().strip_prefix("config_headers="))
    else {
        return Vec::new();
    };
    line.trim_matches(&['"', '\''][..])
        .split_whitespace()
        .map(|spec| match spec.split_once(':') {
            Some((output, template)) => (output.to_string(), template.to_string()),
            // Only the output named: autoconf's default template is
            // `<output>.in`.
            None => (spec.to_string(), format!("{spec}.in")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotools::parse_variables;

    // AC_CONFIG_HEADERS names BOTH sides, and the template is not always
    // `<output>.in` — GNU hello declares config.h:config.in to keep the name
    // 8.3-safe, so assuming would find nothing.
    #[test]
    fn parse_config_headers_reads_both_sides_of_the_declaration() {
        assert_eq!(
            parse_config_headers("config_headers=\" config.h:config.in\"\n"),
            vec![("config.h".to_string(), "config.in".to_string())]
        );
        // Only the output named: autoconf defaults the template to <output>.in.
        assert_eq!(
            parse_config_headers("config_headers=\"config.h\"\n"),
            vec![("config.h".to_string(), "config.h.in".to_string())]
        );
        // A project declaring none — fixture 001 — must yield none rather
        // than a default that does not exist.
        assert!(parse_config_headers("other=1\n").is_empty());
    }

    #[test]
    fn a_substituted_value_is_quoted_unless_it_is_a_number() {
        let vars: HashMap<String, String> = [
            ("PACKAGE_NAME".to_string(), "XZ Utils".to_string()),
            ("ASSUME_RAM".to_string(), "128".to_string()),
        ]
        .into_iter()
        .collect();
        let (header, _) = plan_config_header(
            "config.h",
            "config.in",
            "#undef PACKAGE_NAME\n#undef ASSUME_RAM\n",
            &vars,
        );

        assert_eq!(
            header.values,
            vec![
                ("ASSUME_RAM".to_string(), "128".to_string()),
                ("PACKAGE_NAME".to_string(), "\"XZ Utils\"".to_string()),
            ],
            "autoconf quotes a string value and leaves a numeric one bare; \
             make's database carries both as plain text, so the distinction \
             has to be restored here"
        );
    }

    #[test]
    fn an_empty_make_variable_is_not_a_substituted_value() {
        let vars: HashMap<String, String> = [
            ("PACKAGE".to_string(), "hello".to_string()),
            ("BITSIZEOF_PTRDIFF_T".to_string(), String::new()),
        ]
        .into_iter()
        .collect();
        let (header, unmapped) = plan_config_header(
            "config.h",
            "config.in",
            "#undef PACKAGE\n#undef BITSIZEOF_PTRDIFF_T\n#undef HAVE_UNISTD_H\n",
            &vars,
        );

        assert_eq!(
            header.values,
            vec![("PACKAGE".to_string(), "\"hello\"".to_string())],
            "only a real substitution is a value, and a non-numeric one carries \
             the C quotes autoconf itself would emit: `AC_DEFINE([PACKAGE], \
             [\"hello\"])` puts a string LITERAL in config.h"
        );
        assert_eq!(
            header.catalog_probes,
            vec!["@cc_config//catalog:have_unistd_h"],
            "a catalog fact is probed, not substituted"
        );
        assert_eq!(
            unmapped,
            vec!["BITSIZEOF_PTRDIFF_T"],
            "an empty variable is the ABSENCE of a value, so the macro is \
             escalated rather than emitted as an empty string"
        );
    }
}
