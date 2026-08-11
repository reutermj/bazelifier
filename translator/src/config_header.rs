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
use std::path::Path;

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
    flag_macros: &HashMap<String, String>,
) -> (ConfigHeader, Vec<String>) {
    let mut probes = Vec::new();
    let mut values = Vec::new();
    let mut options = Vec::new();
    let mut unmapped = Vec::new();

    for name in undef_names(template_text) {
        if let Some(label) = catalog_label(&name) {
            probes.push(label);
        } else if let Some(flag) = flag_macros.get(&name) {
            // A macro a BOOLEAN build option controls. Resolved rather than
            // escalated because the input states it — `configure` itself
            // shows the macro appearing and vanishing with the flag — and
            // resolved BEFORE the variable-database branch because these
            // names are usually absent from make's database entirely:
            // HAVE_GREETING lives only in config.status's D[] table.
            //
            // The value is `1`, matching what config.status writes for an
            // AC_DEFINE with no explicit value. Codegen turns the pair into
            // a bool_flag plus a select(), so the option stays settable.
            options.push((name.clone(), flag.clone()));
            values.push((name, "1".to_string()));
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
            //
            // `yes`/`no` are configure's SHELL booleans and not values at
            // all — the same shape as the emptiness check above, since this
            // is configure stating a fact rather than supplying text. A
            // feature it did not find is recorded as the word `no`, and
            // defining the macro to the string "no" INVERTS the answer:
            // autoconf leaves the name undefined, while `#if NAME` on a
            // non-empty string is true. Measured on libidn2's generated
            // config.h — zero macros are defined to either word, quoted or
            // bare — so carrying them through as text is never right.
            //
            // The bare word only; a value that merely contains it (a version
            // string, a path) is ordinary data.
            let value = match value.as_str() {
                "no" => continue,
                "yes" => "1".to_string(),
                _ if is_numeric_literal(value) => value.clone(),
                _ => format!("\"{value}\""),
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
            // Macros a boolean build option controls, elicited by
            // differential configure rather than read — autoconf states the
            // mapping nowhere. See `autotools::resolve_flag_macros`.
            options,
            // autoconf assembles a config header by substitution alone.
            splices: Vec::new(),
            // AC_CONFIG_HEADERS: the template declares with `#undef`.
            dialect: crate::model::ConfigDialect::Undef,
            // Not a shadowing header: these come from AC_CONFIG_HEADERS /
            // AC_CONFIG_FILES / configure_file, which generate a project's own
            // header rather than a replacement for a system one.
            shadow_dir: None,
        },
        unmapped,
    )
}

/// Every name an autoconf template declares with an `#undef`.
///
/// Line-anchored: only WHITESPACE may precede the directive. A mid-line
/// `#undef` is ordinary C undefining a macro, not a declaration to resolve.
///
/// Whitespace *is* allowed — before the hash and after it — because autoconf
/// uses both. `AC_USE_SYSTEM_EXTENSIONS` writes `# undef _GNU_SOURCE` inside
/// an `#ifndef`, and expat and libmicrohttpd nest `WORDS_BIGENDIAN` two
/// levels deep as `#  undef`. Requiring the unspaced column-0 form left all
/// of those undefined, which for `_GNU_SOURCE` disables every GNU extension
/// and for `WORDS_BIGENDIAN` picks the wrong branch silently.
///
/// This is deliberately LOOSER than `expand_config_header.py`'s `_AC_UNDEF`,
/// which must stay strict: that runs over gnulib's `*.in.h` too, where
/// `# undef GUARD` is a header undefining its own inclusion guard and
/// rewriting it breaks the split double-inclusion guard `#include_next`
/// depends on (bzl-vj5). Nothing here ever sees such a file — the only
/// caller passes the template named by `AC_CONFIG_HEADERS`, which is one
/// file per project.
fn undef_names(template_text: &str) -> Vec<String> {
    template_text
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix('#')?;
            let rest = rest.trim_start().strip_prefix("undef")?;
            // A following separator, so `#undefine` is not a match.
            if !rest.starts_with([' ', '\t']) {
                return None;
            }
            let name = rest.trim();
            (!name.is_empty() && !name.contains(char::is_whitespace)).then(|| name.to_string())
        })
        .collect()
}

/// Builds the `config_header` plan for an `AC_CONFIG_FILES` header — one
/// whose template is `@VAR@` substitution rather than `#undef` declarations.
///
/// Separate from [`plan_config_header`] because the two dialects disagree
/// about the SHAPE of an answer, not merely its source. A `#undef NAME` asks
/// whether a fact holds, so a catalog probe can answer it and the value gets
/// C-quoted the way `AC_DEFINE` would. An `@VAR@` is a textual replacement —
/// `#define JSON_INLINE @json_inline@` must yield the keyword `inline`, and
/// quoting it would emit a string literal where a keyword belongs.
///
/// No catalog lookup for the same reason: these names are the project's own
/// lowercase shell variables (`json_have_localeconv`), not portable toolchain
/// facts, so there is nothing for a probe to answer.
pub(crate) fn plan_substitution_header(
    output: &str,
    template: &str,
    template_text: &str,
    vars: &HashMap<String, String>,
) -> (ConfigHeader, Vec<String>) {
    let mut values = Vec::new();
    let mut unmapped = Vec::new();

    for name in crate::configure_file::substituted_names(template_text) {
        match vars.get(&name).filter(|v| !v.is_empty()) {
            // Raw, not quoted. See the dialect argument above.
            Some(value) => values.push((name, value.clone())),
            // Left literal in the output otherwise: an `@NAME@` token that
            // breaks every source including the header, which is a louder
            // failure than an undefined macro but no less wrong.
            None => unmapped.push(name),
        }
    }

    values.sort_unstable();
    values.dedup();
    unmapped.sort_unstable();
    unmapped.dedup();

    (
        ConfigHeader {
            output: output.to_string(),
            template: template.to_string(),
            template_source: None,
            catalog_probes: Vec::new(),
            values,
            // autoconf assembles a config header by substitution alone.
            // autoconf has no marker distinguishing a user option from a
            // probe result — `configure` is a shell script — so this stays
            // empty and such macros escalate instead. See
            // `model::ConfigHeader::options` and bzl-1p6.
            options: Vec::new(),
            splices: Vec::new(),
            // AC_CONFIG_FILES: `@VAR@` substitution, no declarations.
            dialect: crate::model::ConfigDialect::Substitution,
            // Not a shadowing header: these come from AC_CONFIG_HEADERS /
            // AC_CONFIG_FILES / configure_file, which generate a project's own
            // header rather than a replacement for a system one.
            shadow_dir: None,
        },
        unmapped,
    )
}

/// The headers a project generates through `AC_CONFIG_FILES` rather than
/// `AC_CONFIG_HEADERS`, read from `config.status`'s `config_files=`.
///
/// Both variables produce generated files and only one is a header source:
///
/// ```text
/// config_headers=" jansson_private_config.h"
/// config_files="  jansson.pc Makefile src/jansson_config.h test/Makefile"
/// ```
///
/// The distinction is not cosmetic. `AC_CONFIG_HEADERS` templates carry
/// `#undef` declarations, while `AC_CONFIG_FILES` templates are `@VAR@`
/// substitution only — the same dialect CMake's `configure_file` uses, which
/// is why the expansion reuses `configure_file`'s machinery rather than
/// `undef_names`.
///
/// Filtered by extension, which is a judgement the input only half states:
/// `config_files` is a flat list mixing Makefiles, `.pc` files and headers
/// with nothing marking which is which. jansson's ten entries are eight
/// Makefiles, a `.pc`, and one header. Extension is the only available
/// signal, and the failure direction is safe — a missed header fails loudly
/// at compile, while a wrongly-included Makefile would produce a
/// `config_header` rule nothing depends on.
pub(crate) fn parse_config_file_headers(config_status: &str) -> Vec<String> {
    let Some(line) = config_status
        .lines()
        .find_map(|l| l.trim().strip_prefix("config_files="))
    else {
        return Vec::new();
    };
    line.trim_matches(&['"', '\''][..])
        .split_whitespace()
        // Only the output side: unlike `config_headers`, entries here are not
        // written `output:template`, and autoconf's template is `<output>.in`.
        .map(|spec| spec.split(':').next().unwrap_or(spec))
        .filter(|path| {
            matches!(
                Path::new(path).extension().and_then(|e| e.to_str()),
                Some("h" | "hpp" | "hh" | "hxx")
            )
        })
        .map(str::to_string)
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

/// The values `configure` resolved for each config-header macro, read from
/// `config.status`'s own `D[]` table — the table it uses to write the header.
///
/// EVIDENCE, never an answer. These are the CONVERSION HOST's results, and
/// baking them is the anti-pattern the architecture forbids: `BYTEORDER` is
/// `1234` here and wrong on a big-endian consumer,
/// `TUKLIB_CPUCORES_SCHED_GETAFFINITY` is Linux-only. They are quoted into
/// the escalation so the agent can see what configure decided and judge
/// whether it holds for the consumer's toolchain — which is exactly the
/// judgement the escalation exists to ask for.
///
/// Deliberately NOT used to resolve anything. Measured across five projects:
/// no prefix rule separates a project option (`XML_DTD`) from a host fact
/// wearing a project-specific name (`BYTEORDER`), because the point of such
/// a name is that it hides which kind it is. See bzl-yjn.10.
pub(crate) fn parse_resolved_macro_values(config_status: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for line in config_status.lines() {
        // `D["NAME"]=" value"`. The leading space is autoconf's, separating
        // the macro from its value in the `#define` it will write, so it is
        // stripped rather than being part of the value.
        let Some(rest) = line.trim().strip_prefix("D[\"") else {
            continue;
        };
        let Some((name, rest)) = rest.split_once("\"]=") else {
            continue;
        };
        let value = rest.trim().trim_matches('"').trim();
        if !name.is_empty() && !value.is_empty() {
            values.insert(name.to_string(), value.to_string());
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    // An AC_CONFIG_FILES header substitutes @VAR@ and declares no #undef, so
    // planning it with the config.h dialect finds nothing at all. jansson's
    // six substitutions all resolve from make's variable database.
    #[test]
    fn a_substitution_header_resolves_at_vars_from_the_database() {
        const TEMPLATE: &str = "#define JSON_INLINE @json_inline@\n\
                                #define JSON_HAVE_LOCALECONV @json_have_localeconv@\n";
        let vars: HashMap<String, String> = [
            ("json_inline".to_string(), "inline".to_string()),
            ("json_have_localeconv".to_string(), "1".to_string()),
        ]
        .into_iter()
        .collect();

        let (header, unmapped) =
            plan_substitution_header("jansson_config.h", "jansson_config.h.in", TEMPLATE, &vars);

        assert!(
            unmapped.is_empty(),
            "every @VAR@ is in the database, so nothing should escalate: {unmapped:?}"
        );
        assert_eq!(
            header.values,
            vec![
                ("json_have_localeconv".to_string(), "1".to_string()),
                ("json_inline".to_string(), "inline".to_string()),
            ],
            "substituted TEXTUALLY, so neither value is C-quoted — unlike \
             AC_DEFINE, @VAR@ replaces the token in place and `\"inline\"` \
             would emit a string literal where a keyword belongs"
        );
    }

    #[test]
    fn a_substitution_header_escalates_a_var_the_database_cannot_answer() {
        const TEMPLATE: &str = "#define THING @unknowable_thing@\n";
        let (_, unmapped) = plan_substitution_header("x.h", "x.h.in", TEMPLATE, &HashMap::new());
        assert_eq!(
            unmapped,
            vec!["unknowable_thing".to_string()],
            "left LITERAL in the output otherwise — an `@NAME@` token the \
             compiler chokes on, which is worse than an undefined macro"
        );
    }

    // autoconf generates a config header TWO ways and the frontend long read
    // only one. jansson declares its private header with AC_CONFIG_HEADERS
    // and its PUBLIC one (jansson_config.h, included by the installed
    // jansson.h) inside AC_CONFIG_FILES — so the private one was reproduced
    // and the public one silently dropped, surfacing as a missing include
    // several steps from the cause.
    //
    // The list is mixed, which is the whole difficulty: jansson's ten entries
    // are eight Makefiles, a .pc file, and exactly one header.
    #[test]
    fn config_files_yields_only_the_entries_that_are_headers() {
        const STATUS: &str = "config_files=\" jansson.pc Makefile doc/Makefile \
                              src/jansson_config.h test/Makefile\"\n";
        assert_eq!(
            parse_config_file_headers(STATUS),
            vec!["src/jansson_config.h".to_string()],
            "a Makefile and a .pc are generated files this conversion has no \
             use for; only a header is an input to a compile"
        );
    }

    #[test]
    fn a_project_whose_config_files_has_no_header_yields_nothing() {
        const STATUS: &str = "config_files=\" Makefile src/Makefile foo.pc\"\n";
        assert!(
            parse_config_file_headers(STATUS).is_empty(),
            "the common case — most projects generate only Makefiles this way, \
             and inventing a config_header rule for one would emit a rule for \
             a file nothing includes: {:?}",
            parse_config_file_headers(STATUS)
        );
    }

    // AC_CONFIG_HEADERS names BOTH sides, and the template is not always
    // `<output>.in` — GNU hello declares config.h:config.in to keep the name
    // 8.3-safe, so assuming would find nothing.
    #[test]
    // config.status writes the resolved value of every macro it will define
    // into a `D[]` table. The frontend already opens this file for the
    // header/template names; these values are quoted into the escalation as
    // EVIDENCE so the agent can see what configure decided.
    #[test]
    fn resolved_macro_values_are_read_from_the_d_table() {
        let status = "\
D[\"PACKAGE_NAME\"]=\" \\\"Libidn2\\\"\"\n\
D[\"GNULIB_UNISTR_U32_MBTOUC_UNSAFE\"]=\" 1\"\n\
D[\"BYTEORDER\"]=\" 1234\"\n\
ac_cs_usage=\"whatever\"\n";
        let values = parse_resolved_macro_values(status);

        assert_eq!(
            values
                .get("GNULIB_UNISTR_U32_MBTOUC_UNSAFE")
                .map(String::as_str),
            Some("1"),
            "autoconf writes a leading space separating the macro from its \
             value; it is not part of the value: {values:?}"
        );
        assert_eq!(
            values.get("BYTEORDER").map(String::as_str),
            Some("1234"),
            "{values:?}"
        );
        assert!(
            values
                .get("PACKAGE_NAME")
                .is_some_and(|v| v.contains("Libidn2")),
            "a C string literal keeps its own quotes: {values:?}"
        );
        assert!(
            !values.contains_key("ac_cs_usage"),
            "only the D[] table, not every shell assignment in the file: \
             {values:?}"
        );
    }

    // The negative. A config.status with no D[] table is not an error — the
    // escalation simply carries no evidence — and an empty map must not be
    // confused with "every macro resolved to nothing".
    #[test]
    fn a_config_status_without_a_d_table_yields_nothing() {
        assert!(parse_resolved_macro_values("config_headers=\" config.h\"\n").is_empty());
    }

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

    // Both directions, because the two failures are opposite and each is
    // silent at the point it is made. An unquoted string splices into
    // neighbouring tokens (`printf("xz (" XZ Utils ")")` — clang reports
    // undeclared identifiers in a file that never mentions the macro), while a
    // quoted number is a type error wherever `#if` or arithmetic uses it.
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
            &HashMap::new(),
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

    // Both spellings autoconf actually uses. The rule used to require an
    // UNSPACED `#undef` at column 0, on the belief that a spaced one is
    // gnulib undefining its own include guard — true of gnulib, and false as
    // a rule about THIS function's input.
    //
    // undef_names only ever sees the template named by AC_CONFIG_HEADERS
    // (autotools.rs, the `parse_config_headers(&status)` loop). libidn2's
    // config.status says `config_headers=" config.h"` — one file. gnulib's
    // `gl/*.in.h` are replacement headers, reach codegen by a different path
    // as ConfigDialect::Substitution, and never arrive here.
    //
    // Measured on all five fetched projects: 46 spaced `# undef` lines and
    // every one a real declaration. libidn2 and xz carry the 22-macro
    // AC_USE_SYSTEM_EXTENSIONS block; expat and libmicrohttpd each indent
    // WORDS_BIGENDIAN two spaces inside a nested `#if`/`# ifndef`. Neither of
    // those is a gnulib project, so the strict rule was losing a real macro
    // in two projects that could not have had the problem it guarded against.
    //
    // The cc_config expander's `_AC_UNDEF` stays strict on purpose — it runs
    // over gnulib's templates too. See `expand_config_header.py`.
    #[test]
    fn a_spaced_or_indented_undef_is_still_a_declaration() {
        assert_eq!(
            undef_names("#undef PACKAGE_NAME\n# undef _GNU_SOURCE\n#  undef WORDS_BIGENDIAN\n"),
            vec![
                "PACKAGE_NAME".to_string(),
                "_GNU_SOURCE".to_string(),
                "WORDS_BIGENDIAN".to_string(),
            ],
            "autoconf writes AC_USE_SYSTEM_EXTENSIONS macros as `# undef` and \
             a nested WORDS_BIGENDIAN as `#  undef`; requiring column 0 left \
             both undefined"
        );
    }

    #[test]
    fn an_indented_undef_is_a_declaration_too() {
        assert_eq!(
            undef_names("  #undef WORDS_BIGENDIAN\n\t#undef OTHER\n"),
            vec!["WORDS_BIGENDIAN".to_string(), "OTHER".to_string()],
            "expat and libmicrohttpd both indent WORDS_BIGENDIAN inside a \
             conditional, and config.status rewrites it like any other"
        );
    }

    // The half of the old rule that survives: the relaxation is about
    // WHITESPACE before the directive, never about code before it.
    #[test]
    fn a_mid_line_undef_is_not_a_declaration() {
        assert!(
            undef_names("int x; #undef NOT_A_DECL\n").is_empty(),
            "only whitespace may precede the directive: {:?}",
            undef_names("int x; #undef NOT_A_DECL\n")
        );
    }

    // `configure` records a feature it did NOT find as the shell word `no`,
    // and the database keeps it. Taken as a substituted value that becomes
    // `#define HAVE_LIBUNISTRING "no"`, where autoconf writes
    // `/* #undef HAVE_LIBUNISTRING */` — and the two disagree in the worst
    // direction, since a non-empty string makes `#if HAVE_LIBUNISTRING` TRUE
    // exactly where autoconf makes it false.
    //
    // libidn2 also fails to COMPILE on it (clang rejects a string literal at
    // the start of a preprocessor expression), which is luck: a macro used
    // only in `#ifdef` would have taken the wrong branch in silence.
    #[test]
    fn a_shell_no_is_not_a_value() {
        let vars = HashMap::from([("HAVE_LIBUNISTRING".to_string(), "no".to_string())]);
        let (header, unmapped) = plan_config_header(
            "config.h",
            "config.h.in",
            "#undef HAVE_LIBUNISTRING\n",
            &vars,
            &HashMap::new(),
        );
        assert!(
            header.values.is_empty(),
            "`no` is configure's answer NO, not a value to define: {:?}",
            header.values
        );
        assert!(
            unmapped.is_empty(),
            "and it needs no agent decision — autoconf leaves the name \
             undefined, which is what an unresolved `#undef` already \
             produces: {unmapped:?}"
        );
    }

    // `yes` is the same fact in the other direction, and autoconf emits
    // neither word: measured on libidn2's generated config.h, zero macros are
    // defined to `yes` or `no`, quoted or bare. It is a configure-internal
    // boolean, so it resolves as the fact rather than as text.
    #[test]
    fn a_shell_yes_is_the_fact_not_the_word() {
        let vars = HashMap::from([("HAVE_THING".to_string(), "yes".to_string())]);
        let (header, unmapped) = plan_config_header(
            "config.h",
            "config.h.in",
            "#undef HAVE_THING\n",
            &vars,
            &HashMap::new(),
        );
        assert_eq!(
            header.values,
            vec![("HAVE_THING".to_string(), "1".to_string())],
            "autoconf writes `#define HAVE_THING 1`, never `yes`"
        );
        assert!(unmapped.is_empty(), "{unmapped:?}");
    }

    // The narrowness that keeps this from eating real data: only the bare
    // word counts. A version string or a path that merely contains it is an
    // ordinary value.
    #[test]
    fn a_value_that_merely_contains_no_is_untouched() {
        let vars = HashMap::from([
            ("PACKAGE_NAME".to_string(), "nostromo".to_string()),
            ("NOTE".to_string(), "no thanks".to_string()),
        ]);
        let (header, _) = plan_config_header(
            "config.h",
            "config.h.in",
            "#undef PACKAGE_NAME\n#undef NOTE\n",
            &vars,
            &HashMap::new(),
        );
        assert_eq!(
            header.values,
            vec![
                ("NOTE".to_string(), "\"no thanks\"".to_string()),
                ("PACKAGE_NAME".to_string(), "\"nostromo\"".to_string()),
            ],
            "only the exact word is configure's boolean: {:?}",
            header.values
        );
    }

    // `configure` seeds make's database with EVERY macro it knows, most of
    // them EMPTY. Treating an empty variable as a substituted value emitted
    //     "BITSIZEOF_PTRDIFF_T": """"
    // — an unclosed string literal, so the generated module would not parse.
    // The macro belongs in the escalation instead.
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
            &HashMap::new(),
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
    // A macro a boolean build option controls is RESOLVED, not escalated:
    // the input states it (configure shows the macro appearing and vanishing
    // with the flag), and codegen turns the pair into a bool_flag so the
    // option stays settable. Before this, HAVE_GREETING escalated as
    // unmapped and the agent had to decide.
    #[test]
    fn a_flag_controlled_macro_resolves_as_an_option() {
        let flags = HashMap::from([("HAVE_GREETING".to_string(), "--enable-greeting".to_string())]);
        let (header, unmapped) = plan_config_header(
            "config.h",
            "config.h.in",
            "#undef HAVE_GREETING\n",
            &HashMap::new(),
            &flags,
        );

        assert!(unmapped.is_empty(), "the flag resolves it: {unmapped:?}");
        assert_eq!(
            header.options,
            vec![("HAVE_GREETING".to_string(), "--enable-greeting".to_string())],
            "and its provenance is the FLAG, which codegen needs to name the \
             bool_flag target"
        );
        assert_eq!(
            header.values,
            vec![("HAVE_GREETING".to_string(), "1".to_string())],
            "with the value config.status writes for an AC_DEFINE"
        );
    }

    // The negative: a name no flag controls still escalates. Resolving one
    // wrongly would bake a toolchain fact in as a project choice, which is
    // the direction that fails silently.
    #[test]
    fn a_macro_no_flag_controls_still_escalates() {
        let (header, unmapped) = plan_config_header(
            "config.h",
            "config.h.in",
            "#undef HAVE_SOMETHING\n",
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(unmapped, vec!["HAVE_SOMETHING".to_string()]);
        assert!(header.options.is_empty(), "{:?}", header.options);
    }
}
