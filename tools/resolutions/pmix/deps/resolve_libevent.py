# Adapted for the PMIx workspace: module path from argv, and '' (not 0)
# for 'undefined' since the config-header value fix of 2026-09-19.
import re, os, glob, json, shutil
import sys
M = sys.argv[1]
import os
R = os.path.dirname(os.path.abspath(__file__))
p = f"{M}/BUILD.bazel"; s = open(p).read()
def rep(old, new, count=1):
    global s; n = s.count(old); assert n == count, f"{n} != {count}: {old[:100]}"; s = s.replace(old, new)
def sl(v): return json.dumps(v)
def names_of(pattern):
    f = glob.glob(f"{M}/needs_attention/{pattern}")[0]
    resolved, unknown = {}, []
    for line in open(f):
        m = re.match(r"^- `([A-Za-z_0-9]+)`(?: — configure resolved this to `(.*)`)?", line)
        if not m: continue
        if m.group(2) is not None: resolved[m.group(1)] = m.group(2)
        else: unknown.append(m.group(1))
    return resolved, unknown

ALIASES = {"HAVE_DEVPOLL": "have_sys_devpoll_h", "HAVE_EVENT_PORTS": "have_port_create",
           "HAVE_WORKING_KQUEUE": "have_kqueue", "HAVE_EPOLL": "have_epoll_ctl"}
PROBES = ["@cc_config//catalog:have_getaddrinfo", "@cc_config//catalog:sizeof_pthread_t"]
# Decisions that override what configure resolved on THIS host.
DECIDED = {"HAVE_OPENSSL_SSL_H": "", "HAVE_LIBZ": "", "HAVE_ZLIB_H": ""}
ONE = {"HAVE_PTHREAD", "HAVE_GETHOSTBYNAME_R_6_ARG"}

rep('load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n',
    'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\nload("@cc_config//cc_config:probe.bzl", "probe_alias")\n')
# No test rule was rendered (every registered test escalated), so the load is
# absent; MODULE.bazel already depends on rules_shell for exactly this reason.
if 'sh_test.bzl' not in s:
    s = s.replace('load("@rules_cc//cc:cc_shared_library.bzl", "cc_shared_library")\n',
                  'load("@rules_cc//cc:cc_shared_library.bzl", "cc_shared_library")\nload("@rules_shell//shell:sh_test.bzl", "sh_test")\n', 1)

# The frontend surfaced --enable-openssl as this module's own option, with
# configure's default (on). The decision lives on the option: off by
# default, because nothing this module can depend on provides OpenSSL.
rep('''bool_flag(
    name = "enable_openssl",
    build_setting_default = True,
)''', '''# Off by default (configure's default is on): no converted module provides
# OpenSSL, so the default build must not need it. A consumer with a
# converted OpenSSL sets --@libevent//:enable_openssl=true — and must also
# restore libevent_openssl.la, sample_https-client, sample_le-proxy and
# test/regress_ssl.c, which are omitted below for the same reason.
bool_flag(
    name = "enable_openssl",
    build_setting_default = False,
)''')
first = s.index("config_header(\n")
s = s[:first] + '''# ---- Agent-stage resolutions (bzl-7r9.12), each a decision with its reason.
#
# NO OPENSSL. A default configure found this host's OpenSSL, so the build
# produced libevent_openssl.la, sample_https-client, sample_le-proxy, and a
# test/regress with regress_ssl.c (the -lssl/-lcrypto items). No converted
# module provides OpenSSL, and linking the host's copy is what this module
# exists to rule out, so the module replicates a build WITHOUT it: those
# three targets are omitted, regress_ssl.c leaves test/regress, the
# enable_openssl option defaults off (HAVE_OPENSSL follows it) and
# HAVE_OPENSSL_SSL_H is undefined. Converting OpenSSL is a
# separate project; when it exists, re-running this conversion with it as a
# dependency brings the four back with no hand edits.
#
# NO ZLIB, for now. test/regress links -lz for regress_zlib.c. zlib IS a
# converted corpus module, but the CMake frontend does not yet install its
# ground-truth build for a dependent to configure against (bzl-7r9.8), so
# the edge cannot be resolved by path yet. Until then regress builds without
# regress_zlib.c and HAVE_LIBZ is undefined — what libevent's build does on
# a machine without zlib.
#
# include/event2/event-config.h is config.h with every macro renamed under
# an EVENT__ prefix, made at MAKE time by the project's own sed script
# (Makefile.am: `$(SED) -f make-event-config.sed < config.h > $@`), which is
# why no config_header rule produced it. Run over the config_header rule's
# OUTPUT so every probe answer is the consumer's toolchain's; PUBLIC
# (nodist_include_event2_HEADERS installs it), so in libevent_core.la's hdrs.
genrule(
    name = "event_config_h",
    srcs = [
        "make-event-config.sed",
        ":config_h",
    ],
    outs = ["include/event2/event-config.h"],
    cmd = "sed -f $(location make-event-config.sed) < $(location :config_h) > $@",
)

# Facts configure DERIVES from a probe under its own name (configure.ac):
# HAVE_DEVPOLL iff sys/devpoll.h, HAVE_EVENT_PORTS iff port_create,
# HAVE_EPOLL iff epoll_ctl, and HAVE_WORKING_KQUEUE standing `kqueue exists`
# in for `kqueue works` (the run test guards an old NetBSD no toolchain here
# targets). Republished so each consumer's toolchain answers.
''' + "".join(f'''
probe_alias(
    name = "{n.lower()}",
    define = "{n}",
    probe = "@cc_config//catalog:{pr}",
)
''' for n, pr in ALIASES.items()) + '''
# The ten registered tests (test_runner_<backend>) are make targets running
# test/test.sh, which drives test/regress and its sibling programs from one
# directory with EVENT_NO<BACKEND> in the environment. Reproduced below
# through run_layout_script_test.sh, which lays the binaries out the way
# automake does and runs the script with the recipe's own arguments.

''' + s[first:]

# ---- config_h
ci = s.index('config_header(\n    name = "config_h",'); ce = s.index("\n)\n", ci) + 1
b = s[ci:ce]
resolved, unknown = names_of("001-*config-h-*")
pi = b.index("    probes = [\n"); pj = b.index("    ],\n", pi)
b = b[:pj] + "".join(f'        ":{n.lower()}",\n' for n in ALIASES) + "".join(f'        "{pr}",\n' for pr in PROBES if f'"{pr}"' not in b) + b[pj:]
ui = b.index("    unresolved = [\n"); uj = b.index("    ],\n", ui) + len("    ],\n"); b = b[:ui] + b[uj:]
# frozen host answers the probes now supply, and the mis-rendered select
b = b.replace('''    }) | select({
        ":enable_thread_support_on": {"SIZEOF_PTHREAD_T": "1"},
        "//conditions:default": {},
''', "")
for n in ["HAVE_EPOLL", "HAVE_GETADDRINFO"]:
    b = re.sub(r'        "%s": "[^"]*",\n' % n, "", b)
for n, v in DECIDED.items():
    b = re.sub(r'        "%s": "[^"]*",\n' % n, "", b)
lines = []
for n in unknown:
    if n in ALIASES: continue
    lines.append((n, DECIDED.get(n, "1" if n in ONE else "")))
for n, v in resolved.items():
    if n in ALIASES: continue
    lines.append((n, DECIDED.get(n, v)))
for n, v in DECIDED.items():
    if n not in dict(lines) and n in b: pass
note = '''        # ---- resolved by the agent stage. "" = undefined: the DISABLE_ build
        # options (a default configure enables debug mode, malloc replacement
        # and thread support); the two other gethostbyname_r arities (glibc has
        # the 6-argument form, which is 1); HAVE_LIBWS2_32 (Windows);
        # PTHREAD_CREATE_JOINABLE (defined only where the platform spells it
        # PTHREAD_CREATE_UNDETACHED); the AC_USE_SYSTEM_EXTENSIONS names left
        # for other platforms; the fallback typedefs (const, inline, pid_t,
        # size_t, socklen_t, ssize_t: no C99 toolchain lacks them); and the
        # OpenSSL/zlib names, per the notes above. HAVE_PTHREAD is 1 (ax_pthread
        # finds it on every toolchain this module targets). The rest are what
        # configure resolved, quoted from the escalation.
'''
vi = b.index("    }) | {\n") + len("    }) | {\n")
b = b[:vi] + note + "".join(f"        {sl(n)}: {sl(v)},\n" for n, v in lines) + b[vi:]
s = s[:ci] + b + s[ce:]

# ---- evconfig_private_h
ci = s.index('config_header(\n    name = "evconfig_private_h",'); ce = s.index("\n)\n", ci) + 1
b = s[ci:ce]
resolved2, unknown2 = names_of("002-*evconfig-private*")
ui = b.index("    unresolved = [\n"); uj = b.index("    ],\n", ui) + len("    ],\n")
vals = "".join(f"        {sl(n)}: {sl(v)},\n" for n, v in [(n, "") for n in unknown2] + list(resolved2.items()))
b = b[:ui] + "    # The AC_USE_SYSTEM_EXTENSIONS names, resolved as in config.h.\n    values = {\n" + vals + "    },\n" + b[uj:]
s = s[:ci] + b + s[ce:]

# assertions that pinned the frozen host answers now decided away
# HAVE_OPENSSL too: the assertion was generated for the option's configure
# default (on), and the option now defaults off.
for n in list(DECIDED) + ["SIZEOF_PTHREAD_T", "HAVE_EPOLL", "HAVE_GETADDRINFO", "HAVE_OPENSSL"]:
    s = s.replace(f'        "/* #undef {n} */",\n', "").replace(f'        "@{n}@",\n', "")

# ---- OpenSSL targets out; regress without ssl/zlib
for name in ["libevent_openssl.la_shared", "libevent_openssl.la", "sample_https-client", "sample_le-proxy"]:
    for kind in ["cc_shared_library", "cc_library", "cc_binary"]:
        key = f'{kind}(\n    name = "{name}",'
        i = s.find(key)
        if i >= 0:
            j = s.index("\n)\n", i) + len("\n)\n")
            s = s[:i] + s[j:].lstrip("\n")
            break
s = s.replace('        ":libevent_openssl.la",\n', "").replace('        ":libevent_openssl.la_shared",\n', "")
s = s.replace('        "test/regress_ssl.c",\n', "").replace('        "test/regress_zlib.c",\n', "")
s = re.sub(r'    dynamic_deps = \[\n    \],\n', "", s)
s = re.sub(r'    deps = \[\n    \],\n', "", s)

# ---- strlcpy.c beside evutil.c (project note 001); the generated header everywhere config.h is
s = s.replace('        "evutil.c",\n', '        "evutil.c",\n        "strlcpy.c",  # project_notes/001: compiled only where the C library lacks strlcpy\n')
s = s.replace('        ":config_h",\n', '        ":config_h",\n        ":event_config_h",\n')
rep('''    srcs = [
        "make-event-config.sed",
        ":config_h",
        ":event_config_h",
    ],''', '''    srcs = [
        "make-event-config.sed",
        ":config_h",
    ],''')
li = s.index('    name = "libevent_core.la",'); hi = s.index("    hdrs = [\n", li)
s = s[:hi+len("    hdrs = [\n")] + '        ":event_config_h",\n' + s[hi+len("    hdrs = [\n"):]

# ---- the ten runners
RUNNERS = [("epoll", "-b EPOLL"), ("select", "-b SELECT"), ("kqueue", "-b KQUEUE"), ("evport", "-b EVPORT"),
           ("devpoll", "-b DEVPOLL"), ("poll", "-b POLL"), ("win32", "-b WIN32"),
           ("timerfd", "-b '' -t"), ("changelist", "-b '' -c"), ("timerfd_changelist", "-b '' -T")]
progs = ["regress", "test-init", "test-eof", "test-closed", "test-weof", "test-time", "test-changelist", "test-fdleak", "test-dumpevents"]
bins = "".join(f'        "bin=test/{p}=$(rootpath :test_{p})",\n' for p in progs)
data = "".join(f'        ":test_{p}",\n' for p in progs)
out = []
for name, args in RUNNERS:
    def arg(a):
        return sl("@empty@" if a == "''" else a)
    argl = "".join(f"        {arg(a)},\n" for a in args.split(" "))
    out.append(f'''sh_test(
    name = "test_runner_{name}_test",
    srcs = ["run_layout_script_test.sh"],
    args = [
        "$(rootpath test/test.sh)",
        "bin=test/test.sh=$(rootpath test/test.sh)",
        "bin=test/check-dumpevents.py=$(rootpath test/check-dumpevents.py)",
{bins}        "--",
{argl}    ],
    data = [
        "test/test.sh",
        "test/check-dumpevents.py",
{data}    ],
)
''')
s = s.rstrip("\n") + "\n\n" + "\n".join(out)
open(p, "w").write(s)
shutil.copy(f"{R}/run_layout_script_test.sh", f"{M}/run_layout_script_test.sh"); os.chmod(f"{M}/run_layout_script_test.sh", 0o755)
# The comparisons these decisions make impossible, recorded where the
# harness reads them (see build-verification.md, "A recorded omission").
with open(f"{M}/TARGETS", "a") as t:
    t.write("omitted sample_https-client needs OpenSSL; no converted module provides it (enable_openssl defaults off)\n")
    t.write("omitted sample_le-proxy needs OpenSSL; no converted module provides it (enable_openssl defaults off)\n")
    t.write("omitted test_regress ground truth ran its OpenSSL and zlib tests; this module builds regress without them (bzl-7r9.8 for zlib)\n")
for f in glob.glob(f"{M}/needs_attention/*.md"): os.remove(f)
print("resolved libevent:", len(lines), "config values,", len(unknown2)+len(resolved2), "private")
