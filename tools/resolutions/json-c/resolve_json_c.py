#!/usr/bin/env python3
"""Agent-stage resolution for the converted json-c module.

    python3 tools/resolutions/json-c/resolve_json_c.py W/fixtures/json-c

All-or-nothing on its anchors. Read the module's project_notes/ first: both
notes decide something here (apps_config.h, and how the tests run).
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

LOAD = 'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n'
rep(LOAD, LOAD + 'load("@cc_config//cc_config:probe.bzl", "check_include_file")\n'
    + 'load("@rules_shell//shell:sh_test.bzl", "sh_test")\n')

# ---- items 002/003: JSON_C_HAVE_{INTTYPES,STDINT}_H ---------------------------
# CMakeLists.txt sets each from the standard check_include_file result
# (`if (HAVE_INTTYPES_H) set(JSON_C_HAVE_INTTYPES_H 1)`), so the input states
# the alias; probed here under json-c's own names.
rep('config_header(\n    name = "config_h",', '''# json-c's prefixed copies of two header checks (CMakeLists.txt:178-183).
check_include_file(
    name = "json_c_have_inttypes_h",
    define = "JSON_C_HAVE_INTTYPES_H",
    header = "inttypes.h",
)

check_include_file(
    name = "json_c_have_stdint_h",
    define = "JSON_C_HAVE_STDINT_H",
    header = "stdint.h",
)

config_header(
    name = "config_h",''')

rep('''        "@cc_config//catalog:stdc_headers",
    ],
    unresolved = [
        "JSON_C_HAVE_INTTYPES_H",
        "SPEC___THREAD",
        "const",
        "size_t",
        "json_c_strtoll",
        "json_c_strtoull",
        "PROJECT_NAME",
        "JSON_C_BUGREPORT",
        "CPACK_PACKAGE_VERSION_MAJOR",
        "CPACK_PACKAGE_VERSION_MINOR",
        "CPACK_PACKAGE_VERSION_PATCH",
    ],
    values = select({''', '''        "@cc_config//catalog:stdc_headers",
        ":json_c_have_inttypes_h",
    ],
    values = {
        # HAVE___THREAD is set, so CMakeLists.txt:327 picks __thread.
        "SPEC___THREAD": "__thread",
        # Fallbacks for a non-ANSI compiler; "" is undefined on a CMake template.
        "const": "",
        "size_t": "",
        # Set unconditionally (CMakeLists.txt:276), replaced only without strtoll.
        "json_c_strtoll": "strtoll",
        "json_c_strtoull": "strtoull",
        # project(json-c VERSION 0.19): no patch component, so it is empty.
        "PROJECT_NAME": "json-c",
        "JSON_C_BUGREPORT": "json-c@googlegroups.com",
        "CPACK_PACKAGE_VERSION_MAJOR": "0",
        "CPACK_PACKAGE_VERSION_MINOR": "19",
        "CPACK_PACKAGE_VERSION_PATCH": "",
    } | select({''')

rep('''    template = "cmake/json_config.h.in",
    unresolved = [
        "JSON_C_HAVE_INTTYPES_H",
        "JSON_C_HAVE_STDINT_H",
    ],
''', '''    template = "cmake/json_config.h.in",
    probes = [
        ":json_c_have_inttypes_h",
        ":json_c_have_stdint_h",
    ],
''')

# ---- item 004: json.h -------------------------------------------------------
# DISABLE_JSON_POINTER / DISABLE_JSON_PATCH default OFF, so both includes
# are written (CMakeLists.txt:505-513), and the module compiles both sources.
rep('''    template = "json.h.cmakein",
    unresolved = [
        "JSON_H_JSON_PATCH",
        "JSON_H_JSON_POINTER",
    ],
''', '''    template = "json.h.cmakein",
    values = {
        "JSON_H_JSON_PATCH": "#include \\"json_patch.h\\"",
        "JSON_H_JSON_POINTER": "#include \\"json_pointer.h\\"",
    },
''')

# ---- item 005: apps_config.h ------------------------------------------------
# project_notes/001: apps/CMakeLists.txt's in-tree branch writes
# `set(HAVE_JSON_TOKENER_GET_PARSE_END)` with no value, which UNSETS it. The
# project's own build leaves it undefined, so this does too.
rep('''    unresolved = [
        "HAVE_JSON_TOKENER_GET_PARSE_END",
    ],
''', '''    values = {
        "HAVE_JSON_TOKENER_GET_PARSE_END": "",  # project_notes/001: set() with no value unsets it
    },
''')

# ---- item 001: distcheck ----------------------------------------------------
# A UTILITY target that runs `make package_source`, unpacks the tarball and
# builds and tests it with CMake: it checks the source PACKAGING, which this
# module does not reproduce. Nothing consumes it. Nothing emitted.
rep('''config_header(
    name = "json_h",''', '''# CMake's `distcheck` UTILITY target (needs_attention 001) is deliberately
# not reproduced: it checks the source-package round trip through CMake,
# not anything the build produces.
config_header(
    name = "json_h",''')

# ---- item 006: the 28 CTest wrappers ----------------------------------------
# Run AS WRITTEN, in a staged build tree: test-defs.sh resolves
# $top_builddir and appends /tests, so the binaries are linked into
# <build>/tests/ and json_parse into <build>/apps/, which is what
# project_notes/002 found no Bazel layout could bridge directly.
#
# test_json_parse_cli is NOT reproduced: it pipes `xxd -r -p` output into
# json_parse, and xxd is not part of this toolchain — upstream's own ctest
# fails it on the conversion host for the same reason.
tests = re.findall(r"^- `([^`]+)` runs", open(glob.glob(os.path.join(M, "needs_attention", "006-*.md"))[0]).read(), re.M)
assert len(tests) == 28, tests
binaries = sorted(set(re.findall(r'cc_binary\(\n    name = "([^"]+)"', s)) - {"json_parse"})
s = s.rstrip("\n") + '''

# CTest's registered tests (needs_attention 006), each its own wrapper run
# by run_ctest_wrapper.sh. test_json_parse_cli is deliberately absent: it
# needs xxd, which upstream's ctest also lacks on the conversion host.
'''
for t in tests:
    if t == "test_json_parse_cli":
        continue
    s += '''sh_test(
    name = "%s_wrapper_test",
    srcs = ["run_ctest_wrapper.sh"],
    args = ["%s"],
    data = [
        ":json_parse",
%s    ] + glob(["tests/**"]),
)

''' % (t, t, "".join('        ":%s",\n' % b for b in binaries))
s = s.rstrip("\n") + "\n"

open(B, "w").write(s)
shutil.copy(os.path.join(HERE, "run_ctest_wrapper.sh"), os.path.join(M, "run_ctest_wrapper.sh"))
os.chmod(os.path.join(M, "run_ctest_wrapper.sh"), 0o755)
for f in glob.glob(os.path.join(M, "needs_attention", "00[123456]-*.md")):
    os.remove(f)
