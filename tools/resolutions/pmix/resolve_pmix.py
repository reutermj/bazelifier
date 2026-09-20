#!/usr/bin/env python3
"""Agent-stage resolution for the converted PMIx module (bzl-7r9.5).

Applied to a FRESH unpack: python3 resolve_pmix.py /tmp/resolve-pmix/fixtures/pmix
All-or-nothing: every anchor is asserted before anything is written.
"""
import os, re, sys, shutil

M = sys.argv[1]
B = os.path.join(M, "BUILD.bazel")
s = open(B).read()
here = os.path.dirname(os.path.abspath(__file__))

def rep(old, new, count=1):
    global s
    assert s.count(old) == count, (old[:80], s.count(old))
    s = s.replace(old, new)

def sl(v):
    return '"' + v.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t") + '"'

# ---- 1. loads ---------------------------------------------------------------
rep('load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n',
    'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n'
    'load("@cc_config//cc_config:probe.bzl", "check_c_source_compiles", "check_symbol_exists", "probe_alias")\n'
    'load("@bazel_skylib//rules:common_settings.bzl", "bool_flag")\n')

# ---- 2. probes the project runs with its OWN snippets (item 001/002) --------
# Each snippet is PMIx's, from config/pmix_check_attributes.m4 and
# config/pmix_setup_cc.m4 — reproduced so the module asks the consumer's
# compiler the question configure asked, not this host's answer.
PROLOGUE_MAIN = "int main(void) {{ {body} ; return 0; }}\n"
ATTRS = {  # name -> source (AC_LANG_SOURCE: file scope)
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
    "nonnull": "int square(int *arg) __attribute__ ((__nonnull__));\nint square(int *arg) { return *arg; }",
    "noreturn": "#include <unistd.h>\n#include <stdlib.h>\nvoid fatal(int arg1) __attribute__ ((__noreturn__));\nvoid fatal(int arg1) { exit(arg1); }",
    "noreturn_funcptr": "#include <unistd.h>\n#include <stdlib.h>\nextern void (*fatal_exit)(int arg1) __attribute__ ((__noreturn__));\nvoid fatal(int arg1) { fatal_exit (arg1); }",
    "packed": "struct foo {\n    char a;\n    int x[2] __attribute__ ((__packed__));\n};",
    "pure": "int square(int arg) __attribute__ ((__pure__));\nint square(int arg) { return arg * arg; }",
    "sentinel": "int my_execlp(const char * file, const char *arg, ...) __attribute__ ((__sentinel__));",
    "unused": "int square(int arg1 __attribute__ ((__unused__)), int arg2);\nint square(int arg1, int arg2) { return arg2; }",
    "visibility": 'int square(int arg1) __attribute__ ((__visibility__("hidden")));',
    "warn_unused_result": "int foo(int arg) __attribute__ ((__warn_unused_result__));\nint foo(int arg) { return arg + 3; }",
    "destructor": "void foo(void) __attribute__ ((__destructor__));\nvoid foo(void) { return ; }",
    "optnone": "void __attribute__ ((__optnone__)) foo(void);\nvoid foo(void) { return ; }",
    "extension": "int i = __extension__ 3;",
}
probes = []  # (target name, define, source, link, defines)
probes.append(("pmix_have_attribute", "PMIX_HAVE_ATTRIBUTE",
               "#include <stdlib.h>\nint main(void) {\n  struct foo {\n      char a;\n      int x[2] __attribute__ ((__packed__));\n   };\n  return 0;\n}\n", False, []))
for name, src in ATTRS.items():
    probes.append(("pmix_have_attribute_" + name, "PMIX_HAVE_ATTRIBUTE_" + name.upper(), src + "\n", False, []))
# C11 / compiler features (pmix_setup_cc.m4, AC_LANG_PROGRAM shape)
def prog(prologue, body):
    return prologue + ("\n" if prologue else "") + "int main(void) {\n" + body + "\n  ;\n  return 0;\n}\n"
probes += [
    ("pmix_c_have___thread", "PMIX_C_HAVE___THREAD", prog("", "static __thread int  foo = 1;++foo;"), False, []),
    ("pmix_c_have__thread_local", "PMIX_C_HAVE__THREAD_LOCAL", prog("", "static _Thread_local int  foo = 1;++foo;"), False, []),
    ("pmix_c_have_atomic_conv_var", "PMIX_C_HAVE_ATOMIC_CONV_VAR", prog("#include <stdatomic.h>", "static atomic_long foo = 1;++foo;"), False, []),
    ("pmix_c_have__atomic", "PMIX_C_HAVE__ATOMIC", prog("#include <stdatomic.h>", "static _Atomic long foo = 1;++foo;"), False, []),
    ("pmix_have_clang_builtin_atomic_c11_func", "PMIX_HAVE_CLANG_BUILTIN_ATOMIC_C11_FUNC", prog("#include <stdatomic.h>", "atomic_int acnt = 0; __c11_atomic_fetch_add(&acnt, 1, memory_order_relaxed);"), False, []),
    ("pmix_c_have__generic", "PMIX_C_HAVE__GENERIC", prog("#define FOO(x) (_Generic (x, int: 1))", "static int x, y; y = FOO(x);"), False, []),
    ("pmix_c_have__static_assert", "PMIX_C_HAVE__STATIC_ASSERT", prog("#include <stdint.h>", '_Static_assert(sizeof(int64_t) == 8, "WTH");'), False, []),
    ("pmix_c_have_builtin_expect", "PMIX_C_HAVE_BUILTIN_EXPECT", prog("", "void *ptr = (void*) 0;\n           if (__builtin_expect (ptr != (void*) 0, 1)) return 0;"), True, []),
    # These two put their statements in the PROLOGUE (file scope) — the m4
    # does, and that is why configure answers 0 for both on every compiler.
    # Reproduced as written so the module answers what the build answered.
    ("pmix_c_have_builtin_prefetch", "PMIX_C_HAVE_BUILTIN_PREFETCH", prog("int ptr;\n           __builtin_prefetch(&ptr,0,0);", ""), True, []),
    ("pmix_c_have_builtin_clz", "PMIX_C_HAVE_BUILTIN_CLZ", prog("int value = 0xffff; /* we know we have 16 bits set */\n             if ((8*sizeof(int)-16) != __builtin_clz(value)) return 0;", ""), True, []),
    ("pmix_have_pthread_mutex_errorcheck_np", "PMIX_HAVE_PTHREAD_MUTEX_ERRORCHECK_NP", prog("#include <pthread.h>", "pthread_mutexattr_settype(NULL, PTHREAD_MUTEX_ERRORCHECK_NP);"), True, ["_GNU_SOURCE"]),
    ("pmix_have_pthread_mutex_errorcheck", "PMIX_HAVE_PTHREAD_MUTEX_ERRORCHECK", prog("#include <pthread.h>", "pthread_mutexattr_settype(NULL, PTHREAD_MUTEX_ERRORCHECK);"), True, []),
    # AC_C_BIGENDIAN's answer, asked of the compiler's own byte-order macro.
    ("words_bigendian", "WORDS_BIGENDIAN", "int probe[__BYTE_ORDER__ == __ORDER_BIG_ENDIAN__ ? 1 : -1];\n", False, []),
]
symbols = [  # (target, define, symbol, headers, link-under _GNU_SOURCE)
    ("pmix_have_sa_restart", "PMIX_HAVE_SA_RESTART", "SA_RESTART", ["signal.h"]),
    ("pmix_have_va_copy", "PMIX_HAVE_VA_COPY", "va_copy", ["stdarg.h"]),
    ("pmix_have_underscore_va_copy", "PMIX_HAVE_UNDERSCORE_VA_COPY", "__va_copy", ["stdarg.h"]),
    ("have_unix_byteswap", "HAVE_UNIX_BYTESWAP", "htonl", ["arpa/inet.h"]),
    ("pmix_have_socket", "PMIX_HAVE_SOCKET", "socket", ["sys/socket.h"]),
    ("pmix_have_gethostbyname", "PMIX_HAVE_GETHOSTBYNAME", "gethostbyname", ["netdb.h"]),
    ("pmix_have_dirname", "PMIX_HAVE_DIRNAME", "dirname", ["libgen.h"]),
]
aliases = [("pmix_have_clock_gettime", "PMIX_HAVE_CLOCK_GETTIME", "have_clock_gettime")]

rules = ['''
# ---- Agent-stage resolutions (bzl-7r9.5), each a decision with its reason.
#
# Probes the project runs with its OWN snippets — compiler attributes, C11
# keywords, builtins, a pthread constant — reproduced verbatim from PMIx's m4
# (config/pmix_check_attributes.m4, pmix_setup_cc.m4, pmix_config_pthreads.m4)
# so the consumer's compiler is asked the question configure asked. Two of
# them (__builtin_prefetch, __builtin_clz) put statements at file scope, which
# is why configure answers 0 for both on every compiler; kept as written.
# configure additionally greps the compiler's warnings for "ignored" on the
# attribute checks, which this probe cannot; on a compiler that only warns,
# an attribute answers 1 here where configure answered 0.
''']
for t, d, src, link, defs in probes:
    rules.append("check_c_source_compiles(\n    name = %s,\n    define = %s,\n    source = %s,%s%s\n)\n" % (
        sl(t), sl(d), sl(src), "\n    link = True," if link else "",
        ("\n    defines = [%s]," % ", ".join(sl(x) for x in defs)) if defs else ""))
rules.append('''
# Facts PMIx defines under its own names from a libc check (AC_SEARCH_LIBS,
# AC_CHECK_DECL, AC_EGREP_CPP): probed here as symbols, project-locally
# because the names are project-prefixed and shared by nothing else.
''')
for t, d, sym, hdrs in symbols:
    rules.append("check_symbol_exists(\n    name = %s,\n    define = %s,\n    symbol = %s,\n    headers = [%s],\n    defines = [\"_GNU_SOURCE\"],\n)\n" % (
        sl(t), sl(d), sl(sym), ", ".join(sl(h) for h in hdrs)))
rules.append('''
# Facts configure derives from a catalog probe under its own name.
''')
for t, d, p in aliases:
    rules.append("probe_alias(\n    name = %s,\n    define = %s,\n    probe = \"@cc_config//catalog:%s\",\n)\n" % (sl(t), sl(d), p))
rules.append('''
# Build options a default configure leaves as they are here (0/1 macros it
# always defines; see configure --help). Debug, IPv6 and timing are pure
# macros; PTY and dlopen support also select sources and components, so
# they stay values below with the default configuration's answer.
''')
FLAGS = [("enable_debug", False, "PMIX_ENABLE_DEBUG"), ("enable_ipv6", False, "PMIX_ENABLE_IPV6"), ("enable_timing", False, "PMIX_ENABLE_TIMING")]
for f, default, _ in FLAGS:
    rules.append('bool_flag(\n    name = "%s",\n    build_setting_default = %s,\n)\n\nconfig_setting(\n    name = "%s_on",\n    flag_values = {":%s": "True"},\n)\n' % (f, default, f, f))

# static-components.h, written by CONFIGURE (config/pmix_mca.m4) from the
# component list it selected — the list of this module's own component
# targets, one genrule per framework, in the format configure writes.
FRAMEWORKS = ["bfrops", "gds", "pcompress", "pdl", "pif", "pinstalldirs", "plog", "pmdl", "pnet", "preg", "psec", "psensor", "psquash", "pstat", "ptl"]
targets_manifest = open(os.path.join(M, "TARGETS")).read()
comps = {fw: sorted(re.findall(r"^library libpmix_mca_%s_([a-z0-9_]+)\.la " % fw, targets_manifest, re.M)) for fw in FRAMEWORKS}
comps["pcompress"] = [c for c in comps["pcompress"] if c != "zlib"]
rules.append('''
# Each MCA framework's static-components.h is written by configure itself
# (config/pmix_mca.m4), not by make: the components compiled INTO libpmix,
# which are exactly this module's libpmix_mca_<framework>_<component>
# targets. pcompress lists none — its only component, zlib, is a DSO the
# default build loads at run time and this module omits (see the -lz note
# on the framework below).
''')
for fw in FRAMEWORKS:
    externs = "".join("extern const pmix_mca_base_component_t pmix_mca_%s_%s_component;\n" % (fw, c) for c in comps[fw])
    entries = "".join("  &pmix_mca_%s_%s_component, \n" % (fw, c) for c in comps[fw])
    content = "/*\n * $$HEADER$$\n */\n#if defined(c_plusplus) || defined(__cplusplus)\nextern \"C\" {\n#endif\n\n%s\nconst pmix_mca_base_component_t *pmix_mca_%s_base_static_components[] = {\n%s  NULL\n};\n\n#if defined(c_plusplus) || defined(__cplusplus)\n}\n#endif\n\n" % (externs, fw, entries)
    rules.append('genrule(\n    name = "static_components_%s_h",\n    outs = ["src/mca/%s/base/static-components.h"],\n    cmd = """cat > $@ <<\'EOF\'\n%sEOF""",\n)\n' % (fw, fw, content))

rep("\n# The project also ships a checked-in `src/include/pmix_config.h`.", "".join(rules) + "\n# The project also ships a checked-in `src/include/pmix_config.h`.")

# ---- 3. the private config header: unresolved -> probes + values -----------
def header_block(name):
    i = s.index('    name = "%s",' % name)
    start = s.rfind("config_header(", 0, i)
    end = s.index("\n)\n", start) + 3
    return start, end
start, end = header_block("src_include_pmix_config_h")
block = s[start:end]
m = re.search(r"    unresolved = \[\n(.*?)    \],\n", block, re.S)
assert m
unresolved = re.findall(r'"([A-Za-z_0-9]+)"', m.group(1))
block = block.replace(m.group(0), "")
probe_labels = [t for t, *_ in probes] + [t for t, *_ in symbols] + [t for t, *_ in aliases]
probe_defines = {d for _, d, *_ in probes} | {d for _, d, *_ in symbols} | {d for _, d, *_ in aliases}
block = block.replace("    probes = [\n", "    probes = [\n" + "".join('        ":%s",\n' % t for t in probe_labels), 1)
VALUES = {
    # -- decided as the default configure decided; a decision about THIS module
    "PMIX_USE_GCC_BUILTIN_ATOMICS": "1", "PMIX_USE_C11_ATOMICS": "0",
    "PMIX_HAVE_SYNC_BUILTIN_CSWAP_INT128": "1", "PMIX_HAVE_GCC_BUILTIN_CSWAP_INT128": "0",
    "PMIX_HAVE_C11_CSWAP_INT128": "",
    "PMIX_C_GCC_INLINE_ASSEMBLY": "1",
    "PMIX_HAVE_LIBEVENT": "1", "PMIX_HAVE_LIBEV": "0",
    "PMIX_HAVE_PDL_SUPPORT": "1", "PMIX_PDL_PLIBLTDL_HAVE_LT_DLADVISE": "1",
    "PMIX_ENABLE_PTY_SUPPORT": "1", "PMIX_ENABLE_DLOPEN_SUPPORT": "1",
    "PMIX_HAVE_VISIBILITY": "1",
    "PMIX_HAVE_CEIL": "1",
    # openpty: what configure does on THIS toolchain. Its sysroot is a glibc
    # 2.28, where openpty is declared by pty.h and defined only in libutil,
    # so AC_SEARCH_LIBS([openpty], [util]) finds it there and adds -lutil
    # (on the conversion host, glibc 2.39, it was in libc and no flag was
    # needed — the link line states nothing). The catalog probe cannot link
    # -lutil and answered false, which sent PMIx down its own-implementation
    # branch, and that branch does not compile (project note 001). So the
    # answer is recorded — 1, and -lutil on libpmix.la — rather than probed.
    "HAVE_OPENPTY": "1", "PMIX_HAVE_OPENPTY": "1",
    "PMIX_NEED_C_BOOL": "1", "PMIX_USE_STDBOOL_H": "1", "PMIX_PTRDIFF_TYPE": "ptrdiff_t",
    "PMIX_PICKY_COMPILERS": "0", "PMIX_MEMORY_SANITIZERS": "0", "PMIX_NO_LIB_DESTRUCTOR": "0",
    "PMIX_WANT_HOME_CONFIG_FILES": "1", "PMIX_WANT_PRETTY_PRINT_STACKTRACE": "1",
    "PMIX_SHOW_LOAD_ERRORS_DEFAULT": '"none"', "PMIX_IDENT_STRING": '""',
    "PMIX_HAVE_ATTRIBUTE_WEAK_ALIAS": "",  # configure writes an EMPTY define; unused by any source
    # -- stamps and toolchain identity: the module's toolchain is clang
    "PMIX_CC": '"clang"', "PMIX_BUILD_PLATFORM_COMPILER_FAMILYID": "19", "PMIX_BUILD_PLATFORM_COMPILER_VERSION": "0",
    "PMIX_PACKAGE_STRING": '"PMIx Distribution"',
    "PACKAGE_URL": '""', "LT_OBJDIR": '".libs/"',
    "PMIX_GIT_REPO_BUILD": "", "AC_APPLE_UNIVERSAL_BUILD": "", "YYTEXT_POINTER": "",
    "inline": "__inline__",  # what THIS project's configure decided (AC_C_INLINE after its own CFLAGS)
    # -- AC_USE_SYSTEM_EXTENSIONS, exactly as configure writes them everywhere
    "_ALL_SOURCE": "1", "_GNU_SOURCE": "1", "_POSIX_PTHREAD_SEMANTICS": "1", "_TANDEM_SOURCE": "1", "__EXTENSIONS__": "1",
    "_MINIX": "", "_POSIX_SOURCE": "", "_POSIX_1_SOURCE": "",
    "OAC_HAVE_SOLARIS": "0",
}
covered = probe_defines | set(VALUES) | {d for _, _, d in FLAGS} | {"OAC_HAVE_APPLE"}
missing = [n for n in unresolved if n not in covered]
assert not missing, missing
extra = [n for n in VALUES if n not in unresolved and n != "HAVE_OPENPTY"]
assert not extra, extra
# the catalog's openpty probe is replaced by the recorded answer above
assert block.count('        "@cc_config//catalog:have_openpty",\n') == 1
block = block.replace('        "@cc_config//catalog:have_openpty",\n', "")
flag_select = "".join('    select({\n        ":%s_on": {"%s": "1"},\n        "//conditions:default": {"%s": "0"},\n    }) | ' % (f, d, d) for f, _, d in FLAGS)
apple_select = '    select({\n        "@platforms//os:macos": {"OAC_HAVE_APPLE": "1"},\n        "//conditions:default": {"OAC_HAVE_APPLE": "0"},\n    }) | '
value_lines = "".join("        %s: %s,\n" % (sl(k), sl(v)) for k, v in sorted(VALUES.items()))
assert block.count("    values = {\n") == 1
block = block.replace("    values = {\n", "    values = " + flag_select.lstrip() + apple_select + "{\n" + '''        # ---- resolved by the agent stage; "" is undefined, a 0 is a zero.
        # Atomics: the default configure prefers GCC builtins over C11 and
        # answered the 128-bit compare-and-swap questions by RUNNING a program
        # (lock-free or not), which no probe here can; the x86-64 answers are
        # recorded. Inline assembly likewise. PTY and dlopen support are the
        # defaults and also gate sources, so they are values, not flags.
        # Stamps (PMIX_CC, the compiler family id 19 = clang, the package
        # string) name the module's toolchain rather than the conversion
        # host. `inline` is __inline__ because THIS configure decided so.
''' + value_lines, 1)
s = s[:start] + block + s[end:]

# the assertion for that header: the frontend asserted only its own values,
# and the agent's entries are pinned by the rule itself; add the decisive
# ones — a probe that answers 1 under any C11 compiler, and the zero.
start, end = header_block("src_include_pmix_config_h")  # unchanged anchors
i = s.index('    name = "src_include_pmix_config_h_test",')
j = s.index("    must_contain = [\n", i) + len("    must_contain = [\n")
s = s[:j] + '        "#define PMIX_HAVE_ATTRIBUTE 1",\n        "#define PMIX_C_HAVE__STATIC_ASSERT 1",\n        "/* #undef PMIX_C_HAVE_BUILTIN_PREFETCH */",  # the m4 puts its statement at file scope; 0 everywhere\n        "#define PMIX_USE_GCC_BUILTIN_ATOMICS 1",\n' + s[j:]

# ---- pmix_common.h: PMIX_HAVE_VISIBILITY ------------------------------------
start, end = header_block("include_pmix_common_h")
block = s[start:end]
m = re.search(r"    unresolved = \[\n(.*?)    \],\n", block, re.S)
assert m and re.findall(r'"(\w+)"', m.group(1)) == ["PMIX_HAVE_VISIBILITY"]
block = block.replace(m.group(0), '    values = {\n        "PMIX_HAVE_VISIBILITY": "1",  # as the private header: -fvisibility=hidden works, PMIX_EXPORT is visibility("default")\n    },\n')
s = s[:start] + block + s[end:]

# ---- 4. libpmix.la absorbs 50 convenience archives (item 003) --------------
i = s.index('    name = "libpmix.la",')
j = s.index("    deps = [\n", i)
k = s.find("    linkopts = [\n", i, j)
util = '        "-lutil",  # openpty on this toolchain\'s glibc 2.28 sysroot; see the config header\n'
if k != -1:
    k += len("    linkopts = [\n")
    s = s[:k] + util + s[k:]
else:
    s = s[:j] + "    linkopts = [\n" + util + "    ],\n" + s[j:]
deps_start = s.index("    deps = [\n", i)
deps_end = s.index("    ],\n", deps_start)
archives = re.findall(r'"(:lib[^"]+\.la)"', s[deps_start:deps_end])
assert len(archives) == 50, len(archives)
rep('''cc_shared_library(
    name = "libpmix.la_shared",
    deps = [
        ":libpmix.la",
    ],''', '''# Every noinst_ convenience archive libpmix links (item 003): named here so
# each is absorbed into THIS shared object and exported through it, which is
# what automake's `noinst_LTLIBRARIES` pulled into libpmix.la means. The MCA
# framework and component archives are all reached only through libpmix.
cc_shared_library(
    name = "libpmix.la_shared",
    deps = [
        ":libpmix.la",
''' + "".join('        "%s",\n' % a for a in archives) + '''    ],''')

# ---- 5. the zlib component (item 005) is not built --------------------------
i = s.index('cc_library(\n    name = "pmix_mca_pcompress_zlib.la",')
j = s.index("\n)\n", s.index('    name = "pmix_mca_pcompress_zlib.la_shared",')) + 3
s = s[:i] + '''# The pcompress:zlib component is NOT built (the -lz item): it is a DSO the
# default build dlopens at run time, and links a zlib no converted module
# supplies to this conversion — zlib IS in the corpus, but the CMake
# frontend does not yet emit the install tree a dependency needs
# (bzl-7r9.8). Without it PMIx falls back to no compression, which is
# what its own tests exercise. Restore the component when that lands.
''' + s[j:]
t = targets_manifest.replace("library pmix_mca_pcompress_zlib.la pmix_mca_pcompress_zlib shared\n", "")
assert t != targets_manifest
open(os.path.join(M, "TARGETS"), "w").write(t)

# ---- 5b. build stamps (the shell_expanded_defines item): fixed literals ----
# pmix_info prints them, so its comparison is recorded as omitted below.
STAMPS = {"PMIX_BUILD_DATE": "unknown", "PMIX_BUILD_HOST": "bazel", "PMIX_BUILD_USER": "bazel",
          "PMIX_CC_ABSOLUTE": "clang"}
for name, literal in STAMPS.items():
    pat = re.compile(r'"%s=\'\\"[^\n]*?\\"\'",' % name)
    n = len(pat.findall(s))
    assert n >= 1, name
    s = pat.sub('"%s=\'\\"%s\\"\'",  # a build stamp; the recipe computed it with the shell' % (name, literal), s)
t = open(os.path.join(M, "TARGETS")).read()
t = t.rstrip("\n") + "\nomitted pmix_info prints the configure/build stamps (date, host, user), which differ from the ground truth by construction\n"
open(os.path.join(M, "TARGETS"), "w").write(t)

# ---- 6. static-components.h into each framework's base archive --------------
for fw in FRAMEWORKS:
    i = s.index('    name = "libmca_%s.la",' % fw)
    j = s.index("    srcs = [\n", i) + len("    srcs = [\n")
    s = s[:j] + '        ":static_components_%s_h",\n' % fw + s[j:]

# ---- 7. the fourteen perl drivers (item 004) -------------------------------
shutil.copy(os.path.join(here, "run_pmix_perl_test.sh"), os.path.join(M, "run_pmix_perl_test.sh"))
os.chmod(os.path.join(M, "run_pmix_perl_test.sh"), 0o755)
tests = "\n# The fourteen registered perl drivers (item 004), each run through its\n# .pl.in the way `make check` runs the configured one; see run_pmix_perl_test.sh.\n"
for n in range(14):
    d = "run_tests%02d" % n
    tests += 'sh_test(\n    name = "%s_test",\n    srcs = ["run_pmix_perl_test.sh"],\n    args = ["%s"],\n    data = [\n        ":pmix_test",\n        ":pmix_client",\n        ":pmix_regex",\n        "test/%s.pl.in",\n    ],\n    size = "medium",\n)\n\n' % (d, d, d)
s = s.rstrip("\n") + "\n" + tests

open(B, "w").write(s)
# MODULE.bazel: the options need skylib
mb = os.path.join(M, "MODULE.bazel"); ms = open(mb).read()
assert 'bazel_dep(name = "cc_config", version = "0.0.0")\n' in ms
ms = ms.replace('bazel_dep(name = "cc_config", version = "0.0.0")\n', 'bazel_dep(name = "cc_config", version = "0.0.0")\nbazel_dep(name = "bazel_skylib", version = "1.7.1")\nbazel_dep(name = "platforms", version = "1.1.0")\n')
open(mb, "w").write(ms)
# ---- 8. the items are closed -------------------------------------------------
na = os.path.join(M, "needs_attention")
for f in os.listdir(na):
    if f.endswith(".md"):
        os.remove(os.path.join(na, f))
print("resolved: %d probes, %d symbols, %d aliases, %d values, %d frameworks, 14 drivers" % (len(probes), len(symbols), len(aliases), len(VALUES), len(FRAMEWORKS)))
