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

This surfaced writing fixture `013-configure-file-if-define`: a template whose
comment used `check's`, backticks, an em-dash, and `@FOO@` produced a
`config.h` that failed to compile with `missing terminating ' character` and
several `stray` errors — none of them about the actual directives.

**Rule for templates: keep comments plain 7-bit ASCII, no backticks, no
apostrophes in contractions, and never write a literal `@VAR@`/`${VAR}` in a
comment.** Real autoconf-derived projects (json-c included) already follow this
for exactly this reason; it looks like an arbitrary style choice until you see
the error. It is not the translator's job to sanitize a template's comments —
the input is immutable, and a real project's template already compiles.
