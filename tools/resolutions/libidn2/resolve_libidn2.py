#!/usr/bin/env python3
"""Agent-stage resolution for the converted libidn2 module.

    python3 tools/resolutions/libidn2/resolve_libidn2.py W/fixtures/libidn2

All-or-nothing on its anchors. config.h decisions are read from libidn2
2.3.7's configure.ac and gnulib's m4; a scratch `./configure` on the
conversion host was used as EVIDENCE of what each check answers there,
never copied.
"""
import glob, os, re, sys

M = sys.argv[1]
B = os.path.join(M, "BUILD.bazel")
s = open(B).read()

def rep(old, new, text=None):
    global s
    t = s if text is None else text
    assert t.count(old) == 1, (old[:80], t.count(old))
    t = t.replace(old, new)
    if text is None:
        s = t
    return t

def sl(v):
    return '"' + v.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'

# ---- item 004: three gnulib headers at the wrong path -----------------------
# The rules exist, but write gl/stat.h, gl/types.h and unistring/types.h;
# gl/Makefile.am's targets are sys/stat.h and sys/types.h, and sources reach
# them as <sys/stat.h> through `includes = ["gl"]`. So the item's "no rule"
# is the translator dropping the sys/ directory from the output path.
for name, old, new in [("gl_stat_h", "gl/stat.h", "gl/sys/stat.h"),
                       ("gl_types_h", "gl/types.h", "gl/sys/types.h"),
                       ("unistring_types_h", "unistring/types.h", "unistring/sys/types.h")]:
    rep('    name = "%s",\n    output = "%s",\n' % (name, old),
        '    name = "%s",\n    output = "%s",  # make writes sys/, see item 004\n' % (name, new))

# ---- item 001: config.h -----------------------------------------------------
LOAD = 'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n'
rep(LOAD, LOAD + 'load("@cc_config//cc_config:probe.bzl", "check_c_source_compiles", '
    '"check_struct_member", "check_symbol_exists")\n')

rules, labels = [], []
def probe(kind, name, **attrs):
    body = "".join("    %s = %s,\n" % (k, sl(v) if isinstance(v, str) else
                   "[" + ", ".join(sl(x) for x in v) + "]") for k, v in sorted(attrs.items()))
    rules.append("%s(\n    name = %s,\n%s)\n" % (kind, sl(name), body))
    labels.append(":" + name)

# gl_C_BOOL and gl_ASSERT_H: whether the C2023 keywords work, with gnulib's
# own snippets (both false under C17, true under C23).
probe("check_c_source_compiles", "idn2_have_c_bool", define="HAVE_C_BOOL",
      source="#if true == false\n #error \"true == false\"\n#endif\nextern bool b;\nbool b = true == false;\n")
probe("check_c_source_compiles", "idn2_have_c_static_assert", define="HAVE_C_STATIC_ASSERT",
      source="#if defined __clang__ && __STDC_VERSION__ < 202311\n #pragma clang diagnostic error \"-Wc2x-extensions\"\n #pragma clang diagnostic error \"-Wc++1z-extensions\"\n#endif\nstatic_assert (2 + 2 == 4, \"arithmetic does not work\");\nstatic_assert (2 + 2 == 4);\nint main(void) {\n  static_assert (sizeof (char) == 1, \"sizeof does not work\");\n  static_assert (sizeof (char) == 1);\n  return 0;\n}\n")
# AC_CHECK_DECLS([getdtablesize]); undefined reads as 0 in its #if.
probe("check_symbol_exists", "idn2_have_decl_getdtablesize", define="HAVE_DECL_GETDTABLESIZE",
      symbol="getdtablesize", headers=["unistd.h"], defines=["_GNU_SOURCE"])
# gnulib's fcntl-o checks run O_NOATIME/O_NOFOLLOW; a compile probe asks
# whether the flag exists, which is the part that varies by platform.
probe("check_symbol_exists", "idn2_have_working_o_noatime", define="HAVE_WORKING_O_NOATIME",
      symbol="O_NOATIME", headers=["fcntl.h"], defines=["_GNU_SOURCE"])
probe("check_symbol_exists", "idn2_have_working_o_nofollow", define="HAVE_WORKING_O_NOFOLLOW",
      symbol="O_NOFOLLOW", headers=["fcntl.h"], defines=["_GNU_SOURCE"])
# gnulib stat-time / stat-birthtime: AC_CHECK_MEMBERS on struct stat.
for member in ["st_atim.tv_nsec", "st_atimespec.tv_nsec", "st_atimensec", "st_atim.st__tim.tv_nsec",
               "st_birthtimespec.tv_nsec", "st_birthtimensec", "st_birthtim.tv_nsec"]:
    define = "HAVE_STRUCT_STAT_" + member.upper().replace(".", "_")
    probe("check_struct_member", "idn2_" + define.lower(), define=define,
          struct="struct stat", member=member, headers=["sys/types.h", "sys/stat.h"])

rep('config_header(\n    name = "config_h",',
    "# Toolchain facts config.h needs, asked of the consumer's compiler.\n"
    + "\n".join(rules) + '\nconfig_header(\n    name = "config_h",')

U = ""
VALUES = {
    "ENABLE_NLS": ("1", "--enable-nls default; gettext is in the libc"),
    "LT_OBJDIR": ('".libs/"', "libtool's"),
    "PACKAGE_PACKAGER": (U, "--with-packager not given"),
    "PACKAGE_PACKAGER_BUG_REPORTS": (U, "--with-packager-bug-reports not given"),
    "PACKAGE_PACKAGER_VERSION": (U, "--with-packager-version not given"),
    # gnulib module indicators: which modules gnulib-tool imported, i.e.
    # facts about the project's source tree.
    "GNULIB_FSCANF": ("1", "gnulib module indicator"),
    "GNULIB_MSVC_NOTHROW": ("1", "gnulib module indicator"),
    "GNULIB_SCANF": ("1", "gnulib module indicator"),
    "GNULIB_STRERROR": ("1", "gnulib module indicator"),
    "GNULIB_UNISTR_U32_MBTOUC_UNSAFE": ("1", "gnulib module indicator"),
    "GNULIB_UNISTR_U32_UCTOMB": ("1", "gnulib module indicator"),
    "GNULIB_UNISTR_U8_MBTOUC": ("1", "gnulib module indicator"),
    "GNULIB_UNISTR_U8_MBTOUCR": ("1", "gnulib module indicator"),
    "GNULIB_UNISTR_U8_MBTOUC_UNSAFE": ("1", "gnulib module indicator"),
    "GNULIB_UNISTR_U8_UCTOMB": ("1", "gnulib module indicator"),
    "GNULIB_PRINTF_ATTRIBUTE_FLAVOR_GNU": (U, "only with gnulib's printf replacement"),
    # gnulib's stdint.h replacement is not generated (the libc's is used),
    # so none of its BITSIZEOF_/_SUFFIX/HAVE_SIGNED_ computations run.
    **{n: (U, "only for gnulib's stdint.h replacement") for n in [
        "BITSIZEOF_PTRDIFF_T", "BITSIZEOF_SIG_ATOMIC_T", "BITSIZEOF_SIZE_T",
        "BITSIZEOF_WCHAR_T", "BITSIZEOF_WINT_T", "PTRDIFF_T_SUFFIX",
        "SIG_ATOMIC_T_SUFFIX", "SIZE_T_SUFFIX", "WCHAR_T_SUFFIX", "WINT_T_SUFFIX",
        "HAVE_SIGNED_SIG_ATOMIC_T", "HAVE_SIGNED_WCHAR_T", "HAVE_SIGNED_WINT_T",
        "__STDC_CONSTANT_MACROS", "__STDC_LIMIT_MACROS"]},
    # gnulib workarounds for bugs in OTHER libcs, each set by a check that
    # passes on glibc and musl alike. Run checks, so not compile-probeable.
    "C_ALLOCA": (U, "only without a working alloca"),
    "STACK_DIRECTION": (U, "only with C_ALLOCA"),
    "DOUBLE_SLASH_IS_DISTINCT_ROOT": (U, "Cygwin and z/OS"),
    "FCNTL_DUPFD_BUGGY": (U, "workaround for a libc fcntl bug"),
    "OPEN_TRAILING_SLASH_BUG": (U, "workaround for a libc open bug"),
    "REPLACE_FUNC_STAT_FILE": (U, "workaround for a libc stat bug"),
    "REPLACE_STRERROR_0": (U, "workaround for a libc strerror(0) bug"),
    "STAT_MACROS_BROKEN": (U, "workaround for broken S_IS* macros"),
    "MALLOC_0_IS_NONNULL": ("1", "run check; glibc and musl both return non-NULL"),
    "PROMOTED_MODE_T": ("mode_t", "mode_t is not narrower than int"),
    # Checks configure did not run, because an earlier one settled it.
    "HAVE_DECL_GETC_UNLOCKED": (U, "check not run"),
    "HAVE_FLOCKFILE": (U, "check not run"),
    "HAVE_FUNLOCKFILE": (U, "check not run"),
    "HAVE_VAR___PROGNAME": (U, "checked only without program_invocation_short_name"),
    "__GETOPT_PREFIX": (U, "no replacement getopt is compiled"),
    # iconv.m4: glibc's iconv takes a non-const char **, so ICONV_CONST is
    # defined EMPTY. "" would mean undefined, which breaks the iconv calls,
    # so a single space stands in for the empty definition.
    "ICONV_CONST": (" ", "defined, to nothing"),
    "ICONV_FLAVOR": (U, "only for a non-GNU iconv"),
    "restrict": ("__restrict__", "AC_C_RESTRICT's spelling for GCC and Clang"),
    # Platforms this module does not target.
    "HAVE_CFLOCALECOPYCURRENT": (U, "macOS"),
    "HAVE_CFPREFERENCESCOPYAPPVALUE": (U, "macOS"),
    "HAVE_GETEXECNAME": (U, "Solaris"),
    "HAVE_SETDTABLESIZE": (U, "Haiku/OS2"),
    "HAVE__SET_INVALID_PARAMETER_HANDLER": (U, "MSVC"),
    "HAVE___HEADER_INLINE": (U, "Apple headers"),
    "__MINGW_USE_VC2005_COMPAT": (U, "MinGW"),
    # gl_MUSL_LIBC: set from the host TRIPLE (*-musl*), not a probe. This
    # module answers for glibc; a musl consumer needs "1".
    "MUSL_LIBC": (U, "host triple is not *-musl"),
    # AC_USE_SYSTEM_EXTENSIONS (autoconf 2.72): unconditional...
    **{n: ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional") for n in [
        "_ALL_SOURCE", "_DARWIN_C_SOURCE", "_GNU_SOURCE", "_HPUX_ALT_XOPEN_SOCKET_API",
        "_NETBSD_SOURCE", "_OPENBSD_SOURCE", "_POSIX_PTHREAD_SEMANTICS", "_TANDEM_SOURCE",
        "__EXTENSIONS__", "__STDC_WANT_IEC_60559_ATTRIBS_EXT__",
        "__STDC_WANT_IEC_60559_BFP_EXT__", "__STDC_WANT_IEC_60559_DFP_EXT__",
        "__STDC_WANT_IEC_60559_EXT__", "__STDC_WANT_IEC_60559_FUNCS_EXT__",
        "__STDC_WANT_IEC_60559_TYPES_EXT__", "__STDC_WANT_LIB_EXT2__",
        "__STDC_WANT_MATH_SPEC_FUNCS__", "__USE_MINGW_ANSI_STDIO"]},
    # ...and these only on Minix, HP-UX's wchar.h, or a pre-C11 libc.
    "_MINIX": (U, "Minix only"),
    "_POSIX_1_SOURCE": (U, "Minix only"),
    "_POSIX_SOURCE": (U, "Minix only"),
    "_XOPEN_SOURCE": (U, "only where wchar.h needs it"),
    "_ISOC11_SOURCE": (U, "only where the libc hides C11 by default"),
    "_FILE_OFFSET_BITS": (U, "AC_SYS_LARGEFILE, 32-bit only"),
    "_LARGE_FILES": (U, "AC_SYS_LARGEFILE, AIX only"),
    "_TIME_BITS": (U, "AC_SYS_LARGEFILE, 32-bit only"),
    "__STDC_NO_VLA__": (U, "compiler-predefined, never configure's"),
    # autoconf's fallbacks, only for a toolchain lacking the real thing.
    **{n: (U, "fallback definition") for n in [
        "inline", "mode_t", "nlink_t", "pid_t", "size_t", "ssize_t", "va_copy"]},
}

start = s.index('    name = "config_h",')
m = re.compile(r"    unresolved = \[\n(.*?)    \],\n", re.S).search(s, start)
unresolved = re.findall(r'"([A-Za-z_0-9]+)"', m.group(1))
probed = {re.search(r'define = "([A-Z_0-9]+)"', r).group(1) for r in rules}
missing = [n for n in unresolved if n not in VALUES and n not in probed]
extra = [n for n in list(VALUES) + list(probed) if n not in unresolved]
assert not missing and not extra, (missing, extra)
s = s[:m.start()] + s[m.end():]

end = s.index("\n)\n", start)
block = s[start:end]
i = block.index("    probes = [\n")
j = block.index("    ],\n", i)
block = block[:j] + "".join('        "%s",\n' % l for l in labels) + block[j:]
entries = "".join("        %s: %s,  # %s\n" % (sl(k), sl(v), why) for k, (v, why) in sorted(VALUES.items()))
block = rep("    values = {\n", "    values = {\n" + entries, block)
s = s[:start] + block + s[end:]

# ---- item 002: libgnu.la and libunistring.la in libidn2.so ------------------
# Both are noinst_LTLIBRARIES convenience archives that lib/Makefile.am links
# into libidn2.la, and that the idn2 tool and the tests ALSO link directly —
# so folding them in would be wrong. Named in the cc_shared_library's deps:
# absorbed into libidn2.so, where Bazel then satisfies the other targets'
# references too. Two recorded differences from the original: Bazel absorbs
# the WHOLE archive where libtool pulls only the referenced members, and the
# tests resolve libgnu/libunistring symbols from libidn2.so rather than from
# their own static copy (the original's libidn2.map would hide them; the
# version script is not carried — see the exports bead).
rep('''    name = "libidn2.la_shared",
    deps = [
        ":libidn2.la",
    ],
''', '''    name = "libidn2.la_shared",
    deps = [
        ":libgnu.la",
        ":libidn2.la",
        ":libunistring.la",
    ],
''')

# ---- SRCDIR: an absolute conversion-host path -------------------------------
# Not an item. lib/ and tests/ define -DSRCDIR=\"$(srcdir)\" and fuzz/
# -DSRCDIR=\"$(abs_srcdir)\"; the translator froze the conversion SANDBOX's
# absolute path, which no longer exists, so test-IdnaTest-txt cannot open
# IdnaTest.txt — and the three fuzzers, finding no corpus directory, passed
# having replayed nothing. Rewritten relative to the module root, the
# directory run_registered_test.sh runs every test from.
n = 0
for m in re.finditer(r"""SRCDIR='\\"(/[^"']*?/(lib|tests|fuzz))\\"'""", s):
    n += 1
s, k = re.subn(r"""SRCDIR='\\"/[^"']*?/(lib|tests|fuzz)\\"'""", r"""SRCDIR='\\"\1\\"'""", s)
assert k == n and k > 0, (k, n)
assert "/home/" not in s and "sandbox/" not in s

# ---- item 003: tests/test-idn2.sh -------------------------------------------
# It runs $IDN2 (default ../src/idn2) from the tests/ build directory and
# writes temporaries there; the runner points IDN2 at the built tool.
s = s.rstrip("\n") + '''

# automake's TESTS script (item 003), with the tool it drives.
sh_test(
    name = "test-idn2_test",
    srcs = ["run_test_idn2.sh"],
    data = [
        ":idn2",
        "tests/test-idn2.sh",
    ],
    env = {"SKIP_EXIT_CODE": "77"},
)
'''
open(os.path.join(M, "run_test_idn2.sh"), "w").write('''#!/usr/bin/env bash
# Runs tests/test-idn2.sh as automake's driver does: from a writable tests/
# directory, with srcdir set, and IDN2 naming the built tool (its default,
# ../src/idn2, is a build-tree path). Not `set -e`: the script's status is
# the result.
set -uo pipefail
module="$(cd "$(dirname "$0")" && pwd)"
work="${TEST_TMPDIR:-$(mktemp -d)}/tests"
mkdir -p "${work}"
srcdir="${module}/tests"
IDN2="${module}/idn2"
export srcdir IDN2
cd "${work}"
sh "${srcdir}/test-idn2.sh"
exit_code=$?
if [[ -n "${SKIP_EXIT_CODE:-}" && "${exit_code}" == "${SKIP_EXIT_CODE}" ]]; then
  echo "SKIP: exited ${exit_code}, which the project's own test harness reports as skipped"
  exit 0
fi
exit "${exit_code}"
''')
os.chmod(os.path.join(M, "run_test_idn2.sh"), 0o755)

open(B, "w").write(s)
for f in glob.glob(os.path.join(M, "needs_attention", "00[1234]-*.md")):
    os.remove(f)
