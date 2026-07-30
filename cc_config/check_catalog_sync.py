#!/usr/bin/env python3
"""Fails if the translator's CATALOG_DEFINES drifts from the cc_config catalog.

The catalog's set of probed defines is declared twice, deliberately: once in
Starlark (cc_config/catalog/BUILD.bazel), which the Rust translator cannot read
at codegen time, and once as CATALOG_DEFINES in translator/src/cmake_api.rs.
The facts are stable autoconf checks that rarely change, so two hand-maintained
copies are acceptable — but only if a drift between them is caught. This script
is that catch; run it in review (cheap enough for CI). Run from the repo root.

It compares the set of define strings in each file. It does not check that a
define actually has a probe target (the catalog macro guarantees that); it
checks the two lists agree.
"""
import re
import sys

BUILD = "cc_config/catalog/BUILD.bazel"
RUST = "translator/src/cmake_api.rs"

# In BUILD.bazel the defines are the 2nd/3rd tuple element of the header/type
# entries and the 3rd of symbols — but every one is an uppercase HAVE_*/SIZEOF_*
# string literal, so match those directly (the catalog has no other such
# literals).
DEFINE = re.compile(r'"((?:HAVE_|SIZEOF_)[A-Z0-9_]+)"')


def defines_in(path):
    with open(path) as fh:
        text = fh.read()
    # For the Rust file, only look inside the CATALOG_DEFINES array, so an
    # unrelated HAVE_* string elsewhere doesn't count.
    if path == RUST:
        m = re.search(r"const CATALOG_DEFINES.*?\[(.*?)\];", text, re.S)
        if not m:
            print(f"FAIL: could not find CATALOG_DEFINES in {path}", file=sys.stderr)
            sys.exit(1)
        text = m.group(1)
    return set(DEFINE.findall(text))


def main():
    build_defines = defines_in(BUILD)
    rust_defines = defines_in(RUST)

    if build_defines == rust_defines:
        print(f"PASS: CATALOG_DEFINES and the catalog agree ({len(build_defines)} defines).")
        return 0

    only_build = sorted(build_defines - rust_defines)
    only_rust = sorted(rust_defines - build_defines)
    print("FAIL: the translator's CATALOG_DEFINES has drifted from the catalog.", file=sys.stderr)
    if only_build:
        print(f"  in {BUILD} but not CATALOG_DEFINES: {only_build}", file=sys.stderr)
    if only_rust:
        print(f"  in CATALOG_DEFINES but not {BUILD}: {only_rust}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
