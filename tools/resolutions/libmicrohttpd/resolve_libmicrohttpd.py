#!/usr/bin/env python3
"""Agent-stage resolution for the converted libmicrohttpd module.

    python3 tools/resolutions/libmicrohttpd/resolve_libmicrohttpd.py W/fixtures/libmicrohttpd

All-or-nothing on its anchors. Decisions read from libmicrohttpd 1.0.1's
configure.ac, never from this host's MHD_config.h. Needs the converted zlib
module beside it in the workspace (item 002).
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

# ---- item 001: MHD_config.h -------------------------------------------------
LOAD = 'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n'
rep(LOAD, LOAD + 'load("@cc_config//cc_config:probe.bzl", "check_c_source_compiles", '
    '"check_struct_member", "check_symbol_exists", "check_type_exists", "check_type_size")\n')

rules = []
labels = []
def probe(kind, name, **attrs):
    body = "".join("    %s = %s,\n" % (k, sl(v) if isinstance(v, str) else
                   "[" + ", ".join(sl(x) for x in v) + "]") for k, v in sorted(attrs.items()))
    body = body.replace("    link = \"True\",\n", "    link = True,\n")
    rules.append("%s(\n    name = %s,\n%s)\n" % (kind, sl(name), body))
    labels.append(":" + name)

G = ["_GNU_SOURCE"]
# MHD_CHECK_FUNC / AC_CHECK_FUNCS of POSIX and libc functions.
for define, sym, hdrs, defs in [
    ("HAVE_CALLOC", "calloc", ["stdlib.h"], []),
    ("HAVE_GETPID", "getpid", ["unistd.h"], []),
    ("HAVE_GMTIME_R", "gmtime_r", ["time.h"], []),
    ("HAVE_LSEEK64", "lseek64", ["unistd.h"], G),
    ("HAVE_MEMMEM", "memmem", ["string.h"], G),
    ("HAVE_PREAD", "pread", ["unistd.h"], []),
    ("HAVE_PREAD64", "pread64", ["unistd.h"], G),
    ("HAVE_PTHREAD_SIGMASK", "pthread_sigmask", ["signal.h"], []),
    ("HAVE_RANDOM", "random", ["stdlib.h"], []),
    ("HAVE_SCHED_GETAFFINITY", "sched_getaffinity", ["sched.h"], G),
    ("HAVE_SENDFILE64", "sendfile64", ["sys/sendfile.h"], G),
    ("HAVE_TIMESPEC_GET", "timespec_get", ["time.h"], []),
    ("HAVE_WRITEV", "writev", ["sys/uio.h"], []),
    # glibc's CPU_SET macros, checked for the perf_replies tool.
    ("HAVE_CPU_COUNT", "CPU_COUNT", ["sched.h"], G),
    ("HAVE_CPU_COUNT_S", "CPU_COUNT_S", ["sched.h"], G),
    # --enable-epoll=auto: on when <sys/epoll.h> provides epoll.
    ("EPOLL_SUPPORT", "epoll_create1", ["sys/epoll.h"], []),
]:
    kw = dict(define=define, symbol=sym, headers=hdrs)
    if defs:
        kw["defines"] = defs
    probe("check_symbol_exists", "mhd_" + define.lower(), **kw)

# Compiler facts, with configure's own snippets.
probe("check_c_source_compiles", "mhd_have_c_alignof", define="HAVE_C_ALIGNOF",
      source="#include <stdalign.h>\nint main(void) {\n  int var1[(alignof(int) >= 2) ? 1 : -1];\n  int var2[alignof(unsigned int) - 1];\n  int var3[(alignof(char) > 0) ? 1 : -1];\n  int var4[(alignof(long) >= 4) ? 1 : -1];\n  var1[0] = var2[0] = var3[0] = 0;\n  var4[0] = 1;\n  return var1[0] + var2[0] + var3[0] == var4[0];\n}\n")
for bits in ("32", "64"):
    probe("check_c_source_compiles", "mhd_have___builtin_bswap" + bits,
          define="MHD_HAVE___BUILTIN_BSWAP" + bits, link="True",
          source="#include <stdint.h>\nint main(void) {\n  uint%s_t a = 1; uint%s_t b = __builtin_bswap%s(a); a = b; (void) a;\n  return 0;\n}\n" % (bits, bits, bits))
probe("check_c_source_compiles", "mhd_words_bigendian", define="WORDS_BIGENDIAN",
      source="int probe[__BYTE_ORDER__ == __ORDER_BIG_ENDIAN__ ? 1 : -1];\n")
probe("check_type_exists", "mhd_have_builtin_type_bool", define="HAVE_BUILTIN_TYPE_BOOL",
      type="bool", headers=["stdbool.h"])
probe("check_struct_member", "mhd_have_struct_sockaddr_storage_ss_len",
      define="HAVE_STRUCT_SOCKADDR_STORAGE_SS_LEN", struct="struct sockaddr_storage",
      member="ss_len", headers=["sys/types.h", "sys/socket.h"])
probe("check_type_size", "mhd_sizeof_struct_timeval_tv_sec", define="SIZEOF_STRUCT_TIMEVAL_TV_SEC",
      type="((struct timeval *) 0)->tv_sec", headers=["sys/time.h"])
probe("check_type_size", "mhd_sizeof_uint64_t", define="SIZEOF_UINT64_T",
      type="uint64_t", headers=["stdint.h"])
probe("check_type_size", "mhd_sizeof_unsigned_long_long", define="SIZEOF_UNSIGNED_LONG_LONG",
      type="unsigned long long")

rep('config_header(\n    name = "MHD_config_h",',
    "# Toolchain facts MHD_config.h needs, asked of the consumer's compiler.\n"
    + "\n".join(rules) + '\nconfig_header(\n    name = "MHD_config_h",')

U = ""
VALUES = {
    # Project options at the defaults the ground truth was built with; each
    # also selects sources, so the module's source list matches these.
    "BAUTH_SUPPORT": ("1", "--enable-bauth default"),
    "COOKIE_SUPPORT": ("1", "--enable-cookie default"),
    "DAUTH_SUPPORT": ("1", "--enable-dauth default"),
    "HAVE_MESSAGES": ("1", "--enable-messages default"),
    "HAVE_POSTPROCESSOR": ("1", "--enable-postprocessor default"),
    "UPGRADE_SUPPORT": ("1", "--enable-httpupgrade default"),
    "MHD_DAUTH_DEF_MAX_NC_": ("1000", "--enable-dauth default nonce count"),
    "MHD_DAUTH_DEF_TIMEOUT_": ("90", "--enable-dauth default timeout"),
    "MHD_MD5_SUPPORT": ("1", "--enable-md5 default (builtin)"),
    "MHD_SHA256_SUPPORT": ("1", "--enable-sha256 default (builtin)"),
    "MHD_SHA512_256_SUPPORT": ("1", "--enable-sha512-256 default"),
    "MHD_MD5_TLSLIB": (U, "builtin MD5, no TLS library"),
    "MHD_SHA256_TLSLIB": (U, "builtin SHA-256, no TLS library"),
    "HAVE_ASSERT": (U, "--enable-asserts defaults to no"),
    "_MHD_HEAVY_TESTS": (U, "--enable-heavy-tests defaults to no"),
    "_MHD_VHEAVY_TESTS": (U, "--enable-heavy-tests defaults to no"),
    "MHD_NO_THREAD_NAMES": (U, "--enable-thread-names default; thread names are on"),
    # Optional libraries the original configure did not find, so the build
    # it produced has no HTTPS, no libcurl-driven tests and no libmagic
    # example. Converting GnuTLS/libcurl would add them; the ground truth
    # has neither, and the module matches it.
    "HTTPS_SUPPORT": (U, "GnuTLS not found by the original build"),
    "MHD_HTTPS_REQUIRE_GCRYPT": (U, "no HTTPS"),
    "HAVE_GNUTLS_CHECK_VERSION": (U, "no GnuTLS"),
    "HAVE_LIBCURL": (U, "libcurl not found by the original build"),
    "curl_free": (U, "libcurl.m4's fallback, only with libcurl"),
    "MHD_HAVE_LIBMAGIC": (U, "libmagic not found by the original build"),
    "HAVE_MAGIC_OPEN": (U, "libmagic not found by the original build"),
    # Sanitizer checks, all inside --enable-sanitizers (default no).
    "FUNC_ATTR_NOSANITIZE_WORKS": (U, "--enable-sanitizers defaults to no"),
    "FUNC_ATTR_PTRCOMPARE_WORKS": (U, "--enable-sanitizers defaults to no"),
    "FUNC_ATTR_PTRSUBTRACT_WORKS": (U, "--enable-sanitizers defaults to no"),
    "FUNC_PTRCOMPARE_CAST_WORKAROUND_WORKS": (U, "--enable-sanitizers defaults to no"),
    "MHD_ASAN_ACTIVE": (U, "--enable-sanitizers defaults to no"),
    "MHD_ASAN_POISON_ACTIVE": (U, "--enable-sanitizers defaults to no"),
    "HAVE___ASAN_ADDRESS_IS_POISONED": (U, "--enable-sanitizers defaults to no"),
    "HAVE___ASAN_REGION_IS_POISONED": (U, "--enable-sanitizers defaults to no"),
    # Host-OS branches of configure's AS_CASE("$host_os"); LINUX is set.
    "AC_APPLE_UNIVERSAL_BUILD": (U, "Apple universal builds"),
    "CYGWIN": (U, "host_os branch"), "FREEBSD": (U, "host_os branch"),
    "GNU_HURD": (U, "host_os branch"), "MINGW": (U, "host_os branch"),
    "NETBSD": (U, "host_os branch"), "OPENBSD": (U, "host_os branch"),
    "OS390": (U, "host_os branch"), "OSX": (U, "host_os branch"),
    "OTHEROS": (U, "host_os branch"), "SOLARIS": (U, "host_os branch"),
    "SOMEBSD": (U, "host_os branch"), "WINDOWS": (U, "host_os branch"),
    "_REENTRANT": (U, "set only in the Solaris host_os branch"),
    "MHD_USE_W32_THREADS": (U, "MHD_USE_POSIX_THREADS is set"),
    # Other platforms' APIs: Darwin, FreeBSD, NetBSD, Solaris, IBM i, HP-UX,
    # VxWorks, Mach, W32.
    "HAVE_C11_GMTIME_S": (U, "C11 Annex K gmtime_s; not in glibc/musl/Darwin"),
    "HAVE_W32_GMTIME_S": (U, "W32"),
    "HAVE_WSAPOLL": (U, "W32"),
    "HAVE_CLOCK_GET_TIME": (U, "Mach"),
    "HAVE_GETHRTIME": (U, "Solaris"),
    "HAVE_CPUSET_GETAFFINITY": (U, "FreeBSD"),
    "HAVE_SCHED_GETAFFINITY_NP": (U, "NetBSD"),
    "HAVE_PSTAT_GETDYNAMIC": (U, "HP-UX"),
    "HAVE_VXCPUENABLEDGET": (U, "VxWorks"),
    "HAVE_DARWIN_SENDFILE": (U, "Darwin"),
    "HAVE_FREEBSD_SENDFILE": (U, "FreeBSD"),
    "HAVE_SOLARIS_SENDFILE": (U, "Solaris"),
    "HAVE_PTHREAD_ATTR_SETNAME_NP_IBMI": (U, "IBM i"),
    "HAVE_PTHREAD_ATTR_SETNAME_NP_NETBSD": (U, "NetBSD"),
    "HAVE_PTHREAD_SETNAME_NP_DARWIN": (U, "Darwin"),
    "HAVE_PTHREAD_SETNAME_NP_NETBSD": (U, "NetBSD"),
    "HAVE_PTHREAD_SET_NAME_NP_FREEBSD": (U, "FreeBSD"),
    # Alternatives configure checks only when an earlier choice failed; the
    # earlier choice holds here (random, __func__, eventfd, sysconf).
    "HAVE_RAND": (U, "checked only without random()"),
    "HAVE___FUNCTION__": (U, "checked only without __func__"),
    "HAVE___PRETTY_FUNCTION__": (U, "checked only without __func__"),
    "MHD_FUNC_CPU_COUNT_S_GETS_CPUS": (U, "CPU_COUNT_S takes a size, as glibc's does"),
    "MHD_USE_PAGESIZE_MACRO": (U, "page size from sysconf"),
    "MHD_USE_PAGESIZE_MACRO_STATIC": (U, "page size from sysconf"),
    "MHD_USE_PAGE_SIZE_MACRO": (U, "page size from sysconf"),
    "MHD_USE_PAGE_SIZE_MACRO_STATIC": (U, "page size from sysconf"),
    # The inter-thread channel is ONE of eventfd/pipe/socketpair, chosen in
    # that order by run checks; eventfd is the Linux answer. A non-Linux
    # consumer needs the pipe branch, which this does not express.
    "_MHD_ITC_EVENTFD": ("1", "first ITC choice; Linux"),
    "_MHD_ITC_PIPE": (U, "eventfd chosen"),
    "_MHD_ITC_SOCKETPAIR": (U, "eventfd chosen"),
    "HAVE_PIPE2_FUNC": (U, "checked only for the pipe ITC"),
    # AC_RUN_IFELSE checks: behaviour of the running system, which no
    # compile probe can ask. POSIX behaviour, as the ground truth found.
    "MHD_USE_GETSOCKNAME": ("1", "run check: getsockname reports the bound port"),
    "MHD_USE_SYS_TSEARCH": ("1", "run check; also selects no bundled tsearch.c"),
    "USE_IPV6_TESTING": ("1", "run check: the kernel has IPv6, for tests"),
    # FD_SETSIZE as configure computed it with AC_COMPUTE_INT, and its
    # override check failed. 1024 on glibc, musl and the BSDs alike.
    "MHD_SYS_FD_SETSIZE_": ("1024", "FD_SETSIZE"),
    "HAS_FD_SETSIZE_OVERRIDABLE": (U, "configure's override check failed"),
    # Compiler keywords chosen from a ladder; these are the C11 rungs.
    "_MHD_EXTERN": ('__attribute__((visibility("default"))) extern', "visibility works"),
    "_MHD_NORETURN": ("_Noreturn", "C11 keyword"),
    # stdbool.h supplies bool/true/false, so configure defines none of them.
    "bool": (U, "from stdbool.h"),
    "true": (U, "from stdbool.h"),
    "false": (U, "from stdbool.h"),
    "__STDC_NO_VLA__": (U, "compiler-predefined, never configure's"),
    "PTHREAD_CREATE_JOINABLE": (U, "AX_PTHREAD fallback for a missing name"),
    "_FILE_OFFSET_BITS": (U, "AC_SYS_LARGEFILE, 32-bit only"),
    "_LARGEFILE_SOURCE": (U, "AC_FUNC_FSEEKO, only where fseeko needs it"),
    "_LARGE_FILES": (U, "AC_SYS_LARGEFILE, AIX only"),
    "LT_OBJDIR": ('".libs/"', "libtool's"),
}
for n in [l for l in open(glob.glob(os.path.join(M, "needs_attention", "001-*.md"))[0]).read().split("\n")
          if l.startswith("- `LIBCURL_")]:
    VALUES[re.match(r"- `([^`]+)`", n).group(1)] = (U, "libcurl not found by the original build")

start = s.index('    name = "MHD_config_h",')
m = re.compile(r"    unresolved = \[\n(.*?)    \],\n", re.S).search(s, start)
unresolved = re.findall(r'"([A-Za-z_0-9]+)"', m.group(1))
probed = {re.search(r'define = "([A-Z_0-9]+)"', r).group(1) for r in rules}
missing = [n for n in unresolved if n not in VALUES and n not in probed]
extra = [n for n in list(VALUES) + list(probed) if n not in unresolved]
assert not missing and not extra, (missing, extra)
s = s[:m.start()] + s[m.end():]

# Probes join the rule's list; values join its fixed dict.
end = s.index("\n)\n", start)
block = s[start:end]
i = block.index("    probes = [\n")
j = block.index("    ],\n", i)
block = block[:j] + "".join('        "%s",\n' % l for l in labels) + block[j:]
entries = "".join("        %s: %s,  # %s\n" % (sl(k), sl(v), why) for k, (v, why) in sorted(VALUES.items()))
block = rep("    }) | {\n", "    }) | {\n" + entries, block)
s = s[:start] + block + s[end:]

# ---- item 002: -lz ------------------------------------------------------------
# zlib is a corpus project converted into its own module (`zlib`, beside this
# one in the workspace), so the two compression examples depend on it rather
# than on whatever libz the build machine has.
for target in ("http_compression", "http_chunked_compression"):
    start = s.index('    name = "%s",' % target)
    end = s.index("\n)\n", start)
    block = rep("    deps = [\n", '    deps = [\n        "@zlib//:zlib",\n', s[start:end])
    s = s[:start] + block + s[end:]

MOD = os.path.join(M, "MODULE.bazel")
mod = open(MOD).read()
anchor = 'bazel_dep(name = "bazel_skylib", version = "1.7.1")\n'
assert mod.count(anchor) == 1
mod = mod.replace(anchor, anchor + 'bazel_dep(name = "zlib", version = "1.3.2")  # the converted zlib module (needs_attention 002)\n')
open(MOD, "w").write(mod)

# ---- perf_replies: no comparison to make ------------------------------------
# A benchmark SERVER: run bare it binds a random port and waits for ENTER, so
# the harness can only kill it, and it warns on stderr when built without
# __OPTIMIZE__ — the original had autoconf's default -O2, while optimisation
# in Bazel is the consumer's --compilation_mode. Built, not compared.
rep('''cc_binary(
    name = "perf_replies",''', '''# perf_replies is built but its ground-truth comparison is omitted (TARGETS):
# an interactive benchmark server whose output is a random port and a
# warning that depends on the optimisation level.
cc_binary(
    name = "perf_replies",''')
with open(os.path.join(M, "TARGETS"), "a") as t:
    t.write("omitted perf_replies interactive benchmark server; output is a random port and an -O-dependent warning\n")

open(B, "w").write(s)
for f in glob.glob(os.path.join(M, "needs_attention", "00[12]-*.md")):
    os.remove(f)
