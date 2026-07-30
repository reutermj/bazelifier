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

FOO is "set" when a probe named FOO reported "true" OR FOO has a non-empty
entry in the values map — matching CMake, where #cmakedefine is defined when
the name is truthy in any variable, probe-derived or plain.
"""

import argparse
import json
import re
import sys

_CMAKEDEFINE01 = re.compile(r"^(\s*)#cmakedefine01\s+(\S+)\s*$")
_CMAKEDEFINE = re.compile(r"^(\s*)#cmakedefine\s+(\S+)(.*)$")
_VAR = re.compile(r"@([A-Za-z0-9_]+)@|\$\{([A-Za-z0-9_]+)\}")


def _substitute_vars(text, values):
    def repl(m):
        name = m.group(1) or m.group(2)
        # Leave an unknown reference in place rather than blanking it: an
        # unresolved @FOO@ becomes a visible compile error in the header,
        # which is safer than a silently-empty value while the map is by hand.
        return values[name] if name in values else m.group(0)

    return _VAR.sub(repl, text)


def expand(template, is_set, values):
    out = []
    for line in template.splitlines(keepends=True):
        eol = "\n" if line.endswith("\n") else ""
        body = line[:-1] if eol else line

        m01 = _CMAKEDEFINE01.match(body)
        if m01:
            indent, name = m01.group(1), m01.group(2)
            out.append("%s#define %s %d%s" % (indent, name, 1 if is_set(name) else 0, eol))
            continue

        mdef = _CMAKEDEFINE.match(body)
        if mdef:
            indent, name, rest = mdef.group(1), mdef.group(2), mdef.group(3)
            if is_set(name):
                # The value after the name is itself @VAR@-expanded, e.g.
                # `#cmakedefine FOO "@FOO_VALUE@"`.
                out.append("%s#define %s%s%s" % (indent, name, _substitute_vars(rest, values), eol))
            else:
                out.append("%s/* #undef %s */%s" % (indent, name, eol))
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
    args = parser.parse_args()

    with open(args.template) as fh:
        template = fh.read()
    with open(args.values) as fh:
        values = json.load(fh)

    probe_true = set()
    for spec in args.result:
        macro, _, path = spec.partition("=")
        with open(path) as fh:
            if fh.read().strip() == "true":
                probe_true.add(macro)

    def is_set(name):
        return name in probe_true or bool(values.get(name))

    with open(args.output, "w") as fh:
        fh.write(expand(template, is_set, values))
    return 0


if __name__ == "__main__":
    sys.exit(main())
