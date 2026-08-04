#!/usr/bin/env python3
"""Macros a `configure` defines from an --enable/--with FLAG rather than a probe.

The block starts at `# Check whether --enable-X was given.` and runs until the
next such marker or the next probe (`{ printf ... checking ...`), whichever
comes first — otherwise every #define downstream is falsely attributed.
"""
import re, sys

START = re.compile(r'^# Check whether --(enable|with)-([a-z0-9-]+) was given')
PROBE = re.compile(r'checking (for|whether|if) ')
DEFINE = re.compile(r'#define ([A-Z_][A-Z0-9_]*)\b')

def flag_macros(text):
    out, cur = {}, None
    for line in text.splitlines():
        m = START.match(line)
        if m:
            cur = f"--{m.group(1)}-{m.group(2)}"
            continue
        if cur and PROBE.search(line):
            cur = None          # a real probe ends the flag's region
            continue
        if cur:
            d = DEFINE.search(line)
            if d:
                out.setdefault(d.group(1), cur)
    return out

if __name__ == "__main__":
    for macro, flag in sorted(flag_macros(open(sys.argv[1], errors="ignore").read()).items()):
        print(f"{macro}\t{flag}")
