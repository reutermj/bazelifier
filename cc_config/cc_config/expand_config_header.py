#!/usr/bin/env python3
"""Expand a CMake configure_file template into a config header.

The config-header slice of `configure_file`, not all of it. Driven by probe
results (files containing "true"/"false", each keyed to the macro it controls
via a MACRO=path argument) and a plain {name: value} map for @VAR@
substitutions. See ../../docs/architecture/configure-file-and-toolchain-probes.md.

Directives handled (matching CMake):
  #cmakedefine FOO [rest]  -> "#define FOO [rest]"  if FOO is set, else
                              "/* #undef FOO */"
  #cmakedefine01 FOO       -> "#define FOO 1" or "#define FOO 0"
  @VAR@ and ${VAR}         -> the value of VAR (left untouched if unknown, so
                              an unresolved reference surfaces loudly)

Every probe result is folded into the values map (see main), so both a
#cmakedefine and a @VAR@ referencing the same name resolve consistently: FOO
is "set" iff its value is CMake-true (see _is_cmake_true) — matching CMake,
where #cmakedefine is defined when the name is truthy, and OFF/FALSE/NO/0 are
not. A bare @VAR@ still substitutes the raw value, token and all.
"""

import argparse
import json
import re
import sys

# The prefix is anything before the directive, not just indentation. CMake
# does not require `#cmakedefine` to start the line — json-c writes
# `/* #cmakedefine json_c_strtoll @json_c_strtoll@*/`, and CMake expands it to
# `/* #define json_c_strtoll strtoll*/`. Anchoring on `^(\s*)` left those
# lines verbatim, so the generated header kept a literal "#cmakedefine" that
# CMake's own output does not have (bzl-fxa.24).
#
# Verified against CMake 3.28: a prefix of `/* `, of two spaces, and of `xx`
# all expand, so the rule is "wherever it appears", not "in comments".
# autoconf's dialect. Where CMake writes `#cmakedefine FOO`, autoconf's
# config.h.in writes a bare `#undef FOO` — the SAME statement ("define this if
# the probe succeeded"), spelled as the false case. autoconf's own config.status
# rewrites the line to `#define FOO 1` when set and leaves it as a comment when
# not, which is exactly what this expander already does for #cmakedefine.
#
# Matched only as a WHOLE line, unlike _CMAKEDEFINE: a `#undef` with anything
# around it is ordinary C undefining a macro, and rewriting that would corrupt
# a header rather than configure it. autoconf emits its own directives alone on
# the line.
#
# "Anything around it" includes a SPACE AFTER THE HASH. The pattern was
# `#\s*undef` and that permitted `# undef GUARD`, which gnulib's headers write
# inside an `#if` to undefine a macro they set themselves — rewriting it to
# `/* #undef GUARD */` breaks the split double-inclusion guard `#include_next`
# depends on (bzl-vj5). autoheader always emits `#undef` unspaced at column 0.
#
# The trailing `\s*$` and the exact `#undef` prefix are what enforce this, not
# the leading `^` — these are applied with `re.match`, which already anchors at
# the start. The `^` is therefore redundant and kept only for symmetry with the
# patterns below.
_AC_UNDEF = re.compile(r"^#undef[ \t]+(\S+)\s*$")

# autoconf's one TERNARY macro family. AC_CHECK_DECLS writes "Define to 1 if
# you have the declaration of `X', and to 0 if you don't" — so the macro is
# ALWAYS defined, and its absence means something different from its being 0.
# Projects rely on both readings in the same file:
#
#     #ifdef HAVE_DECL_CPU_SETSIZE      <- presence
#     #  if 0 == HAVE_DECL_CPU_SETSIZE  <- value
#
# Rendering a false result as `/* #undef ... */` fails the outer test where
# autoconf passes it. libmicrohttpd happens to survive that (its inner branch
# undefs anyway), but a project writing `#if HAVE_DECL_X` under an `#ifdef`
# guard would take the wrong branch in silence.
#
# Anchored at the start, so a project's own `MHD_HAVE_DECL_*` is not caught.
_AC_TERNARY = re.compile(r"^HAVE_DECL_")

# The SPACED form, which is ambiguous and so is resolved by evidence rather
# than by shape. Two dialects write it and this expander sees both:
#
#   autoconf   `# undef _GNU_SOURCE` inside `#ifndef _GNU_SOURCE`
#              (AC_USE_SYSTEM_EXTENSIONS), and `#  undef WORDS_BIGENDIAN`
#              nested two levels — real declarations config.status rewrites.
#   gnulib     `# undef _GL_ALREADY_INCLUDING_LIMITS_H` inside an `#if`,
#              a header undefining its own inclusion guard. Rewriting it
#              breaks the split double-inclusion guard (bzl-vj5).
#
# Nothing in the LINE separates them, so the caller decides: a spaced undef is
# a declaration only when the name is one the translator resolved and passed
# in (a probe result or a `values` entry). gnulib's guards are internal to the
# header and are never passed, so they fall through untouched.
#
# This is why the pattern above stays strict rather than growing a `\s*`. An
# unspaced `#undef` at column 0 is unambiguous and resolves even for a name
# with no value; a spaced one needs the corroboration.
_AC_UNDEF_SPACED = re.compile(r"^[ \t]*#[ \t]*undef[ \t]+(\S+)\s*$")

_CMAKEDEFINE01 = re.compile(r"^(.*?)#cmakedefine01\s+(\S+)\s*$")
_CMAKEDEFINE = re.compile(r"^(.*?)#cmakedefine\s+(\S+)(.*)$")
_VAR = re.compile(r"@([A-Za-z0-9_]+)@|\$\{([A-Za-z0-9_]+)\}")

# CMake's if()-false constants (docs: "Constant"). A #cmakedefine tests the
# name against these, NOT against Python truthiness — bool("OFF") is True but
# CMake treats OFF as false, so `#cmakedefine ENABLE_X` with ENABLE_X=OFF must
# undef, not `#define ENABLE_X OFF`. This is the bzl-fxa.8 gap: an option left
# OFF reached a config value verbatim and, under bool(), defined the macro to
# the literal token. Note a bare @VAR@ still substitutes that token literally
# (CMake does: `#define BARE @OFF_VAR@` -> `#define BARE OFF`); only the
# define-presence question uses these.
_CMAKE_FALSE = frozenset(["", "0", "off", "false", "n", "no", "ignore", "notfound"])


def _is_cmake_true(value):
    """CMake if()-truthiness of a string value: false constants (and any
    *-NOTFOUND) are false; everything else is true."""
    v = value.strip().lower()
    return v not in _CMAKE_FALSE and not v.endswith("-notfound")


def _substitute_vars(text, values):
    def repl(m):
        name = m.group(1) or m.group(2)
        # Leave an unknown reference in place rather than blanking it: an
        # unresolved @FOO@ becomes a visible compile error in the header,
        # which is safer than a silently-empty value while the map is by hand.
        return values[name] if name in values else m.group(0)

    return _VAR.sub(repl, text)


def splice_files(template, splices):
    """Insert whole files after the lines matching their markers — sed's `r`.

    gnulib assembles a generated header from parts: the template carries
    `/* The definitions of _GL_FUNCDECL_RPL etc. are copied here.  */` and the
    recipe splices `c++defs.h` in at that comment. No `#include` is involved,
    which is why nothing else in this file would see it.

    Three behaviours copied from sed, each of which changes the output:

    - the matched line is KEPT and the file goes after it, not over it;
    - the marker is a substring match on the line, not an anchored one;
    - every matching line gets a copy, since `r` is not once-only.

    `splices` is an ordered list of `(marker, path)`. Applied in order, and a
    later splice can match a line an earlier one inserted — as sed would.
    """
    for marker, path in splices:
        with open(path) as fh:
            content = fh.read()
        if content and not content.endswith("\n"):
            content += "\n"
        out = []
        for line in template.splitlines(keepends=True):
            out.append(line)
            if marker in line:
                if not line.endswith("\n"):
                    out.append("\n")
                out.append(content)
        template = "".join(out)
    return template


def expand(template, is_set, values):
    out = []
    for line in template.splitlines(keepends=True):
        eol = "\n" if line.endswith("\n") else ""
        body = line[:-1] if eol else line

        m01 = _CMAKEDEFINE01.match(body)
        if m01:
            prefix, name = m01.group(1), m01.group(2)
            out.append("%s#define %s %d%s" % (prefix, name, 1 if is_set(name) else 0, eol))
            continue

        # The spaced form first, since it is the narrower claim: it fires only
        # for a name the caller resolved. An unresolved one falls through to
        # the strict pattern below, which will not match it either, so the
        # line is emitted verbatim — the behaviour gnulib's guards need.
        mspaced = _AC_UNDEF_SPACED.match(body)
        if mspaced and mspaced.group(1) in values:
            name = mspaced.group(1)
            if is_set(name):
                out.append("#define %s %s%s" % (name, values.get(name, "1"), eol))
            elif _AC_TERNARY.match(name):
                # This pattern's `[ \t]*` also matches the UNSPACED form, so
                # it reaches here before `_AC_UNDEF` below and has to honour
                # the ternary contract too. It did not, and the unspaced
                # `#undef HAVE_DECL_X` that every autoheader template writes
                # took the plain-undef path — the branch under `_AC_UNDEF`
                # was unreachable for exactly the names it was written for.
                out.append("#define %s 0%s" % (name, eol))
            else:
                out.append("/* #undef %s */%s" % (name, eol))
            continue

        mundef = _AC_UNDEF.match(body)
        if mundef:
            name = mundef.group(1)
            # Same resolution as #cmakedefine, deliberately: both ask whether a
            # probe succeeded, so a project's autoconf template and its CMake
            # equivalent produce identical output from identical probe results.
            if is_set(name):
                value = values.get(name, "1")
                out.append("#define %s %s%s" % (name, value, eol))
            elif _AC_TERNARY.match(name):
                # Defined to 0, not undefined — see _AC_TERNARY.
                out.append("#define %s 0%s" % (name, eol))
            else:
                out.append("/* #undef %s */%s" % (name, eol))
            continue

        mdef = _CMAKEDEFINE.match(body)
        if mdef:
            prefix, name, rest = mdef.group(1), mdef.group(2), mdef.group(3)
            if is_set(name):
                # The value after the name is itself @VAR@-expanded, e.g.
                # `#cmakedefine FOO "@FOO_VALUE@"`.
                out.append(
                    "%s#define %s%s%s" % (prefix, name, _substitute_vars(rest, values), eol)
                )
            else:
                # The prefix is DROPPED here, not preserved. CMake renders the
                # unset form as its own comment and discards whatever preceded
                # the directive: `/* #cmakedefine FOO @V@*/` becomes
                # `/* #undef FOO */`, not `/* /* #undef FOO */`. Keeping the
                # prefix would nest comment openers and, for a C prefix,
                # produce something that no longer compiles.
                out.append("/* #undef %s */%s" % (name, eol))
            continue

        out.append(_substitute_vars(body, values) + eol)
    return "".join(out)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--template", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--values", required=True, help="JSON object of @VAR@ values.")
    parser.add_argument(
        "--result",
        action="append",
        default=[],
        help="MACRO=/path/to/result-file (containing true/false), repeatable.",
    )
    parser.add_argument(
        "--splice",
        action="append",
        default=[],
        help="MARKER=/path/to/file, repeatable and ORDER-SIGNIFICANT: the "
        "file is inserted after each line containing MARKER (sed's `r`).",
    )
    args = parser.parse_args()

    with open(args.template) as fh:
        template = fh.read()
    with open(args.values) as fh:
        values = json.load(fh)

    # A probe result is either boolean ("true"/"false" — a check_include_file
    # or check_symbol_exists) or a value (a number from check_type_size). Every
    # probe becomes a `values` entry so it drives BOTH the #cmakedefine (via
    # is_set) AND any @VAR@ that references it — matching CMake, where after
    # configure_file `@FOO@` substitutes the value of the variable FOO the
    # check set. A boolean check sets FOO to "1" on success (falsey/empty on
    # failure), so:
    #   true  -> "1"   (#cmakedefine fires; @FOO@ -> "1")
    #   false -> ""    (#cmakedefine undefs; @FOO@ -> "")
    # A size probe's number is its own value. Projects write facts as
    # `#cmakedefine FOO @FOO@` and then evaluate them with `#if FOO`, so a
    # boolean probe MUST populate its @VAR@ or the `#if` sees a literal `@FOO@`
    # and fails to compile (the json-c gap this fixes). An explicit `values`
    # entry from the translator (a version string, an option) wins over a
    # probe of the same name.
    for spec in args.result:
        macro, _, path = spec.partition("=")
        if macro in values:
            continue
        with open(path) as fh:
            answer = fh.read().strip()
        if answer == "true":
            values[macro] = "1"
        elif answer == "false":
            values[macro] = ""
        elif answer:
            values[macro] = answer

    def is_set(name):
        # A name absent from values is unset; a present one is "set" iff its
        # value is CMake-true (so OFF/FALSE/NO/0/NOTFOUND undef, matching CMake).
        return name in values and _is_cmake_true(values[name])

    splices = []
    for spec in args.splice:
        marker, _, path = spec.partition("=")
        if not path:
            sys.stderr.write("--splice needs MARKER=path, got %r\n" % spec)
            return 1
        splices.append((marker, path))

    # AFTER expansion, which is the order the recipe uses and NOT the intuitive
    # one. gnulib's `r` commands are in the LAST sed pass, so the spliced text
    # is never substituted. Verified on libidn2: c++defs.h contains seven
    # @VAR@ references and all seven survive verbatim into the generated
    # stdlib.h (they sit in a documentation comment, where an expanded value
    # would be actively misleading). Splicing first would substitute them and
    # produce a header the project's own build never generates.
    with open(args.output, "w") as fh:
        fh.write(splice_files(expand(template, is_set, values), splices))
    return 0


if __name__ == "__main__":
    sys.exit(main())
