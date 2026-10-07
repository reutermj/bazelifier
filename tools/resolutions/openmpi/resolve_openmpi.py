#!/usr/bin/env python3
"""Agent-stage resolution for the converted Open MPI module (bzl-7r9.7).

Applied to a FRESH unpack, after its dependencies' own resolutions:
  python3 tools/resolutions/pmix/deps/resolve_hwloc.py    W/fixtures/hwloc
  python3 tools/resolutions/pmix/deps/resolve_libevent.py W/fixtures/libevent
  python3 tools/resolutions/pmix/resolve_pmix.py          W/fixtures/pmix
  python3 tools/resolutions/prrte/resolve_prrte.py        W/fixtures/prrte
  python3 tools/resolutions/openmpi/resolve_openmpi.py    W/fixtures/openmpi
All-or-nothing on its anchors.

The rule for every config header, stated once: a name keeps the value
configure resolved for the DEFAULT configuration (quoted in its item) —
component selections, the Fortran-off stubs, MCA headers, a flag's macro at
its default — EXCEPT where it is a fact about the consumer's toolchain, which
gets a probe here (Open MPI's own m4 snippets, or a libc symbol) or an alias
of a catalog probe, and except the build machine's identity (compiler,
triple, user@host), rewritten for the module's toolchain.
"""
import glob, os, re, sys

M = sys.argv[1]
B = os.path.join(M, "BUILD.bazel")
s = open(B).read()
NA = os.path.join(M, "needs_attention")


def rep(old, new, count=1):
    global s
    assert s.count(old) == count, (old[:80], s.count(old))
    s = s.replace(old, new)


def sl(v):
    return '"' + v.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t") + '"'


def item_values(pattern):
    """name -> what configure resolved ('' when it resolved nothing)."""
    f = glob.glob(os.path.join(NA, pattern))
    assert len(f) == 1, (pattern, f)
    out = {}
    for line in open(f[0]):
        m = re.match(r"^- `([A-Za-z_0-9]+)`(?: — configure resolved this to `([^`]*)`)?", line)
        if m:
            out[m.group(1)] = m.group(2) if m.group(2) is not None else ""
    return out


# ---- 1. probes Open MPI runs with its OWN snippets -------------------------
# config/opal_check_attributes.m4 and opal_setup_cc.m4 — the same oac family
# as PMIx's and PRRTE's, plus constructor, error and the Intel _Atomic check.
ATTRS = {
    "aligned": "struct foo { char text[4]; }  __attribute__ ((__aligned__(8)));",
    "always_inline": "int foo (int arg) __attribute__ ((__always_inline__));",
    "cold": "int foo(int arg1, int arg2) __attribute__ ((__cold__));\nint foo(int arg1, int arg2) { return arg1 * arg2 + arg1; }",
    "const": "int foo(int arg1, int arg2) __attribute__ ((__const__));\nint foo(int arg1, int arg2) { return arg1 * arg2 + arg1; }",
    "constructor": "void foo(void) __attribute__ ((__constructor__));\nvoid foo(void) { return ; }",
    "deprecated": "int foo(int arg1, int arg2) __attribute__ ((__deprecated__));\nint foo(int arg1, int arg2) { return arg1 * arg2 + arg1; }",
    "deprecated_argument": 'int foo(int arg1, int arg2) __attribute__ ((__deprecated__("compiler allows argument")));\nint foo(int arg1, int arg2) { return arg1 * arg2 + arg1; }',
    "destructor": "void foo(void) __attribute__ ((__destructor__));\nvoid foo(void) { return ; }",
    "error": 'int foo(int arg1, int arg2) __attribute__ ((__error__("")));',
    "extension": "int i = __extension__ 3;",
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
    "optnone": "void __attribute__ ((__optnone__)) foo(void);\nvoid foo(void) { return ; }",
    "packed": "struct foo {\n    char a;\n    int x[2] __attribute__ ((__packed__));\n};",
    "pure": "int square(int arg) __attribute__ ((__pure__));\nint square(int arg) { return arg * arg; }",
    "sentinel": "int my_execlp(const char * file, const char *arg, ...) __attribute__ ((__sentinel__));",
    "unused": "int square(int arg1 __attribute__ ((__unused__)), int arg2);\nint square(int arg1, int arg2) { return arg2; }",
    "visibility": 'int square(int arg1) __attribute__ ((__visibility__("hidden")));',
    "warn_unused_result": "int foo(int arg) __attribute__ ((__warn_unused_result__));\nint foo(int arg) { return arg + 3; }",
    "weak_alias": 'int foo(int arg);\nint foo(int arg) { return arg + 3; }\nint foo2(int arg) __attribute__ ((__weak__, __alias__("foo")));',
}


def prog(prologue, body):
    return prologue + ("\n" if prologue else "") + "int main(void) {\n" + body + "\n  ;\n  return 0;\n}\n"


probes = []  # (target, define, source, link, defines)
for name, src in ATTRS.items():
    probes.append(("ompi_have_attribute_" + name, "OPAL_HAVE_ATTRIBUTE_" + name.upper(), src + "\n", False, []))
probes += [
    ("ompi_c_have___thread", "OPAL_C_HAVE___THREAD", prog("", "static __thread int  foo = 1;++foo;"), False, []),
    ("ompi_c_have__thread_local", "OPAL_C_HAVE__THREAD_LOCAL", prog("", "static _Thread_local int  foo = 1;++foo;"), False, []),
    ("ompi_c_have_atomic_conv_var", "OPAL_C_HAVE_ATOMIC_CONV_VAR", prog("#include <stdatomic.h>", "static atomic_long foo = 1;++foo;"), False, []),
    ("ompi_c_have__atomic", "OPAL_C_HAVE__ATOMIC", prog("#include <stdatomic.h>", "static _Atomic long foo = 1;++foo;"), False, []),
    # Intel before 2020-03-10 lacks proper atomics on _Atomic; everyone else has them.
    ("ompi_c_have_atomic_support_for__atomic", "OPAL_C_HAVE_ATOMIC_SUPPORT_FOR__ATOMIC",
     "#ifdef __INTEL_COMPILER\n#if __INTEL_COMPILER_BUILD_DATE <= 20200310\n#error lacks support\n#endif\n#endif\nint ok;\n", False, []),
    ("ompi_c_have__generic", "OPAL_C_HAVE__GENERIC", prog("#define FOO(x) (_Generic (x, int: 1))", "static int x, y; y = FOO(x);"), False, []),
    ("ompi_c_have__static_assert", "OPAL_C_HAVE__STATIC_ASSERT", prog("#include <stdint.h>", '_Static_assert(sizeof(int64_t) == 8, "WTH");'), False, []),
    ("ompi_c_have_builtin_expect", "OPAL_C_HAVE_BUILTIN_EXPECT", prog("", "void *ptr = (void*) 0;\n           if (__builtin_expect (ptr != (void*) 0, 1)) return 0;"), True, []),
    # File-scope statements in the m4: 0 on every compiler, as in PMIx.
    ("ompi_c_have_builtin_prefetch", "OPAL_C_HAVE_BUILTIN_PREFETCH", prog("int ptr;\n           __builtin_prefetch(&ptr,0,0);", ""), True, []),
    ("ompi_c_have_builtin_clz", "OPAL_C_HAVE_BUILTIN_CLZ", prog("int value = 0xffff; /* we know we have 16 bits set */\n             if ((8*sizeof(int)-16) != __builtin_clz(value)) return 0;", ""), True, []),
    ("ompi_have_pthread_mutex_errorcheck_np", "OPAL_HAVE_PTHREAD_MUTEX_ERRORCHECK_NP", prog("#include <pthread.h>", "pthread_mutexattr_settype(NULL, PTHREAD_MUTEX_ERRORCHECK_NP);"), True, ["_GNU_SOURCE"]),
    ("ompi_have_pthread_mutex_errorcheck", "OPAL_HAVE_PTHREAD_MUTEX_ERRORCHECK", prog("#include <pthread.h>", "pthread_mutexattr_settype(NULL, PTHREAD_MUTEX_ERRORCHECK);"), True, []),
    ("ompi_words_bigendian", "WORDS_BIGENDIAN", "int probe[__BYTE_ORDER__ == __ORDER_BIG_ENDIAN__ ? 1 : -1];\n", False, []),
    # mpl (3rd-party/romio341/mpl/configure.ac) compiler checks
    ("mpl_have_var_attribute_aligned", "HAVE_VAR_ATTRIBUTE_ALIGNED", "int foo __attribute__((aligned(32)));\n", False, []),
    ("mpl_have_var_attribute_used", "HAVE_VAR_ATTRIBUTE_USED", "static int foo __attribute__((used));\n", False, []),
    ("mpl_have_func_attribute_fallthrough", "HAVE_FUNC_ATTRIBUTE_FALLTHROUGH",
     prog('#pragma GCC diagnostic error "-Wattributes"', "int x = 1; switch (x) { case 1: x++; __attribute__((fallthrough)); default: x++; }"), False, []),
    ("mpl_have__bool", "HAVE__BOOL", "_Bool b = 1;\n", False, []),
]
symbols = [  # (target, define, symbol, headers)
    ("ompi_have_socket", "OPAL_HAVE_SOCKET", "socket", ["sys/socket.h"]),
    ("ompi_have_gethostbyname", "OPAL_HAVE_GETHOSTBYNAME", "gethostbyname", ["netdb.h"]),
    ("ompi_have_dirname", "OPAL_HAVE_DIRNAME", "dirname", ["libgen.h"]),
    ("ompi_have_va_copy", "OPAL_HAVE_VA_COPY", "va_copy", ["stdarg.h"]),
    ("ompi_have_underscore_va_copy", "OPAL_HAVE_UNDERSCORE_VA_COPY", "__va_copy", ["stdarg.h"]),
    ("ompi_have_sa_restart", "OPAL_HAVE_SA_RESTART", "SA_RESTART", ["signal.h"]),
    ("ompi_have_sched_yield", "OPAL_HAVE_SCHED_YIELD", "sched_yield", ["sched.h"]),
    ("ompi_have_clock_gettime", "OPAL_HAVE_CLOCK_GETTIME", "clock_gettime", ["time.h"]),
    ("ompi_have_backtrace_execinfo", "OPAL_HAVE_BACKTRACE_EXECINFO", "backtrace", ["execinfo.h"]),
    ("ompi_have_unix_byteswap", "HAVE_UNIX_BYTESWAP", "htonl", ["arpa/inet.h"]),
    ("ompi_have__sc_nprocessors_onln", "OPAL_HAVE__SC_NPROCESSORS_ONLN", "_SC_NPROCESSORS_ONLN", ["unistd.h"]),
    # romio (3rd-party/romio341/configure.ac)
    ("romio_have_fsync", "HAVE_FSYNC", "fsync", ["unistd.h"]),
    ("romio_have_ftruncate", "HAVE_FTRUNCATE", "ftruncate", ["unistd.h"]),
    ("romio_have_lstat", "HAVE_LSTAT", "lstat", ["sys/stat.h"]),
    ("romio_have_readlink", "HAVE_READLINK", "readlink", ["unistd.h"]),
    ("romio_have_lseek64", "HAVE_LSEEK64", "lseek64", ["unistd.h"]),
    # mpl
    ("mpl_have_aligned_alloc", "HAVE_ALIGNED_ALLOC", "aligned_alloc", ["stdlib.h"]),
    ("mpl_have_clock_getres", "HAVE_CLOCK_GETRES", "clock_getres", ["time.h"]),
    ("mpl_have_fdopen", "HAVE_FDOPEN", "fdopen", ["stdio.h"]),
    ("mpl_have_getpid", "HAVE_GETPID", "getpid", ["unistd.h"]),
    ("mpl_have_munmap", "HAVE_MUNMAP", "munmap", ["sys/mman.h"]),
    ("mpl_have_sched_yield", "HAVE_SCHED_YIELD", "sched_yield", ["sched.h"]),
    ("mpl_have_sleep", "HAVE_SLEEP", "sleep", ["unistd.h"]),
]
# OPAL_C_GET_ALIGNMENT is AC_CHECK_ALIGNOF (config/c_get_alignment.m4), so each
# OPAL_ALIGNMENT_<T> is exactly the catalog's ALIGNOF_<T>.
ALIGN = {
    "BOOL": "BOOL", "CHAR": "CHAR", "DOUBLE": "DOUBLE", "DOUBLE_COMPLEX": "DOUBLE__COMPLEX",
    # OPAL_ALIGNMENT___FLOAT128 is __float128, OPAL_ALIGNMENT__FLOAT128 _Float128
    "FLOAT": "FLOAT", "__FLOAT128": "__FLOAT128", "_FLOAT128": "_FLOAT128",
    "__FLOAT128_COMPLEX": "__FLOAT128__COMPLEX", "_FLOAT128_COMPLEX": "_FLOAT128__COMPLEX",
    "FLOAT_COMPLEX": "FLOAT__COMPLEX", "INT": "INT", "INT128": "INT128_T", "INT16": "INT16_T",
    "INT32": "INT32_T", "INT64": "INT64_T", "INT8": "INT8_T", "LONG": "LONG",
    "LONG_DOUBLE": "LONG_DOUBLE", "LONG_DOUBLE_COMPLEX": "LONG_DOUBLE__COMPLEX",
    "LONG_LONG": "LONG_LONG", "SHORT": "SHORT", "SHORT_FLOAT": "SHORT_FLOAT",
    "SHORT_FLOAT_COMPLEX": "SHORT_FLOAT__COMPLEX", "SIZE_T": "SIZE_T", "VOID_P": "VOID_P",
    "WCHAR": "WCHAR_T",
}
aliases = [("ompi_alignment_" + k.lower(), "OPAL_ALIGNMENT_" + k, "alignof_" + v.lower()) for k, v in ALIGN.items()]

probe_of = {d: t for t, d, *_ in probes}
probe_of.update({d: t for t, d, *_ in symbols})
probe_of.update({d: t for t, d, _ in aliases})

rules = ['''
# ---- Agent-stage resolutions (bzl-7r9.7), each a decision with its reason.
#
# Probes Open MPI (and the romio/mpl it builds) runs with its OWN snippets —
# compiler attributes, C11 keywords, builtins, pthread constants — and the
# libc symbols it checks for under its own names, so the consumer's toolchain
# answers. As in PMIx/PRRTE: the attribute m4 also greps warnings for
# "ignored", which a probe cannot; and __builtin_prefetch/__builtin_clz put
# statements at file scope, which is why configure answers 0 for both.
# OPAL_ALIGNMENT_<T> is AC_CHECK_ALIGNOF (config/c_get_alignment.m4), so it
# is wired to the catalog's ALIGNOF_<T>.
''']
for t, d, src, link, defs in probes:
    rules.append("check_c_source_compiles(\n    name = %s,\n    define = %s,\n    source = %s,%s%s\n)\n" % (
        sl(t), sl(d), sl(src), "\n    link = True," if link else "",
        ("\n    defines = [%s]," % ", ".join(sl(x) for x in defs)) if defs else ""))
for t, d, sym, hdrs in symbols:
    rules.append("check_symbol_exists(\n    name = %s,\n    define = %s,\n    symbol = %s,\n    headers = [%s],\n    defines = [\"_GNU_SOURCE\"],\n)\n" % (
        sl(t), sl(d), sl(sym), ", ".join(sl(h) for h in hdrs)))
for t, d, p in aliases:
    rules.append("probe_alias(\n    name = %s,\n    define = %s,\n    probe = \"@cc_config//catalog:%s\",\n)\n" % (sl(t), sl(d), p))

# ---- 2. the build machine's identity, rewritten for the module's toolchain --
# Read by ompi_info (omitted on record below) and by mpi.h, which enables
# deprecation attributes when a user compiles with the compiler that built
# Open MPI: the module's is the llvm 22.1.8 its MODULE.bazel declares —
# family 19 (CLANG) and PLATFORM_COMPILER_VERSION_INT(22,1,8) in
# opal/include/opal/opal_portable_platform_real.h.
IDENTITY = {
    "OPAL_CC": '"clang"', "OMPI_CXX": '"clang++"',
    "OPAL_BUILD_PLATFORM_COMPILER_FAMILYID": "19",
    "OPAL_BUILD_PLATFORM_COMPILER_VERSION": str((22 << 16) | (1 << 8) | 8),
    # configure's default is "Open MPI $USER@$HOST Distribution": the
    # conversion host's user and name, dropped.
    "OPAL_PACKAGE_STRING": '"Open MPI Distribution"',
    # The command line OMPI ran romio's configure with (CC='gcc', this host's
    # CFLAGS and sysroot paths), which the romio component reports as an
    # info string. A stamp of the build machine, not a fact about romio.
    "MCA_io_romio341_COMPLETE_CONFIGURE_FLAGS": '""',
}
# The triple configure ran for, per target CPU.
SELECTS = {
    "OPAL_ARCH": '    select({\n        "@platforms//cpu:aarch64": {"OPAL_ARCH": "\\"aarch64-unknown-linux-gnu\\""},\n'
                 '        "//conditions:default": {"OPAL_ARCH": "\\"x86_64-pc-linux-gnu\\""},\n    }) | ',
}


def header_block(name):
    i = s.index('    name = "%s",' % name)
    start = s.rfind("config_header(", 0, i)
    end = s.index("\n)\n", start) + 3
    return start, end


def resolve(rule, item):
    """Close one config header: probes where the toolchain answers, the
    identity rewrites, configure's value for everything else."""
    global s
    vals = item_values(item)
    start, end = header_block(rule)
    block = s[start:end]
    m = re.search(r"    unresolved = \[\n(.*?)    \],\n", block, re.S)
    assert m, rule
    unresolved = re.findall(r'"([A-Za-z_0-9]+)"', m.group(1))
    assert set(unresolved) == set(vals), (rule, set(unresolved) ^ set(vals))
    block = block.replace(m.group(0), "")
    labels = sorted({":" + probe_of[n] for n in unresolved if n in probe_of})
    if labels:
        add = "".join("        %s,\n" % sl(l) for l in labels)
        if "    probes = [\n" in block:
            block = block.replace("    probes = [\n", "    probes = [\n" + add, 1)
        else:
            ti = block.index("    template = ")
            tj = block.index("\n", ti) + 1
            block = block[:tj] + "    probes = [\n" + add + "    ],\n" + block[tj:]
    chosen = {n: IDENTITY.get(n, v) for n, v in vals.items() if n not in probe_of and n not in SELECTS}
    selects = "".join(SELECTS[n] for n in unresolved if n in SELECTS)
    body = "".join("        %s: %s,\n" % (sl(k), sl(v)) for k, v in sorted(chosen.items()))
    note = "        # ---- resolved by the agent stage: configure's value for the default\n" \
           "        # configuration unless probed above or rewritten for the module's toolchain.\n"
    if "    }) | {\n" in block:
        block = block.replace("    }) | {\n", "    }) | " + selects.lstrip() + "{\n" + note + body, 1)
    elif "    values = {\n" in block:
        block = block.replace("    values = {\n", "    values = " + selects.lstrip() + "{\n" + note + body, 1)
    else:
        block = block[:-3] + "\n    values = " + selects.lstrip() + "{\n" + note + body + "    },\n)\n"
    s = s[:start] + block + s[end:]


HEADERS = [
    ("opal_include_opal_config_h", "*-opal-include-opal-config-h-*"),
    ("ompi_include_mpi_h", "*-ompi-include-mpi-h-*"),
    ("ompi_include_ompi_version_h", "*-ompi-include-ompi-version-h-*"),
    ("oshmem_include_oshmem_version_h", "*-oshmem-include-oshmem-version-h-*"),
    ("opal_include_opal_version_h", "*-opal-include-opal-version-h-*"),
    ("3rd_party_romio341_adio_include_romioconf_h", "*-romioconf-h-*"),
    ("3rd_party_romio341_include_mpio_h", "*-romio341-include-mpio-h-*"),
    ("3rd_party_romio341_mpl_include_config_h", "*-mpl-include-config-h-*"),
    # Headers nothing compiles or installs in this configuration: Fortran is
    # off (no Fortran compiler — and the module's toolchain is C/C++ only), so
    # configure's values are its Fortran-off answers and nothing reads them.
    ("ompi_mpi_fortran_configure_fortran_output_h", "*-configure-fortran-output-h-*"),
    ("ompi_mpiext_shortfloat_mpif_h_mpiext_shortfloat_mpifh_h", "*-mpiext-shortfloat-mpifh-h-*"),
    ("ompi_include_mpif_config_h", "*-ompi-include-mpif-config-h-*"),
    ("ompi_mpi_fortran_use_mpi_ignore_tkr_mpi_ignore_tkr_interfaces_h", "*-mpi-ignore-tkr-interfaces-h-*"),
    ("ompi_mpi_fortran_use_mpi_ignore_tkr_mpi_ignore_tkr_file_interfaces_h", "*-mpi-ignore-tkr-file-interfaces-h-*"),
    ("ompi_mpi_fortran_use_mpi_ignore_tkr_mpi_ignore_tkr_removed_interfaces_h", "*-mpi-ignore-tkr-removed-interfaces-h-*"),
    ("ompi_mpi_fortran_use_mpi_f08_mod_mpi_f08_interfaces_h", "*-mpi-f08-interfaces-h-*"),
]
for rule, item in HEADERS:
    resolve(rule, item)

# openpty: the same decision as PMIx (project note 001). opal_pty.c's non-
# openpty branch calls opal_string_copy without its header and does not
# compile; the toolchain's glibc 2.28 defines openpty only in libutil, so the
# catalog's link probe answers absent and selects that branch. What configure
# does on such a system is AC_SEARCH_LIBS([openpty], [util]): recorded as 1,
# with -lutil on the library that compiles opal_pty.c.
start, end = header_block("opal_include_opal_config_h")
block = s[start:end]
assert block.count('        "@cc_config//catalog:have_openpty",\n') == 1
block = block.replace('        "@cc_config//catalog:have_openpty",\n', "")
block = block.replace("        # ---- resolved by the agent stage:", '        "HAVE_OPENPTY": "1",  # see the openpty note above the probes\n        # ---- resolved by the agent stage:', 1)
s = s[:start] + block + s[end:]


def add_linkopt(source, flag, why):
    """-l<lib> on the cc_library that compiles `source`."""
    global s
    at = s.index('        "%s",\n' % source)
    lib_start = s.rfind("cc_library(\n", 0, at)
    lib_end = s.index("\n)\n", at)
    lib = s[lib_start:lib_end]
    entry = '        "%s",  # %s\n' % (flag, why)
    if "    linkopts = [\n" in lib:
        lib = lib.replace("    linkopts = [\n", "    linkopts = [\n" + entry, 1)
    else:
        lib = lib.replace("    visibility = ", "    linkopts = [\n" + entry + "    ],\n    visibility = ", 1)
    s = s[:lib_start] + lib + s[lib_end:]


add_linkopt("opal/util/opal_pty.c", "-lutil", "openpty on the toolchain's glibc 2.28; see opal_config.h")
# POSIX AIO, the same shape: ompi/mca/fbtl/posix/configure.m4 runs
# OPAL_SEARCH_LIBS_COMPONENT([fbtl_posix], [aio_write], [rt]); the conversion
# host's glibc 2.39 has aio_* in libc, the toolchain's 2.28 only in librt.
add_linkopt("ompi/mca/fbtl/posix/fbtl_posix.c", "-lrt", "aio_* on the toolchain's glibc 2.28 (configure.m4 searches -lrt)")

# ---- 3. headers configure (or a make rule) generates (the generated item) ----
FRAMEWORKS = [
    "ompi/mca/bml", "ompi/mca/coll", "ompi/mca/fbtl", "ompi/mca/fcoll", "ompi/mca/fs", "ompi/mca/hook",
    "ompi/mca/io", "ompi/mca/mtl", "ompi/mca/op", "ompi/mca/osc", "ompi/mca/part", "ompi/mca/pml",
    "ompi/mca/sharedfp", "ompi/mca/topo", "ompi/mca/vprotocol", "opal/mca/accelerator", "opal/mca/allocator",
    "opal/mca/backtrace", "opal/mca/btl", "opal/mca/dl", "opal/mca/if", "opal/mca/installdirs",
    "opal/mca/memchecker", "opal/mca/memcpy", "opal/mca/memory", "opal/mca/mpool", "opal/mca/patcher",
    "opal/mca/rcache", "opal/mca/reachable", "opal/mca/shmem", "opal/mca/smsc", "opal/mca/threads",
    "opal/mca/timer",
]
targets_manifest = open(os.path.join(M, "TARGETS")).read()
built = set(re.findall(r"^library libmca_([a-z0-9_]+)\.la ", targets_manifest, re.M))
# config/opal_mca.m4 orders a framework's static list: components WITHOUT a
# configure.m4 first (alphabetically), then the configured ones — by declared
# priority in PRIORITY mode, alphabetically otherwise. The configure.m4 files
# do not ship in the module, so the frameworks where that differs from plain
# alphabetical order record their configured set here.
CONFIGURED = {
    "coll": ["ftagree", "monitoring", "sm"],
    "osc": ["monitoring", "rdma"],
    "installdirs": ["env", "config"],  # PRIORITY mode: env 10, config 0
}


def components(fw):
    comps = sorted(c[len(fw) + 1:] for c in built if c.startswith(fw + "_"))
    configured = [c for c in CONFIGURED.get(fw, []) if c in comps]
    return [c for c in comps if c not in configured] + configured


generated = []
for path in FRAMEWORKS:
    fw = path.rsplit("/", 1)[1]
    comps = components(fw)
    # An empty list is configure `cat`ing an empty file, one blank line each.
    externs = "".join("extern const mca_base_component_t mca_%s_%s_component;\n" % (fw, c) for c in comps) or "\n"
    entries = "".join("  &mca_%s_%s_component, \n" % (fw, c) for c in comps) or "\n"
    content = ("/*\n * $$HEADER$$\n */\n#if defined(c_plusplus) || defined(__cplusplus)\nextern \"C\" {\n#endif\n\n%s\n"
               "const mca_base_component_t *mca_%s_base_static_components[] = {\n%s  NULL\n};\n\n"
               "#if defined(c_plusplus) || defined(__cplusplus)\n}\n#endif\n\n") % (externs, fw, entries)
    name = "static_components_" + path.replace("/", "_")
    generated.append(name)
    rules.append('genrule(\n    name = "%s",\n    outs = ["%s/base/static-components.h"],\n    cmd = """cat > $@ <<\'EOF\'\n%sEOF""",\n)\n' % (name, path, content))

# MPI extensions configure compiled in statically: none (OMPI_MPIEXT_COMPONENTS
# names the ones it CONFIGURED; ompi/mpiext/static-components.h lists none).
generated.append("ompi_mpiext_static_components_h")
rules.append('''genrule(
    name = "ompi_mpiext_static_components_h",
    outs = ["ompi/mpiext/static-components.h"],
    cmd = """cat > $@ <<'EOF'
/*
 * $$HEADER$$
 */
#if defined(c_plusplus) || defined(__cplusplus)
extern "C" {
#endif



const ompi_mpiext_component_t *ompi_mpiext_components[] = {

  NULL
};

#if defined(c_plusplus) || defined(__cplusplus)
}
#endif

EOF""",
)
''')
# ompi/include/Makefile.am symlinks the platform header under its MPI name.
generated.append("ompi_include_mpi_portable_platform_h")
rules.append('''genrule(
    name = "ompi_include_mpi_portable_platform_h",
    srcs = ["opal/include/opal/opal_portable_platform_real.h"],
    outs = ["ompi/include/mpi_portable_platform.h"],
    cmd = "cp $< $@",
)
''')
# Fortran is off, so configure writes the C-constants header empty.
generated.append("ompi_include_mpif_c_constants_h")
rules.append('''genrule(
    name = "ompi_include_mpif_c_constants_h",
    outs = ["ompi/include/mpif-c-constants.h"],
    cmd = "touch $@",
)
''')
# configure writes a dummy config.h so the embedded treematch does not pick up
# hwloc's (see the comment it carries).
generated.append("treematch_config_h")
rules.append('''genrule(
    name = "treematch_config_h",
    outs = ["3rd-party/treematch/config.h"],
    cmd = """cat > $@ <<'EOF'
/*
 * This file is automatically generated by configure.  Edits will be lost
 *
 * This is an dummy config.h in order to prevent the embedded treematch from using
 * the config.h from the embedded hwloc
 *
 * see https://github.com/open-mpi/ompi/pull/6185#issuecomment-458807930
 */
EOF""",
)
''')
# ompi/mca/osc/monitoring/configure.m4 writes a template instance per osc
# component it KNOWS (configured: portals4, rdma, ucx; not: sm), built or not.
OSC_KNOWN = ["portals4", "rdma", "ucx", "sm"]
osc = ("/* ompi/mca/osc/monitoring/osc_monitoring_template_gen.h\n *\n * This file was generated from ompi/mca/osc/monitoring/configure.m4\n *\n * DO NOT EDIT THIS FILE.\n *\n */\n"
       "/*\n * Copyright (c) 2017-2018 Inria.  All rights reserved.\n * $$COPYRIGHT$$\n *\n * Additional copyrights may follow\n *\n * $$HEADER$$\n */\n\n"
       "#ifndef MCA_OSC_MONITORING_GEN_TEMPLATE_H\n#define MCA_OSC_MONITORING_GEN_TEMPLATE_H\n\n"
       "#include \"ompi_config.h\"\n#include \"ompi/mca/osc/osc.h\"\n#include \"ompi/mca/osc/monitoring/osc_monitoring_template.h\"\n\n"
       "/************************************************************/\n/* Include template generating macros and inlined functions */\n\n"
       + "".join("OSC_MONITORING_MODULE_TEMPLATE_GENERATE(%s)\n" % c for c in OSC_KNOWN)
       + "\n/************************************************************/\n\ntypedef struct {\n    const char * name;\n    ompi_osc_base_module_t * (*fct) (ompi_osc_base_module_t *);\n} osc_monitoring_components_list_t;\n\n"
       "static const osc_monitoring_components_list_t osc_monitoring_components_list[] = {\n"
       + "".join("    { .name = \"%s\", .fct = OSC_MONITORING_SET_TEMPLATE_FCT_NAME(%s) },\n" % (c, c) for c in OSC_KNOWN)
       + "    { .name = NULL, .fct = NULL }\n};\n\n#endif /* MCA_OSC_MONITORING_GEN_TEMPLATE_H */\n")
generated.append("osc_monitoring_template_gen_h")
rules.append('genrule(\n    name = "osc_monitoring_template_gen_h",\n    outs = ["ompi/mca/osc/monitoring/osc_monitoring_template_gen.h"],\n    cmd = """cat > $@ <<\'EOF\'\n%sEOF""",\n)\n' % osc)
# mpl's configure runs its own perl script to copy config.h under an MPL_
# prefix (AC_CONFIG_COMMANDS prefix-config); run over the config_header rule's
# output, so every probe answer in it is the consumer's.
generated.append("mpl_mplconfig_h")
rules.append('''genrule(
    name = "mpl_mplconfig_h",
    srcs = [
        "3rd-party/romio341/mpl/confdb/cmd_prefix_config_h.pl",
        ":3rd_party_romio341_mpl_include_config_h",
    ],
    outs = ["3rd-party/romio341/mpl/include/mplconfig.h"],
    cmd = "perl $(location 3rd-party/romio341/mpl/confdb/cmd_prefix_config_h.pl) MPL $(location :3rd_party_romio341_mpl_include_config_h) $@",
)
''')

# Every target lists every config header beside its sources; the generated
# headers ride along the same way, so each compile has them as inputs.
rep('        ":opal_include_opal_config_h",\n',
    '        ":opal_include_opal_config_h",\n' + "".join('        ":%s",\n' % g for g in generated),
    count=s.count('        ":opal_include_opal_config_h",\n'))

rep('load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n',
    'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n'
    'load("@cc_config//cc_config:probe.bzl", "check_c_source_compiles", "check_symbol_exists", "probe_alias")\n')
first = s.index("\nconfig_header(\n")
s = s[:first] + "\n" + "".join(rules) + s[first:]

# ---- 4. libmpi and libopen-pal absorb their convenience archives ------------
def absorb(lib, expected):
    global s
    i = s.index('    name = "%s",' % lib)
    ds = s.index("    deps = [\n", i)
    de = s.index("    ],\n", ds)
    # Convenience archives only: libmpi also links libopen-pal, which is its
    # own shared library (it has a _shared rule) and must stay one.
    archives = [a for a in re.findall(r'"(:[^"]+\.la)"', s[ds:de])
                if 'name = "%s_shared",' % a[1:] not in s]
    # Transitively: libtool absorbed a convenience archive's own archives
    # too — libopen-pal_core.la carries libopalutil_core.la, where
    # opal_sha256_* live, and the test programs linking libopen-pal need them.
    seen = list(archives)
    while archives:
        a = archives.pop()
        j = s.find('    name = "%s",' % a[1:])
        k = s.find("\n)\n", j)
        m = re.search(r"    deps = \[\n(.*?)    \],", s[j:k], re.S)
        for d in re.findall(r'"(:[^"]+\.la)"', m.group(1)) if m else []:
            if d not in seen and 'name = "%s_shared",' % d[1:] not in s:
                seen.append(d)
                archives.append(d)
    archives = seen
    assert len(archives) >= expected, (lib, len(archives))
    rep('''cc_shared_library(
    name = "%s_shared",
    deps = [
        ":%s",
    ],''' % (lib, lib), '''# Every convenience archive %s links, absorbed into THIS shared object
# and exported through it — what automake's noinst_LTLIBRARIES pulled into
# %s means.
cc_shared_library(
    name = "%s_shared",
    deps = [
        ":%s",
''' % (lib, lib, lib, lib) + "".join('        "%s",\n' % a for a in archives) + '''    ],''')
    return len(archives)


n_mpi = absorb("libmpi.la", 61)
n_pal = absorb("libopen-pal.la", 36)

# ---- 5. build stamps: fixed literals; ompi_info omitted ---------------------
STAMPS = {"OMPI_BUILD_DATE": "unknown", "OMPI_BUILD_HOST": "bazel", "OMPI_BUILD_USER": "bazel"}
for name, literal in STAMPS.items():
    pat = re.compile(r'"%s=\'\\"[^\n]*?\\"\'",' % name)
    assert len(pat.findall(s)) >= 1, name
    s = pat.sub('"%s=\'\\"%s\\"\'",  # a build stamp; the recipe computed it with the shell' % (name, literal), s)
t = targets_manifest.rstrip("\n") + "\nomitted ompi_info prints the configure/build stamps (date, host, user, compiler), which differ from the ground truth by construction\n"
open(os.path.join(M, "TARGETS"), "w").write(t)

# Two comparisons whose stdout is timings by construction: reduce_local and
# test_overhead are benchmarks (seconds per operation), and the module's
# unoptimised build runs them slower than the -O3 ground truth, past the
# comparison's window.
t = open(os.path.join(M, "TARGETS")).read().rstrip("\n")
t += "\nomitted reduce_local a benchmark: prints elapsed times, which differ run to run by construction"
t += "\nomitted test_overhead a benchmark: prints elapsed times, which differ run to run by construction\n"
open(os.path.join(M, "TARGETS"), "w").write(t)

# ---- 5b. registered tests that need their harness ---------------------------
import shutil
here = os.path.dirname(os.path.abspath(__file__))
for runner in ["run_asm_test.sh", "run_dlopen_test.sh"]:
    shutil.copy(os.path.join(here, runner), os.path.join(M, runner))
    os.chmod(os.path.join(M, runner), 0o755)


def retarget_test(name, srcs, args, data):
    global s
    i = s.index('    name = "%s_test",' % name)
    start = s.rfind("sh_test(\n", 0, i)
    end = s.index("\n)\n", i) + 3
    block = s[start:end]
    env = re.search(r"    env = \{[^}]*\},\n", block)
    s = s[:start] + ('sh_test(\n    name = "%s_test",\n    srcs = ["%s"],\n    args = [%s],\n    data = [%s],\n%s)\n'
                     % (name, srcs, ", ".join(sl(a) for a in args), ", ".join(sl(d) for d in data),
                        env.group(0) if env else "")) + s[end:]


# test/asm runs every program through run_tests (TESTS_ENVIRONMENT), which
# passes a thread count; bare, they report "Incorrect number of arguments".
for prog in ["atomic_barrier", "atomic_barrier_noinline", "atomic_cmpset", "atomic_cmpset_noinline",
             "atomic_math", "atomic_math_noinline", "atomic_spinlock", "atomic_spinlock_noinline"]:
    retarget_test(prog, "run_asm_test.sh", [prog], [":" + prog])
# dlopen_test needs the debugger plugin beside it, as in make check.
retarget_test("dlopen_test", "run_dlopen_test.sh", [], [":dlopen_test", ":libompi_dbg_msgq.la_shared"])

# ---- 6. the registered test the module does not build -----------------------
# mpl's own `strsep` (3rd-party/romio341/mpl/Makefile.am, TESTS) is declared
# but never built or run by Open MPI's make check, which does not descend into
# mpl: upstream's 44 tests do not include it. Not reproduced, by that record.

open(B, "w").write(s)
mb = os.path.join(M, "MODULE.bazel")
ms = open(mb).read()
if 'name = "platforms"' not in ms:
    ms = ms.replace('bazel_dep(name = "cc_config", version = "0.0.0")\n',
                    'bazel_dep(name = "cc_config", version = "0.0.0")\nbazel_dep(name = "platforms", version = "1.1.0")\n')
    open(mb, "w").write(ms)
for f in os.listdir(NA):
    if f.endswith(".md"):
        os.remove(os.path.join(NA, f))
print("resolved openmpi: %d probes, %d symbols, %d aliases, %d config headers, %d generated, archives %d+%d" % (
    len(probes), len(symbols), len(aliases), len(HEADERS), len(generated), n_mpi, n_pal))
