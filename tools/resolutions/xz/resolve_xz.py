#!/usr/bin/env python3
"""Agent-stage resolution for the converted xz module.

    python3 tools/resolutions/xz/resolve_xz.py W/fixtures/xz

All-or-nothing on its anchors. Decisions read from xz 5.4.7's configure.ac
and m4/, never from this host's config.h.
"""
import glob, os, shutil, sys

HERE = os.path.dirname(os.path.abspath(__file__))
M = sys.argv[1]
B = os.path.join(M, "BUILD.bazel")
s = open(B).read()

def rep(old, new):
    global s
    assert s.count(old) == 1, (old[:80], s.count(old))
    s = s.replace(old, new)

# ---- item 001: config.h ----------------------------------------------------
LOAD = 'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n'
rep(LOAD, LOAD + 'load("@cc_config//cc_config:probe.bzl", "check_c_source_compiles", '
    '"check_struct_member", "check_symbol_exists", "check_type_exists")\n')

PROBES = '''# Toolchain facts configure probes, asked of the consumer's compiler.
# AC_C_BIGENDIAN, via the compiler's own byte-order macro.
check_c_source_compiles(
    name = "xz_words_bigendian",
    define = "WORDS_BIGENDIAN",
    source = "int probe[__BYTE_ORDER__ == __ORDER_BIG_ENDIAN__ ? 1 : -1];\\n",
)

# m4/getopt.m4: AC_CHECK_DECL([optreset]) in <getopt.h> (BSD resetting).
check_c_source_compiles(
    name = "xz_have_optreset",
    define = "HAVE_OPTRESET",
    source = "#include <getopt.h>\\nint main(void) {\\n#ifndef optreset\\n  (void) optreset;\\n#endif\\n  return 0;\\n}\\n",
)

# AC_CHECK_FUNCS inside the pthread branch, and tuklib_mbstr.m4's.
check_symbol_exists(
    name = "xz_have_pthread_condattr_setclock",
    define = "HAVE_PTHREAD_CONDATTR_SETCLOCK",
    headers = ["pthread.h"],
    symbol = "pthread_condattr_setclock",
)

check_symbol_exists(
    name = "xz_have_wcwidth",
    define = "HAVE_WCWIDTH",
    defines = ["_GNU_SOURCE"],
    headers = ["wchar.h"],
    symbol = "wcwidth",
)

# AC_HEADER_STDBOOL's AC_CHECK_TYPES([_Bool]).
check_type_exists(
    name = "xz_have__bool",
    define = "HAVE__BOOL",
    type = "_Bool",
)

'''
# AC_CHECK_MEMBERS on struct stat, all five checked independently.
MEMBERS = [
    ("st_atim.tv_nsec", "HAVE_STRUCT_STAT_ST_ATIM_TV_NSEC"),
    ("st_atimespec.tv_nsec", "HAVE_STRUCT_STAT_ST_ATIMESPEC_TV_NSEC"),
    ("st_atimensec", "HAVE_STRUCT_STAT_ST_ATIMENSEC"),
    ("st_uatime", "HAVE_STRUCT_STAT_ST_UATIME"),
    ("st_atim.st__tim.tv_nsec", "HAVE_STRUCT_STAT_ST_ATIM_ST__TIM_TV_NSEC"),
]
for member, define in MEMBERS:
    PROBES += '''check_struct_member(
    name = "xz_%s",
    define = "%s",
    headers = ["sys/stat.h"],
    member = "%s",
    struct = "struct stat",
)

''' % (define.lower(), define, member)
rep('config_header(\n    name = "config_h",', PROBES + 'config_header(\n    name = "config_h",')

probe_labels = ["xz_words_bigendian", "xz_have_optreset", "xz_have_pthread_condattr_setclock",
                "xz_have_wcwidth", "xz_have__bool"] + ["xz_" + d.lower() for _, d in MEMBERS]
rep('''        "@cc_config//catalog:stdc_headers",
    ],
    unresolved = [''', '''        "@cc_config//catalog:stdc_headers",
''' + "".join('        ":%s",\n' % l for l in probe_labels) + '''    ],
    unresolved = [''')

U = ""  # undefined on an autoconf template
VALUES = {
    # Project options, at the defaults the ground truth was built with.
    "ASSUME_RAM": ("128", "--enable-assume-ram default"),
    "ENABLE_NLS": ("1", "--enable-nls default; gettext is in the libc"),
    "HAVE_ICONV": (U, "AM_ICONV left it undefined with the libc's gettext"),
    "HAVE_LZIP_DECODER": ("1", "--enable-lzip-decoder default; the module compiles lzip_decoder.c"),
    "HAVE_SMALL": (U, "--enable-small defaults to no; the module compiles the full sources"),
    # --enable-symbol-versions=auto picks the Linux flavour (2) on a Linux
    # host, and liblzma links with liblzma_linux.map to match (below). A
    # different platform needs a different map, not just this value.
    "HAVE_SYMBOL_VERSIONS_LINUX": ("2", "auto on Linux; matches liblzma_linux.map"),
    "LT_OBJDIR": ('".libs/"', "libtool's"),
    # Only checked when __builtin_bswap16/32/64 are NOT supported, and
    # HAVE___BUILTIN_BSWAPXX is set (tuklib_integer.m4's else branch).
    "HAVE_BSWAP_16": (U, "checked only without __builtin_bswapXX"),
    "HAVE_BSWAP_32": (U, "checked only without __builtin_bswapXX"),
    "HAVE_BSWAP_64": (U, "checked only without __builtin_bswapXX"),
    # Only with --enable-external-sha256 (default no).
    "HAVE_CC_SHA256_CTX": (U, "--enable-external-sha256 defaults to no"),
    "HAVE_CC_SHA256_INIT": (U, "--enable-external-sha256 defaults to no"),
    "HAVE_SHA256INIT": (U, "--enable-external-sha256 defaults to no"),
    "HAVE_SHA256_CTX": (U, "--enable-external-sha256 defaults to no"),
    "HAVE_SHA256_INIT": (U, "--enable-external-sha256 defaults to no"),
    "HAVE_SHA2_CTX": (U, "--enable-external-sha256 defaults to no"),
    # Platforms this module does not target.
    "AC_APPLE_UNIVERSAL_BUILD": (U, "Apple universal builds"),
    "HAVE_CAPSICUM": (U, "FreeBSD sandboxing; --enable-sandbox finds none on Linux"),
    "HAVE_CFLOCALECOPYPREFERREDLANGUAGES": (U, "macOS gettext"),
    "HAVE_CFPREFERENCESCOPYAPPVALUE": (U, "macOS gettext"),
    "MYTHREAD_VISTA": (U, "Windows threads; MYTHREAD_POSIX is set"),
    "MYTHREAD_WIN95": (U, "Windows threads; MYTHREAD_POSIX is set"),
    "__MINGW_USE_VC2005_COMPAT": (U, "MinGW"),
    # tuklib picks exactly ONE method each, and TUKLIB_CPUCORES_SCHED_GETAFFINITY
    # / TUKLIB_PHYSMEM_SYSCONF are already set.
    "TUKLIB_CPUCORES_CPUSET": (U, "one method; sched_getaffinity is set"),
    "TUKLIB_CPUCORES_PSTAT_GETDYNAMIC": (U, "one method; sched_getaffinity is set"),
    "TUKLIB_CPUCORES_SYSCONF": (U, "one method; sched_getaffinity is set"),
    "TUKLIB_CPUCORES_SYSCTL": (U, "one method; sched_getaffinity is set"),
    "TUKLIB_PHYSMEM_AIX": (U, "one method; sysconf is set"),
    "TUKLIB_PHYSMEM_GETINVENT_R": (U, "one method; sysconf is set"),
    "TUKLIB_PHYSMEM_GETSYSINFO": (U, "one method; sysconf is set"),
    "TUKLIB_PHYSMEM_PSTAT_GETSTATIC": (U, "one method; sysconf is set"),
    "TUKLIB_PHYSMEM_SYSCTL": (U, "one method; sysconf is set"),
    "TUKLIB_PHYSMEM_SYSINFO": (U, "one method; sysconf is set"),
    # gnulib getopt replacement: only when <getopt.h> or getopt_long is
    # missing, and the module compiles no lib/getopt.c to go with it.
    "__GETOPT_PREFIX": (U, "no replacement getopt is compiled"),
    # AX_PTHREAD defines it only where the name is missing (AIX).
    "PTHREAD_CREATE_JOINABLE": (U, "AX_PTHREAD fallback for a missing name"),
    # AC_USE_SYSTEM_EXTENSIONS: these are defined unconditionally...
    "_ALL_SOURCE": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "_DARWIN_C_SOURCE": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "_GNU_SOURCE": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "_HPUX_ALT_XOPEN_SOCKET_API": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "_NETBSD_SOURCE": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "_OPENBSD_SOURCE": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "_POSIX_PTHREAD_SEMANTICS": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "_TANDEM_SOURCE": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "__EXTENSIONS__": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "__STDC_WANT_IEC_60559_ATTRIBS_EXT__": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "__STDC_WANT_IEC_60559_BFP_EXT__": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "__STDC_WANT_IEC_60559_DFP_EXT__": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "__STDC_WANT_IEC_60559_EXT__": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "__STDC_WANT_IEC_60559_FUNCS_EXT__": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "__STDC_WANT_IEC_60559_TYPES_EXT__": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "__STDC_WANT_LIB_EXT2__": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    "__STDC_WANT_MATH_SPEC_FUNCS__": ("1", "AC_USE_SYSTEM_EXTENSIONS, unconditional"),
    # ...and these only on Minix, or (_XOPEN_SOURCE) on HP-UX's wchar.h.
    "_MINIX": (U, "AC_USE_SYSTEM_EXTENSIONS, Minix only"),
    "_POSIX_1_SOURCE": (U, "AC_USE_SYSTEM_EXTENSIONS, Minix only"),
    "_POSIX_SOURCE": (U, "AC_USE_SYSTEM_EXTENSIONS, Minix only"),
    "_XOPEN_SOURCE": (U, "AC_USE_SYSTEM_EXTENSIONS, only where wchar.h needs it"),
    # AC_SYS_LARGEFILE: needed only on a 32-bit off_t/time_t; not portable
    # enough to probe here, and undefined is the LP64 answer.
    "_FILE_OFFSET_BITS": (U, "AC_SYS_LARGEFILE, 32-bit only"),
    "_LARGE_FILES": (U, "AC_SYS_LARGEFILE, AIX only"),
    "_TIME_BITS": (U, "AC_SYS_LARGEFILE, 32-bit only"),
}
# autoconf's fallback typedefs, only for a toolchain lacking <stdint.h>.
for n in ("_UINT32_T", "_UINT64_T", "_UINT8_T", "int32_t", "int64_t", "uint16_t",
          "uint32_t", "uint64_t", "uint8_t", "uintptr_t"):
    VALUES[n] = (U, "fallback typedef")

import re
start = s.index('    name = "config_h",')
m = re.compile(r"    unresolved = \[\n(.*?)    \],\n", re.S).search(s, start)
unresolved = re.findall(r'"([A-Za-z_0-9]+)"', m.group(1))
probed = {d for _, d in MEMBERS} | {"WORDS_BIGENDIAN", "HAVE_OPTRESET",
          "HAVE_PTHREAD_CONDATTR_SETCLOCK", "HAVE_WCWIDTH", "HAVE__BOOL"}
missing = [n for n in unresolved if n not in VALUES and n not in probed]
extra = [n for n in list(VALUES) + list(probed) if n not in unresolved]
assert not missing and not extra, (missing, extra)
s = s[:m.start()] + s[m.end():]

def sl(v):
    return '"' + v.replace("\\", "\\\\").replace('"', '\\"') + '"'
entries = "".join("        %s: %s,  # %s\n" % (sl(k), sl(v), why) for k, (v, why) in sorted(VALUES.items()))
rep('''    }) | {
        "HAVE_CHECK_CRC32": "1",''', '''    }) | {
''' + entries + '''        "HAVE_CHECK_CRC32": "1",''')

# ---- liblzma's version script ----------------------------------------------
# Not an item, and the translator dropped it silently: src/liblzma/Makefile.am
# adds -Wl,--version-script=liblzma_linux.map under COND_SYMVERS_LINUX, the
# branch HAVE_SYMBOL_VERSIONS_LINUX=2 above takes. Without it liblzma exports
# every global (211 symbols, unversioned) where the original exports its
# API at XZ_5.0/XZ_5.2/... (114).
rep('''    shared_lib_name = "liblzma.so.5",
''', '''    additional_linker_inputs = ["src/liblzma/liblzma_linux.map"],
    shared_lib_name = "liblzma.so.5",
    user_link_flags = ["-Wl,--version-script=$(location src/liblzma/liblzma_linux.map)"],
''')

# libtool compiles a shared library's objects with -DPIC, and common.h turns
# the XZ_5.2.2/XZ_5.1.2alpha compat versions (lzma_get_progress@XZ_5.2.2 and
# six more) off without it. liblzma is shared-only here, as in the ground
# truth, so every object libtool built for it had PIC defined.
start = s.index('cc_library(\n    name = "liblzma.la",')
end = s.index("\n)\n", start)
head, s, tail = s[:start], s[start:end], s[end:]
rep('''    local_defines = [
        "HAVE_CONFIG_H",
''', '''    local_defines = [
        "HAVE_CONFIG_H",
        "PIC",  # libtool's, for a shared library's objects
''')
s = head + s + tail

# ---- the binary tests' working directory ------------------------------------
# Not an item: the translator runs every automake test from the module root,
# but automake runs it from its Makefile.am's directory, and tuktest opens
# files/... relative to that. All twelve are tests/Makefile.am's.
n = 0
for t in re.findall(r'name = "(test_[a-z_]+)_test",\n    srcs = \["run_registered_test.sh"\]', s):
    rep('         "%s",\n         ".",\n' % t, '         "%s",\n         "tests",\n' % t)
    n += 1
assert n == 12, n

# ---- item 002: the script tests ---------------------------------------------
# xzdiff and xzgrep are AC_CONFIG_FILES outputs (configure.ac, `chmod +x`),
# which the module did not reproduce; test_scripts.sh runs them. Generated
# here with configure's substitutions: POSIX_SHELL as gl_POSIX_SHELL finds it
# on a POSIX system, and no enable_path_for_scripts (set only on Solaris).
SUBST = {
    "POSIX_SHELL": "/bin/sh",
    "enable_path_for_scripts": "",
    "xz": "xz",
    "PACKAGE_NAME": "XZ Utils",
    "VERSION": "5.4.7",
    "PACKAGE_BUGREPORT": "xz@tukaani.org",
}
sed = " ".join("-e 's|@%s@|%s|g'" % (k, v) for k, v in SUBST.items())
SCRIPTS = ["xzdiff", "xzgrep", "xzless", "xzmore"]
TESTS = [
    "test_files.sh", "test_suffix.sh", "test_compress_prepared_bcj_sparc",
    "test_compress_prepared_bcj_x86", "test_compress_generated_abc",
    "test_compress_generated_random", "test_compress_generated_text",
    "test_scripts.sh",
]
block = '''
# xz's configure-generated scripts (AC_CONFIG_FILES), installed beside xz.
''' + "".join('''genrule(
    name = "%s_script",
    srcs = ["src/scripts/%s.in"],
    outs = ["src/scripts/%s"],
    cmd = "sed %s $< > $@ && chmod +x $@",
    visibility = ["//visibility:public"],
)

''' % (n, n, n, sed.replace('"', '\\"'), ) for n in SCRIPTS) + '''# automake's TESTS scripts (item 002), each run by run_script_test.sh in the
# build-tree layout it expects: ../src/xz/xz, ../src/xzdec/xzdec,
# ../src/scripts/, ../config.h and ./create_compress_files, with $srcdir the
# module's tests/.
''' + "".join('''sh_test(
    name = "%s_test",
    srcs = ["run_script_test.sh"],
    args = ["%s"],
    data = [
        ":config_h",
        ":create_compress_files",
        ":xz",
        ":xzdec",
        ":xzdiff_script",
        ":xzgrep_script",
    ] + glob(["tests/**"]),
    env = {"SKIP_EXIT_CODE": "77"},
)

''' % (t.replace(".sh", "").replace(".", "_"), t) for t in TESTS)
s = s.rstrip("\n") + "\n" + block.rstrip("\n") + "\n"

open(B, "w").write(s)
shutil.copy(os.path.join(HERE, "run_script_test.sh"), os.path.join(M, "run_script_test.sh"))
os.chmod(os.path.join(M, "run_script_test.sh"), 0o755)
for f in glob.glob(os.path.join(M, "needs_attention", "00[12]-*.md")):
    os.remove(f)
