#!/usr/bin/env python3
"""Agent-stage resolution for the converted expat module.

    python3 tools/resolutions/expat/resolve_expat.py W/fixtures/expat

All-or-nothing on its anchors. One item, the config header; every name is
decided below from expat's configure.ac (2.6.2) rather than from this host's
expat_config.h.
"""
import glob, os, sys

M = sys.argv[1]
B = os.path.join(M, "BUILD.bazel")
s = open(B).read()

def rep(old, new):
    global s
    assert s.count(old) == 1, (old[:80], s.count(old))
    s = s.replace(old, new)

LOAD = 'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n'
rep(LOAD, LOAD + 'load("@cc_config//cc_config:probe.bzl", "check_c_source_compiles")\n')

# Toolchain facts, asked of the consumer's compiler with configure's own
# snippets.
rep('config_header(\n    name = "expat_config_h",', '''# configure.ac's `--with-sys-getrandom` check, default "check": a Linux
# syscall, so a probe rather than a value.
check_c_source_compiles(
    name = "expat_have_syscall_getrandom",
    define = "HAVE_SYSCALL_GETRANDOM",
    link = True,
    source = "#include <stdlib.h>\\n#include <unistd.h>\\n#include <sys/syscall.h>\\nint main() {\\n  syscall(SYS_getrandom, NULL, 0, 0);\\n  return 0;\\n}\\n",
)

# AC_C_BIGENDIAN, asked of the compiler's own byte-order macro.
check_c_source_compiles(
    name = "expat_words_bigendian",
    define = "WORDS_BIGENDIAN",
    source = "int probe[__BYTE_ORDER__ == __ORDER_BIG_ENDIAN__ ? 1 : -1];\\n",
)

config_header(
    name = "expat_config_h",''')

rep('''        "@cc_config//catalog:stdc_headers",
    ],
    unresolved = [
        "AC_APPLE_UNIVERSAL_BUILD",
        "BYTEORDER",
        "HAVE_CXX11",
        "HAVE_LIBBSD",
        "HAVE_SYSCALL_GETRANDOM",
        "LT_OBJDIR",
        "PACKAGE_URL",
        "WORDS_BIGENDIAN",
        "const",
        "off_t",
        "size_t",
    ],
''', '''        "@cc_config//catalog:stdc_headers",
        ":expat_have_syscall_getrandom",
        ":expat_words_bigendian",
    ],
''')

rep('''    }) | {
        "PACKAGE": "\\"expat\\"",''', '''    }) | {
        "AC_APPLE_UNIVERSAL_BUILD": "",  # no Apple universal build
        # configure sets BYTEORDER beside WORDS_BIGENDIAN from the same
        # AC_C_BIGENDIAN answer; the template wants the number, so it is the
        # compiler's byte order as an expression (xmltok.c only tests it in
        # #if), not this host's 1234.
        "BYTEORDER": "(__BYTE_ORDER__ == __ORDER_BIG_ENDIAN__ ? 4321 : 1234)",
        "HAVE_CXX11": "1",  # --with-tests, the default the ground truth used
        "HAVE_LIBBSD": "",  # --with-libbsd defaults to no
        "LT_OBJDIR": "\\".libs/\\"",
        "PACKAGE_URL": "\\"\\"",
        # autoconf's fallback definitions, only for a toolchain lacking them
        "const": "",
        "off_t": "",
        "size_t": "",
        "PACKAGE": "\\"expat\\"",''')

open(B, "w").write(s)
for f in glob.glob(os.path.join(M, "needs_attention", "001-*.md")):
    os.remove(f)
