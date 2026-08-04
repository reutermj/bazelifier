#!/usr/bin/env python3
"""Harvest (macro, header path) pairs from an autoconf `configure` script.

autoconf states the mapping itself: a header check emits
`#define HAVE_$ac_header 1` with non-alphanumerics uppercased to `_`. So the
forward direction is deterministic; only guessing BACKWARD from the macro is
ambiguous (HAVE_NETINET_IP_ICMP_H could be netinet/ip_icmp.h or
netinet/ip/icmp.h).

Two forms carry the paths:
  ac_fn_c_check_header_compile "$LINENO" "sys/capsicum.h" "ac_cv_header_..."
  for ac_header in byteswap.h sys/endian.h sys/byteorder.h
"""
import re, sys

LITERAL = re.compile(r'ac_fn_c_check_header[a-z_]*\s+"\$LINENO"\s+"([^"$]+\.h)"')
LOOP = re.compile(r'^\s*for ac_header in (.+)$')
# A third form, and the only one that states the macro itself rather than
# leaving it to the transform: `ac_header_c_list " path cache_var MACRO"`.
# autoconf uses it for the headers it checks unconditionally up front.
CLIST = re.compile(r'ac_header_c_list\s+"\s*([^\s"]+\.h)\s+\S+\s+(HAVE_[A-Z0-9_]+)"')


def macro_for(path):
    """autoconf's own transform: HAVE_ + uppercase, non-alnum -> _."""
    return "HAVE_" + re.sub(r'[^A-Za-z0-9]', '_', path).upper()


def harvest(text):
    paths = set()
    stated = {}
    for line in text.splitlines():
        for m in CLIST.finditer(line):
            # Trust the script's own macro over the transform. They agree
            # everywhere measured, but the script is the authority.
            stated[m.group(2)] = m.group(1)
        for m in LITERAL.finditer(line):
            paths.add(m.group(1))
        loop = LOOP.match(line)
        if loop:
            for tok in loop.group(1).split():
                # Skip shell metachars and variable references.
                if tok.endswith('.h') and '$' not in tok and '`' not in tok:
                    paths.add(tok)
    pairs = {macro_for(p): p for p in paths}
    pairs.update(stated)
    return pairs


if __name__ == "__main__":
    pairs = harvest(open(sys.argv[1], errors="ignore").read())
    for macro, path in sorted(pairs.items()):
        print(f"{macro}\t{path}")
