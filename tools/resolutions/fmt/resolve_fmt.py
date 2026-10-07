#!/usr/bin/env python3
"""Agent-stage resolution for the converted fmt module.

    python3 tools/resolutions/fmt/resolve_fmt.py W/fixtures/fmt

All-or-nothing on its anchors. Both items are header_visibility: the
headers promoted to `hdrs` are the ones a dependent in this module
#includes by that spelling (grepped over test/*.cc, 2026-10-06) AND whose
owner is the target in question. The header-only test helpers (scan.h,
test-assert.h, posix-mock.h, mock-allocator.h) stay in `srcs`: no target
owns them, so naming test-main their public home would be a guess, and
srcs still propagates them to every dependent.
"""
import os, sys

M = sys.argv[1]
B = os.path.join(M, "BUILD.bazel")
s = open(B).read()

def rep(old, new):
    global s
    assert s.count(old) == 1, (old[:80], s.count(old))
    s = s.replace(old, new)

# gtest: every test reaches gmock.h/gtest.h through it, and
# gtest-extra-test.cc includes gtest-spi.h.
rep('''        "test/gtest/gmock-gtest-all.cc",
        "test/gtest/gmock/gmock.h",
        "test/gtest/gtest/gtest.h",
        "test/gtest/gtest/gtest-spi.h",
    ],
''', '''        "test/gtest/gmock-gtest-all.cc",
    ],
    hdrs = [
        "test/gtest/gmock/gmock.h",
        "test/gtest/gtest/gtest.h",
        "test/gtest/gtest/gtest-spi.h",
    ],
''')

# test-main: the two headers whose implementation it compiles. The test
# binaries list the same sources, so the edit is confined to its block.
start = s.index('    name = "test-main",')
end = s.index("\n)\n", start)
head, s, tail = s[:start], s[start:end], s[end:]
rep('''        "test/gtest-extra.cc",
        "test/gtest-extra.h",
        "test/util.cc",
''', '''        "test/gtest-extra.cc",
        "test/util.cc",
''')
rep('''        "test/test-assert.h",
        "test/util.h",
        "test/gtest/gmock/gmock.h",
        "test/gtest/gtest/gtest.h",
    ],
    includes = [
        "include",
    ],
''', '''        "test/test-assert.h",
        "test/gtest/gmock/gmock.h",
        "test/gtest/gtest/gtest.h",
    ],
    hdrs = [
        "test/gtest-extra.h",
        "test/util.h",
    ],
    includes = [
        "include",
    ],
''')
s = head + s + tail

open(B, "w").write(s)
for f in ("001-library-gtest-has-headers-with-no-public-declaration.md",
          "002-library-test-main-has-headers-with-no-public-declaration.md"):
    os.remove(os.path.join(M, "needs_attention", f))
