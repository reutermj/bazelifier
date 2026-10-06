#!/usr/bin/env python3
"""Agent-stage resolution for the converted PRRTE module (bzl-7r9.6).

Applied to a FRESH unpack, after its dependencies' own resolutions:
  python3 tools/resolutions/pmix/deps/resolve_hwloc.py    W/fixtures/hwloc
  python3 tools/resolutions/pmix/deps/resolve_libevent.py W/fixtures/libevent
  python3 tools/resolutions/pmix/resolve_pmix.py          W/fixtures/pmix
  python3 tools/resolutions/prrte/resolve_prrte.py        W/fixtures/prrte
All-or-nothing on its anchors. Modelled on resolve_pmix.py: PRRTE carries
the same oac/MCA shape under a PRTE_ prefix (its m4 differs from PMIx's by
two extra attributes and the vendor dispatch; diffed 2026-10-06).
"""
import os, re, sys

M = sys.argv[1]
B = os.path.join(M, "BUILD.bazel")
s = open(B).read()

def rep(old, new, count=1):
    global s
    assert s.count(old) == count, (old[:80], s.count(old))
    s = s.replace(old, new)

def sl(v):
    return '"' + v.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t") + '"'

# ---- 1. loads ---------------------------------------------------------------
rep('load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n',
    'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n'
    'load("@cc_config//cc_config:probe.bzl", "check_c_source_compiles", "check_symbol_exists")\n')

# ---- 2. probes PRRTE runs with its OWN snippets (item 001) ------------------
# From config/prte_check_attributes.m4, prte_setup_cc.m4, prte_config_pthreads.m4
# and prte_check_ptrace.m4, so the consumer's compiler is asked what configure
# asked rather than handed this host's answer.
ATTRS = {
    "aligned": "struct foo { char text[4]; }  __attribute__ ((__aligned__(8)));",
    "always_inline": "int foo (int arg) __attribute__ ((__always_inline__));",
    "cold": "int foo(int arg1, int arg2) __attribute__ ((__cold__));\nint foo(int arg1, int arg2) { return arg1 * arg2 + arg1; }",
    "const": "int foo(int arg1, int arg2) __attribute__ ((__const__));\nint foo(int arg1, int arg2) { return arg1 * arg2 + arg1; }",
    "deprecated": "int foo(int arg1, int arg2) __attribute__ ((__deprecated__));\nint foo(int arg1, int arg2) { return arg1 * arg2 + arg1; }",
    "deprecated_argument": 'int foo(int arg1, int arg2) __attribute__ ((__deprecated__("compiler allows argument")));\nint foo(int arg1, int arg2) { return arg1 * arg2 + arg1; }',
    "format": "int this_printf (void *my_object, const char *my_format, ...) __attribute__ ((__format__ (__printf__, 2, 3)));",
    "format_funcptr": "int (*this_printf)(void *my_object, const char *my_format, ...) __attribute__ ((__format__ (__printf__, 2, 3)));",
    "hot": "int foo(int arg1, int arg2) __attribute__ ((__hot__));\nint foo(int arg1, int arg2) { return arg1 * arg2 + arg1; }",
    "malloc": "#include <stdlib.h>\nint * foo(int arg1) __attribute__ ((__malloc__));\nint * foo(int arg1) { return (int*) malloc(arg1); }",
    "may_alias": "int * p_value __attribute__ ((__may_alias__));",
    "no_instrument_function": "int * foo(int arg1) __attribute__ ((__no_instrument_function__));",
    "noinline": "int foo(int arg1, int arg2) __attribute__ ((__noinline__));\nint foo(int arg1, int arg2) { return arg1 * arg2 + arg1; }\nstatic int bar(int arg1, int arg2) __attribute__ ((__noinline__));\nstatic int bar(int arg1, int arg2) { return arg1 * arg2 + arg1; }",
    "nonnull": "int square(int *arg) __attribute__ ((__nonnull__));\nint square(int *arg) { return *arg; }",
    "noreturn": "#include <unistd.h>\n#include <stdlib.h>\nvoid fatal(int arg1) __attribute__ ((__noreturn__));\nvoid fatal(int arg1) { exit(arg1); }",
    "noreturn_funcptr": "#include <unistd.h>\n#include <stdlib.h>\nextern void (*fatal_exit)(int arg1) __attribute__ ((__noreturn__));\nvoid fatal(int arg1) { fatal_exit (arg1); }",
    "packed": "struct foo {\n    char a;\n    int x[2] __attribute__ ((__packed__));\n};",
    "pure": "int square(int arg) __attribute__ ((__pure__));\nint square(int arg) { return arg * arg; }",
    "sentinel": "int my_execlp(const char * file, const char *arg, ...) __attribute__ ((__sentinel__));",
    "unused": "int square(int arg1 __attribute__ ((__unused__)), int arg2);\nint square(int arg1, int arg2) { return arg2; }",
    "visibility": 'int square(int arg1) __attribute__ ((__visibility__("hidden")));',
    "warn_unused_result": "int foo(int arg) __attribute__ ((__warn_unused_result__));\nint foo(int arg) { return arg + 3; }",
    "weak_alias": 'int foo(int arg);\nint foo(int arg) { return arg + 3; }\nint foo2(int arg) __attribute__ ((__weak__, __alias__("foo")));',
    "destructor": "void foo(void) __attribute__ ((__destructor__));\nvoid foo(void) { return ; }",
    "optnone": "void __attribute__ ((__optnone__)) foo(void);\nvoid foo(void) { return ; }",
    "extension": "int i = __extension__ 3;",
}
probes = [("prte_have_attribute", "PRTE_HAVE_ATTRIBUTE",
           "#include <stdlib.h>\nint main(void) {\n  struct foo {\n      char a;\n      int x[2] __attribute__ ((__packed__));\n   };\n  return 0;\n}\n", False, [])]
for name, src in ATTRS.items():
    probes.append(("prte_have_attribute_" + name, "PRTE_HAVE_ATTRIBUTE_" + name.upper(), src + "\n", False, []))

def prog(prologue, body):
    return prologue + ("\n" if prologue else "") + "int main(void) {\n" + body + "\n  ;\n  return 0;\n}\n"

PTRACE_BOTH = ("#include <sys/ptrace.h>\n"
               "#if !(defined(PTRACE_TRACEME) || defined(PT_TRACE_ME)) || !(defined(PTRACE_DETACH) || defined(PT_DETACH))\n"
               "#error no stop-on-exec\n#endif\n")
probes += [
    ("prte_c_have___thread", "PRTE_C_HAVE___THREAD", prog("", "static __thread int  foo = 1;++foo;"), False, []),
    ("prte_c_have__thread_local", "PRTE_C_HAVE__THREAD_LOCAL", prog("", "static _Thread_local int  foo = 1;++foo;"), False, []),
    ("prte_c_have_atomic_conv_var", "PRTE_C_HAVE_ATOMIC_CONV_VAR", prog("#include <stdatomic.h>", "static atomic_long foo = 1;++foo;"), False, []),
    ("prte_c_have__atomic", "PRTE_C_HAVE__ATOMIC", prog("#include <stdatomic.h>", "static _Atomic long foo = 1;++foo;"), False, []),
    ("prte_have_clang_builtin_atomic_c11_func", "PRTE_HAVE_CLANG_BUILTIN_ATOMIC_C11_FUNC", prog("#include <stdatomic.h>", "atomic_int acnt = 0; __c11_atomic_fetch_add(&acnt, 1, memory_order_relaxed);"), False, []),
    ("prte_c_have__generic", "PRTE_C_HAVE__GENERIC", prog("#define FOO(x) (_Generic (x, int: 1))", "static int x, y; y = FOO(x);"), False, []),
    ("prte_c_have__static_assert", "PRTE_C_HAVE__STATIC_ASSERT", prog("#include <stdint.h>", '_Static_assert(sizeof(int64_t) == 8, "WTH");'), False, []),
    ("prte_c_have_builtin_expect", "PRTE_C_HAVE_BUILTIN_EXPECT", prog("", "void *ptr = (void*) 0;\n           if (__builtin_expect (ptr != (void*) 0, 1)) return 0;"), True, []),
    # File-scope statements in the m4, which is why configure answers 0 for
    # these on every compiler; kept as written (as in PMIx).
    ("prte_c_have_builtin_prefetch", "PRTE_C_HAVE_BUILTIN_PREFETCH", prog("int ptr;\n           __builtin_prefetch(&ptr,0,0);", ""), True, []),
    ("prte_c_have_builtin_clz", "PRTE_C_HAVE_BUILTIN_CLZ", prog("int value = 0xffff; /* we know we have 16 bits set */\n             if ((8*sizeof(int)-16) != __builtin_clz(value)) return 0;", ""), True, []),
    ("prte_have_pthread_mutex_errorcheck_np", "PRTE_HAVE_PTHREAD_MUTEX_ERRORCHECK_NP", prog("#include <pthread.h>", "pthread_mutexattr_settype(NULL, PTHREAD_MUTEX_ERRORCHECK_NP);"), True, ["_GNU_SOURCE"]),
    ("prte_have_pthread_mutex_errorcheck", "PRTE_HAVE_PTHREAD_MUTEX_ERRORCHECK", prog("#include <pthread.h>", "pthread_mutexattr_settype(NULL, PTHREAD_MUTEX_ERRORCHECK);"), True, []),
    ("words_bigendian", "WORDS_BIGENDIAN", "int probe[__BYTE_ORDER__ == __ORDER_BIG_ENDIAN__ ? 1 : -1];\n", False, []),
    # prte_check_ptrace.m4: the Linux ptrace signature, compiled with -Werror
    # in the m4 — carried here by the pragma, since the probe takes no copts.
    # glibc declares ptrace variadic, so this answers 0 there as configure did.
    ("prte_have_linux_ptrace", "PRTE_HAVE_LINUX_PTRACE",
     prog('#pragma GCC diagnostic error "-Wincompatible-pointer-types"\n#include "sys/ptrace.h"',
          "long (*ptr)(enum __ptrace_request request, pid_t pid, void *addr, void *data);\nptr = ptrace;"), False, []),
    # Stop-on-exec needs a traceme and a detach request, either spelling.
    ("prte_have_stop_on_exec", "PRTE_HAVE_STOP_ON_EXEC", prog(PTRACE_BOTH, "(void) ptrace;"), True, []),
]
symbols = [  # (target, define, symbol, headers)
    ("prte_have_unix_byteswap", "HAVE_UNIX_BYTESWAP", "htonl", ["arpa/inet.h"]),
    ("prte_have_socket", "PRTE_HAVE_SOCKET", "socket", ["sys/socket.h"]),
    ("prte_have_gethostbyname", "PRTE_HAVE_GETHOSTBYNAME", "gethostbyname", ["netdb.h"]),
    ("prte_have_backtrace_execinfo", "PRTE_HAVE_BACKTRACE_EXECINFO", "backtrace", ["execinfo.h"]),
]

rules = ['''
# ---- Agent-stage resolutions (bzl-7r9.6), each a decision with its reason.
#
# Probes PRRTE runs with its OWN snippets — compiler attributes, C11
# keywords, builtins, pthread and ptrace constants — reproduced from its m4
# so the consumer's compiler is asked the question configure asked. As in
# PMIx: the m4 also greps warnings for "ignored", which a probe cannot, and
# __builtin_prefetch/__builtin_clz put statements at file scope (0 always).
''']
for t, d, src, link, defs in probes:
    rules.append("check_c_source_compiles(\n    name = %s,\n    define = %s,\n    source = %s,%s%s\n)\n" % (
        sl(t), sl(d), sl(src), "\n    link = True," if link else "",
        ("\n    defines = [%s]," % ", ".join(sl(x) for x in defs)) if defs else ""))
rules.append('''
# Facts PRRTE defines under its own names from a libc check, probed as
# symbols project-locally because the names are PRRTE's.
''')
for t, d, sym, hdrs in symbols:
    rules.append("check_symbol_exists(\n    name = %s,\n    define = %s,\n    symbol = %s,\n    headers = [%s],\n    defines = [\"_GNU_SOURCE\"],\n)\n" % (
        sl(t), sl(d), sl(sym), ", ".join(sl(h) for h in hdrs)))

# ---- static-components.h per framework (item 004) ---------------------------
FRAMEWORKS = ["errmgr", "ess", "filem", "grpcomm", "iof", "odls", "oob", "plm", "prtebacktrace",
              "prtedl", "prteinstalldirs", "prtereachable", "ras", "rmaps", "rtc", "schizo", "state"]
targets_manifest = open(os.path.join(M, "TARGETS")).read()
comps = {fw: sorted(re.findall(r"^library libprtemca_%s_([a-z0-9_]+)\.la " % fw, targets_manifest, re.M)) for fw in FRAMEWORKS}
assert all(comps.values()), comps
rules.append('''
# Each MCA framework's static-components.h is written by configure itself
# (config/prte_mca.m4), not by make: the components compiled INTO libprrte,
# which are exactly this module's libprtemca_<framework>_<component> targets.
''')
for fw in FRAMEWORKS:
    externs = "".join("extern const pmix_mca_base_component_t prte_mca_%s_%s_component;\n" % (fw, c) for c in comps[fw])
    entries = "".join("  &prte_mca_%s_%s_component, \n" % (fw, c) for c in comps[fw])
    content = "/*\n * $$HEADER$$\n */\n#if defined(c_plusplus) || defined(__cplusplus)\nextern \"C\" {\n#endif\n\n%s\nconst pmix_mca_base_component_t *prte_%s_base_static_components[] = {\n%s  NULL\n};\n\n#if defined(c_plusplus) || defined(__cplusplus)\n}\n#endif\n\n" % (externs, fw, entries)
    rules.append('genrule(\n    name = "static_components_%s_h",\n    outs = ["src/mca/%s/base/static-components.h"],\n    cmd = """cat > $@ <<\'EOF\'\n%sEOF""",\n)\n' % (fw, fw, content))

anchor = "\nconfig_header(\n"
i = s.index(anchor)
s = s[:i] + "\n" + "".join(rules) + s[i:]

# ---- 3. the private config header (item 001) --------------------------------
def header_block(name):
    i = s.index('    name = "%s",' % name)
    start = s.rfind("config_header(", 0, i)
    end = s.index("\n)\n", start) + 3
    return start, end

start, end = header_block("src_include_prte_config_h")
block = s[start:end]
m = re.search(r"    unresolved = \[\n(.*?)    \],\n", block, re.S)
assert m
unresolved = re.findall(r'"([A-Za-z_0-9]+)"', m.group(1))
block = block.replace(m.group(0), "")
probe_defines = {d for _, d, *_ in probes} | {d for _, d, *_ in symbols}
labels = [t for t, *_ in probes] + [t for t, *_ in symbols]
if "    probes = [\n" in block:
    block = block.replace("    probes = [\n", "    probes = [\n" + "".join('        ":%s",\n' % t for t in labels), 1)
else:
    ti = block.index("    template = "); tj = block.index("\n", ti) + 1
    block = block[:tj] + "    probes = [\n" + "".join('        ":%s",\n' % t for t in labels) + "    ],\n" + block[tj:]

VALUES = {
    # -- the default configuration, as configure decided it; not host facts
    "PRTE_ENABLE_DLOPEN_SUPPORT": "1", "PRTE_HAVE_DL_SUPPORT": "1",  # --enable-dlopen switches sources; the default holds
    "PRTE_DL_LIBLTDL_HAVE_LT_DLADVISE": "0",  # the libltdl prtedl component is not built; dlopen is
    "PRTE_HAVE_LIBEVENT": "1", "PRTE_HAVE_LIBEV": "0",
    "PRTE_PICKY_COMPILERS": "0", "PRTE_MEMORY_SANITIZERS": "0",
    "PRTE_PMIX_MINIMUM_VERSION": "0x00040204",
    "PRTE_PROXY_BUGREPORT": '"https://github.com/openpmix/prrte/"',
    "PRTE_PROXY_PACKAGE_NAME": '"PMIx Reference RunTime Environment"',
    "PRTE_PROXY_VERSION_STRING": '"3.0.14"',
    "PRTE_IDENT_STRING": '"3.0.14"', "PRTE_GREEK_VERSION": '""',
    "PRTE_C_HAVE_VISIBILITY": "1",  # -fvisibility=hidden: as PMIx, the toolchain's clang accepts it
    "PRTE_HAVE_CEIL": "1",
    # #ident is accepted by gcc and clang alike; the pragma forms are not used
    "PRTE_CC_USE_IDENT": "1", "PRTE_CC_USE_CONST_CHAR_IDENT": "0", "PRTE_CC_USE_PRAGMA_IDENT": "0",
    "PRTE_CC_USE_PRAGMA_COMMENT": "",
    # ptrace request names on the glibc this module's toolchain declares
    # (configure found PT_TRACE_ME/PT_DETACH: glibc's PTRACE_* are enum
    # members, not macros, so its #ifdef chain falls through to PT_*)
    "PRTE_TRACEME": "PT_TRACE_ME", "PRTE_DETACH": "PT_DETACH",
    # read by no source (AC_SEARCH_LIBS bookkeeping); configure's answer
    "PRTE_HAVE_SHM_OPEN_RT": "1", "PRTE_HAVE_YP_ALL_NSL": "1",
    # -- stamps and toolchain identity, read only by prte_info (its
    # comparison is omitted on record): the module's toolchain is clang
    "PRTE_CC": '"clang"', "PRTE_BUILD_PLATFORM_COMPILER_FAMILYID": "0", "PRTE_BUILD_PLATFORM_COMPILER_VERSION": "0",
    # configure's default is "Open MPI $USER@$HOST Distribution" — the
    # conversion host's user and name; the stamp is dropped
    "PRTE_PACKAGE_STRING": '"Open MPI Distribution"',
    "PACKAGE_URL": '""', "LT_OBJDIR": '".libs/"',
    "PRTE_GIT_REPO_BUILD": "", "AC_APPLE_UNIVERSAL_BUILD": "",
    "OAC_HAVE_SOLARIS": "0",
    # -- AC_USE_SYSTEM_EXTENSIONS, exactly as configure writes them everywhere
    "_ALL_SOURCE": "1", "_GNU_SOURCE": "1", "_POSIX_PTHREAD_SEMANTICS": "1", "_TANDEM_SOURCE": "1", "__EXTENSIONS__": "1",
    "_MINIX": "", "_POSIX_SOURCE": "", "_POSIX_1_SOURCE": "",
}
covered = probe_defines | set(VALUES) | {"OAC_HAVE_APPLE", "PRTE_ARCH"}
missing = [n for n in unresolved if n not in covered]
assert not missing, missing
extra = [n for n in VALUES if n not in unresolved]
assert not extra, extra
selects = (
    '    select({\n        "@platforms//os:macos": {"OAC_HAVE_APPLE": "1"},\n        "//conditions:default": {"OAC_HAVE_APPLE": "0"},\n    }) | '
    # the triple configure was configured for, per target CPU; read only by prte_info
    '    select({\n        "@platforms//cpu:aarch64": {"PRTE_ARCH": "\\"aarch64-unknown-linux-gnu\\""},\n'
    '        "//conditions:default": {"PRTE_ARCH": "\\"x86_64-pc-linux-gnu\\""},\n    }) | '
)
value_lines = "".join("        %s: %s,\n" % (sl(k), sl(v)) for k, v in sorted(VALUES.items()))
note = '''        # ---- resolved by the agent stage; "" is undefined, a 0 is a zero.
        # Probes above answer every compiler question; these are the default
        # configuration's choices, the glibc spellings of the ptrace requests,
        # and prte_info's stamps rewritten for the module's toolchain.
'''
anchor = "    }) | {\n" if "    }) | {\n" in block else "    values = {\n"
assert block.count(anchor) == 1
block = block.replace(anchor, anchor[:-2] + selects.lstrip() + "{\n" + note + value_lines, 1)
s = s[:start] + block + s[end:]

# ---- version.h (item 002) ---------------------------------------------------
start, end = header_block("src_include_version_h")
block = s[start:end]
m = re.search(r"    unresolved = \[\n(.*?)    \],\n", block, re.S)
assert m and sorted(re.findall(r'"(\w+)"', m.group(1))) == ["PRTE_GREEK_VERSION", "PRTE_WANT_REPO_REV"], m.group(1)
block = block.replace(m.group(0), "")
vals = '        "PRTE_GREEK_VERSION": "\\"\\"",  # a release: no greek suffix, as configure resolved\n        "PRTE_WANT_REPO_REV": "",  # a tarball: no repository revision to report\n'
anchor = "    }) | {\n" if "    }) | {\n" in block else "    values = {\n"
if anchor in block:
    block = block.replace(anchor, anchor + vals, 1)
else:
    block = block.replace("\n)\n", "\n    values = {\n" + vals + "    },\n)\n")
s = s[:start] + block + s[end:]

# ---- 4. libprrte.la absorbs its convenience archives (item 003) -------------
i = s.index('    name = "libprrte.la",')
deps_start = s.index("    deps = [\n", i)
deps_end = s.index("    ],\n", deps_start)
archives = re.findall(r'"(:lib(?:prrteutil|prtemca)[^"]*\.la)"', s[deps_start:deps_end])
assert len(archives) == 49, len(archives)
rep('''cc_shared_library(
    name = "libprrte.la_shared",
    deps = [
        ":libprrte.la",
    ],''', '''# Every noinst_ convenience archive libprrte links (item 003), absorbed into
# THIS shared object and exported through it — what automake's
# noinst_LTLIBRARIES pulled into libprrte.la means.
cc_shared_library(
    name = "libprrte.la_shared",
    deps = [
        ":libprrte.la",
''' + "".join('        "%s",\n' % a for a in archives) + '''    ],''')

# ---- 5. build stamps (item 005): fixed literals; prte_info omitted ----------
STAMPS = {"PRTE_BUILD_DATE": "unknown", "PRTE_BUILD_HOST": "bazel", "PRTE_BUILD_USER": "bazel"}
for name, literal in STAMPS.items():
    pat = re.compile(r'"%s=\'\\"[^\n]*?\\"\'",' % name)
    assert len(pat.findall(s)) >= 1, name
    s = pat.sub('"%s=\'\\"%s\\"\'",  # a build stamp; the recipe computed it with the shell' % (name, literal), s)
t = targets_manifest.rstrip("\n") + "\nomitted prte_info prints the configure/build stamps (date, host, user, paths), which differ from the ground truth by construction\n"
open(os.path.join(M, "TARGETS"), "w").write(t)

# ---- 6. static-components.h into each framework's base archive --------------
for fw in FRAMEWORKS:
    i = s.index('    name = "libprtemca_%s.la",' % fw)
    j = s.index("    srcs = [\n", i) + len("    srcs = [\n")
    s = s[:j] + '        ":static_components_%s_h",\n' % fw + s[j:]

open(B, "w").write(s)
mb = os.path.join(M, "MODULE.bazel"); ms = open(mb).read()
if 'name = "platforms"' not in ms:
    ms = ms.replace('bazel_dep(name = "cc_config", version = "0.0.0")\n',
                    'bazel_dep(name = "cc_config", version = "0.0.0")\nbazel_dep(name = "platforms", version = "1.1.0")\n')
    open(mb, "w").write(ms)
na = os.path.join(M, "needs_attention")
for f in os.listdir(na):
    if f.endswith(".md"):
        os.remove(os.path.join(na, f))
print("resolved prrte: %d probes, %d symbols, %d values, %d frameworks, %d archives" % (len(probes), len(symbols), len(VALUES), len(FRAMEWORKS), len(archives)))
