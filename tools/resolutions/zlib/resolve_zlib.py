#!/usr/bin/env python3
"""Agent-stage resolution for the converted zlib module.

    python3 tools/resolutions/zlib/resolve_zlib.py W/fixtures/zlib

All-or-nothing on its anchors.
"""
import glob, os, sys

M = sys.argv[1]
B = os.path.join(M, "BUILD.bazel")
s = open(B).read()

def rep(old, new):
    global s
    assert s.count(old) == 1, (old[:80], s.count(old))
    s = s.replace(old, new)

# Item 001: Z_PREFIX is the project option ZLIB_PREFIX (CMakeLists.txt,
# `option(ZLIB_PREFIX ... OFF)`), not a toolchain fact. The ground truth was
# built with the default, so it stays undefined; "" is undefined on a CMake
# template.
rep('''    unresolved = [
        "Z_PREFIX",
    ],
''', '''    values = {
        "Z_PREFIX": "",  # option(ZLIB_PREFIX ... OFF), the default the ground truth used
    },
''')

# Item 002: all 13 test the build SYSTEM, which a converted module does not
# have, so none is reproduced. Recorded here so the omission is visible.
rep('''sh_test(
    name = "zlib_example_test",''', '''# Registered tests NOT reproduced, deliberately (needs_attention item 002):
#   zlib_coverage-summary          gcov report over a coverage build
#   zlib_install                   `cmake --install`
#   zlib_find_package_*            an installed zlib found by find_package()
#   zlib_add_subdirectory_*        zlib consumed via add_subdirectory()
# Each checks CMake packaging, which this module does not reproduce. The
# coverage driver itself (infcover) still runs, as zlib_coverage_test.
sh_test(
    name = "zlib_example_test",''')

open(B, "w").write(s)
for f in glob.glob(os.path.join(M, "needs_attention", "00[12]-*.md")):
    os.remove(f)
