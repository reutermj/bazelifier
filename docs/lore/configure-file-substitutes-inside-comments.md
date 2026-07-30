# configure_file substitutes the whole file, comments included

`configure_file(config.h.in config.h)` does not understand C. It performs
`@VAR@`/`${VAR}` substitution and `#cmakedefine` rewriting over **every line**,
including C comments. Two consequences bite when writing a `.h.in` template (a
fixture's, or a real project's):

1. An `@FOO@` **inside a comment** is substituted like any other. If `FOO` is a
   boolean probe that came back false, `@FOO@` becomes empty, so a comment
   reading `The @FOO@ MUST be set` turns into `The  MUST be set` in the output.
   Harmless as prose, but it means you cannot mention `@VAR@` literally in a
   template comment.

2. The generated file is then compiled, so any character that is a **stray
   token to the C compiler** — a bare backtick, an em-dash (`U+2014`), or an
   unbalanced apostrophe (`check's` reads as an unterminated char literal) — is
   a compile error, even though it sits in a comment. `configure_file` copies
   those bytes through untouched; the compiler is the one that rejects them.

3. **The translator's `parse_template_macros` scans comments too** — it must,
   since a *defined* `@VAR@` in a comment really is substituted (consequence 1).
   So an `@VAR@` written as comment prose is parsed as a real variable
   reference. If it names nothing the translator can resolve, the unresolved-
   `@VAR@` escalation (bzl-fxa.9) fires on it — even though CMake itself would
   have left that undefined `@VAR@` literal and the build would have been fine.
   The escalation is correct given what the translator can see; the template is
   what's wrong.

Both `@FOO@`-in-comment cases surfaced from fixtures. Consequence 2 was writing
`013-configure-file-if-define`: a comment using `check's`, backticks, an
em-dash, and `@FOO@` produced a `config.h` that failed to compile with
`missing terminating ' character` and several `stray` errors — none about the
actual directives. Consequence 3 was `012-configure-file`: a comment reading
`A plain @VAR@ from the project() VERSION` made the translator escalate a
phantom macro `VAR`, turning a green fixture red at test time (not at
conversion time — the gate only fires when the comparison runs), which is the
trap: a stray comment `@VAR@` is invisible until an unpacked-workspace run.

**Rule for templates: keep comments plain 7-bit ASCII, no backticks, no
apostrophes in contractions, and never write a literal `@VAR@`/`${VAR}` in a
comment.** Real autoconf-derived projects (json-c included) already follow this
for exactly this reason; it looks like an arbitrary style choice until you see
the error. It is not the translator's job to sanitize a template's comments —
the input is immutable, and a real project's template already compiles.
