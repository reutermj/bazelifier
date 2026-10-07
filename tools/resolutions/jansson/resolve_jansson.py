#!/usr/bin/env python3
"""Agent-stage resolution for the converted jansson module.

    python3 tools/resolutions/jansson/resolve_jansson.py W/fixtures/jansson

All-or-nothing on its anchors. Decisions read from jansson 2.14's
configure.ac and test harness.
"""
import glob, os, re, shutil, sys

HERE = os.path.dirname(os.path.abspath(__file__))
M = sys.argv[1]
B = os.path.join(M, "BUILD.bazel")
s = open(B).read()

def rep(old, new):
    global s
    assert s.count(old) == 1, (old[:80], s.count(old))
    s = s.replace(old, new)

# ---- item 001: jansson_private_config.h -------------------------------------
LOAD = 'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n'
rep(LOAD, LOAD + 'load("@cc_config//cc_config:probe.bzl", "check_symbol_exists")\n'
    + 'load("@rules_shell//shell:sh_test.bzl", "sh_test")\n')

# configure.ac's AC_CHECK_FUNCS([close getpid ... localeconv ... read
# sched_yield ...]); the rest of that list is already in the catalog.
FUNCS = [("close", "unistd.h"), ("getpid", "unistd.h"), ("localeconv", "locale.h"),
         ("read", "unistd.h"), ("sched_yield", "sched.h")]
probes = "".join('''check_symbol_exists(
    name = "jansson_have_%s",
    define = "HAVE_%s",
    headers = ["%s"],
    symbol = "%s",
)

''' % (f, f.upper(), h, f) for f, h in FUNCS)
rep('config_header(\n    name = "jansson_private_config_h",',
    probes + 'config_header(\n    name = "jansson_private_config_h",')

rep('''    unresolved = [
        "HAVE_CLOSE",
        "HAVE_GETPID",
        "HAVE_LOCALECONV",
        "HAVE_READ",
        "HAVE_SCHED_YIELD",
        "INITIAL_HASHTABLE_ORDER",
        "LT_OBJDIR",
        "PACKAGE_URL",
        "_UINT32_T",
        "_UINT8_T",
        "inline",
        "int32_t",
        "uint16_t",
        "uint32_t",
        "uint8_t",
    ],
''', '''    unresolved = [],
''')
# The probe list sits just above; append to it.
start = s.index('    name = "jansson_private_config_h",')
end = s.index("    unresolved = [],\n", start)
block = s[start:end]
assert block.count("    ],\n") == 1, block
block = block.replace("    ],\n", "".join('        ":jansson_have_%s",\n' % f for f, _ in FUNCS) + "    ],\n")
s = s[:start] + block + s[end:].replace("    unresolved = [],\n", "", 1)

rep('''    }) | {
        "HAVE_ATOMIC_BUILTINS": "1",''', '''    }) | {
        "INITIAL_HASHTABLE_ORDER": "3",  # --enable-initial-hashtable-order default
        "LT_OBJDIR": "\\".libs/\\"",
        "PACKAGE_URL": "\\"\\"",
        # AC_C_INLINE defines `inline` only for a compiler lacking it, and
        # autoconf's fallback typedefs only for a toolchain lacking stdint.h.
        "inline": "",
        "_UINT32_T": "",
        "_UINT8_T": "",
        "int32_t": "",
        "uint16_t": "",
        "uint32_t": "",
        "uint8_t": "",
        "HAVE_ATOMIC_BUILTINS": "1",''')

# ---- item 002: src/jansson_config.h -----------------------------------------
# `var` is not a substitution: it is "@var@" in the template's own comment
# ("replaces @var@ substitutions by values"). config.status leaves an unknown
# @name@ as written, so the faithful value is the token itself.
rep('''    unresolved = [
        "var",
    ],
    values = {
        "json_have_atomic_builtins": "1",''', '''    values = {
        "var": "@var@",  # a comment's example, which config.status leaves alone
        "json_have_atomic_builtins": "1",''')

# ---- libjansson's exports ---------------------------------------------------
# Not an item, and dropped silently: src/Makefile.am links with
#   -export-symbols-regex '^json_|^jansson_' @JSON_SYMVER_LDFLAGS@ @JSON_BSYMBOLIC_LDFLAGS@
# i.e. only the API exported, every symbol versioned by --default-symver
# (glibc), and -Bsymbolic-functions (configure's default "check", which any
# GNU-compatible linker passes). check-exports below is what caught it.
# lld has no --default-symver; a version node NAMED for the soname is what
# that flag produces (jansson_version_str@@libjansson.so.4 in the ground truth).
rep('''cc_shared_library(
    name = "libjansson.la_shared",
''', '''genrule(
    name = "libjansson_ver",
    outs = ["libjansson.ver"],
    cmd = "printf 'libjansson.so.4 {\\n  global:\\n    json_*;\\n    jansson_*;\\n  local:\\n    *;\\n};\\n' > $@",
)

cc_shared_library(
    name = "libjansson.la_shared",
    additional_linker_inputs = [":libjansson_ver"],
    user_link_flags = [
        "-Wl,--version-script=$(location :libjansson_ver)",
        "-Wl,-Bsymbolic-functions",
    ],
''')

# ---- item 003: the TESTS scripts --------------------------------------------
# test/run-suites runs every suite under test/suites against the built
# binaries, from automake's test/ build directory: bin/json_process,
# suites/api/test_*, and ../src/.libs/libjansson.so for check-exports.
# run_suites_test.sh lays that tree out.
#
# scripts/clang-format-check is NOT reproduced: it diffs `git ls-files`
# sources against clang-format, a lint of the source tree that needs a git
# checkout and checks nothing about what the build produces.
api = sorted(set(re.findall(r'cc_binary\(\n    name = "(test_[a-z_]+)"', s)))
assert len(api) == 18, api
s = s.rstrip("\n") + '''

# automake's TESTS (item 003): test/run-suites, in the build layout it expects.
# scripts/clang-format-check is deliberately not reproduced — a formatting
# lint over `git ls-files`, not a check of the build.
sh_test(
    name = "run_suites_test",
    srcs = ["run_suites_test.sh"],
    data = [
        ":json_process",
        ":libjansson.la_shared",
''' + "".join('        ":%s",\n' % t for t in api) + '''    ] + glob(["test/**", "src/jansson.def"]),
    env = {"SKIP_EXIT_CODE": "77"},
)
'''

open(B, "w").write(s)
shutil.copy(os.path.join(HERE, "run_suites_test.sh"), os.path.join(M, "run_suites_test.sh"))
os.chmod(os.path.join(M, "run_suites_test.sh"), 0o755)
for f in glob.glob(os.path.join(M, "needs_attention", "00[123]-*.md")):
    os.remove(f)
