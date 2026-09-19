# Adapted for the PMIx workspace: module path from argv, and '' (not 0)
# for 'undefined' since the config-header value fix of 2026-09-19.
import re, os, glob, json, shutil, sys
M = sys.argv[1]
R = "/tmp/claude-1000/-workspaces-bazelifier/0bb7bbbd-6cbf-4128-9413-172653ee329b/scratchpad/hwloc/resolve"
p = f"{M}/BUILD.bazel"; s = open(p).read()
def rep(s, old, new, count=1):
    n = s.count(old); assert n == count, f"{n} != {count}: {old[:100]}"
    return s.replace(old, new)
def sl(v):  # a C-literal value as a Starlark string
    return json.dumps(v)

# ---- names from the items
resolved, unknown = {}, {"private": [], "public": []}
for kind, pat in [("private", "001-*private*"), ("public", "002-*hwloc-autogen*")]:
    f = glob.glob(f"{M}/needs_attention/{pat}")[0]
    for line in open(f):
        m = re.match(r"^- `([A-Za-z_0-9]+)`(?: — configure resolved this to `(.*)`)?", line)
        if not m: continue
        if m.group(2) is not None: resolved.setdefault(m.group(1), m.group(2))
        else: unknown[kind].append(m.group(1))

ALIASES = {  # macro -> catalog probe (the fact configure derived it from)
    "HWLOC_HAVE_CLZ": "have_clz", "HWLOC_HAVE_CLZL": "have_clzl",
    "HWLOC_HAVE_FLS": "have_fls", "HWLOC_HAVE_FLSL": "have_flsl",
    "HWLOC_HAVE_FFS": "have_ffs", "HWLOC_HAVE_FFSL": "have_ffsl",
    "HWLOC_HAVE_DECL_CLZ": "have_decl_clz", "HWLOC_HAVE_DECL_CLZL": "have_decl_clzl",
    "HWLOC_HAVE_DECL_FLS": "have_decl_fls", "HWLOC_HAVE_DECL_FLSL": "have_decl_flsl",
    "HWLOC_HAVE_DECL_FFS": "have_decl_ffs", "HWLOC_HAVE_DECL_FFSL": "have_decl_ffsl",
    "HWLOC_HAVE_DECL_STRCASECMP": "have_decl_strcasecmp", "HWLOC_HAVE_DECL_STRNCASECMP": "have_decl_strncasecmp",
    "HWLOC_HAVE_WINDOWS_H": "have_windows_h", "HWLOC_HAVE_STDINT_H": "have_stdint_h",
}
ONE = {"HWLOC_HAVE_ATTRIBUTE"}
# Values the frontend froze from this host that are a DECISION here, not a fact.
DECIDED = {
    "HWLOC_HAVE_LIBTERMCAP": "", "HWLOC_USE_NCURSES": "", "HWLOC_HAVE_X11_KEYSYM": "",
    "HWLOC_HAVE_LIBXML2": "", "HWLOC_XML_LIBXML_COMPONENT_BUILTIN": "",
}

# ---- 1. loads
s = rep(s, 'load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")\n',
'''load("@cc_config//cc_config:config_header.bzl", "assert_config_header_test", "config_header")
load("@cc_config//cc_config:probe.bzl", "probe_alias")
''')

# ---- 2. header notes + aliases + components genrule, before the first config_header
alias_rules = "".join(f'''
probe_alias(
    name = "{name.lower()}",
    define = "{name}",
    probe = "@cc_config//catalog:{probe}",
)
''' for name, probe in ALIASES.items())
first_ch = s.index("config_header(\n")
s = s[:first_ch] + '''# ---- Agent-stage resolutions (bzl-7r9.4). Each is a decision, said why.
#
# hwloc/static-components.h is written by CONFIGURE (config/hwloc.m4,
# hwloc_static_components_file), not by make, so the build output has no
# recipe for it: it lists the components compiled into libhwloc, which is
# the set configure decided from the options recorded on this conversion
# (--enable-plugins --disable-libxml2 --disable-pci ... on Linux/x86). That
# set is a decision about THIS module, not a fact about the conversion host,
# so it is written out here where the next reader can see it. Adding a
# component means adding it here and to libhwloc.la's sources.
genrule(
    name = "static_components_h",
    outs = ["hwloc/static-components.h"],
    cmd = """cat > $@ <<'EOF'
#include <private/internal-components.h>
static const struct hwloc_component * hwloc_static_components[] = {
  &hwloc_noos_component,
  &hwloc_xml_component,
  &hwloc_synthetic_component,
  &hwloc_xml_nolibxml_component,
  &hwloc_linux_component,
  &hwloc_x86_component,
  NULL
};
EOF""",
)

# Facts configure DERIVES from a catalog probe under its own name
# (config/hwloc.m4): HWLOC_HAVE_CLZ from ac_cv_func_clz, HWLOC_HAVE_DECL_CLZ
# from ac_cv_have_decl_clz, and so on. Republished so each consumer's own
# toolchain answers — the BSD bit-scan family is absent on glibc and present
# there — rather than baking this host's answer.
''' + alias_rules + '''
# libhwloc links no libxml2. A default configure found this host's libxml2
# and compiled the xml_libxml backend into libhwloc (the -lxml2 item); no
# converted module provides libxml2, and hwloc's own xml_nolibxml backend
# gives the same XML import/export without it. So the module replicates a
# build without libxml2: HWLOC_HAVE_LIBXML2 and the component's BUILTIN
# macro undefined, topology-xml-libxml.c out of libhwloc's sources, and the
# component absent from static-components.h above — every one of hwloc's
# XML topology tests still passes through the built-in backend. The
# decision is hwloc's, made once here; converting libxml2 is a separate
# project.
#
# lstopo-no-graphics links no terminal library. Its configure probes for
# ncurses/termcap and links whatever it finds (the conversion host: -lncursesw),
# which is used only to colour the ASCII renderer's output on a terminal; no
# converted module provides one, Open MPI never uses the hwloc utilities, and
# linking the host's copy is the non-hermeticity this module exists to rule
# out. HWLOC_HAVE_LIBTERMCAP / HWLOC_USE_NCURSES are therefore undefined below
# (lstopo-ascii.c guards every use), which is the behaviour of a build on a
# machine without curses. Recorded as the resolution of the
# unconverted_dependency item for -lncursesw.
#
# The eleven configure-generated test scripts under utils/ and tests/hwloc/
# (test-hwloc-*.sh.in, test-lstopo*.sh.in, test-hwloc-dump-hwdata.sh.in) and
# the 123 recorded-topology tests are reproduced below by two runners,
# run_hwloc_script_test.sh and run_topology_test.sh. Two are deliberately NOT
# reproduced: test-fake-plugin.sh loads a dlopen plugin and this module builds
# none (libxml2 disabled), and tests/hwloc/linux/gather/test-gather-topology.sh
# reads the LIVE machine's /sys and compares it against itself — a check on
# the conversion host, not on the project.

''' + s[first_ch:]

# ---- 3. private config header: unresolved -> values/probes
def resolve_header(s, name, kind):
    ci = s.index(f'config_header(\n    name = "{name}",')
    ce = s.index("\n)\n", ci)
    block = s[ci:ce]
    # probes: add aliases + the catalog probes for names configure answered by probe
    # An alias goes wherever the TEMPLATE names the macro — the frontend had
    # frozen HWLOC_HAVE_STDINT_H in both headers, and the public one gates
    # its #include <stdint.h> on it.
    template = re.search(r'template = "([^"]+)"', block).group(1)
    ttext = open(f"{M}/{template}").read()
    used_aliases = [n for n in ALIASES if re.search(r"\b%s\b" % n, ttext)]
    add = "".join(f'        ":{n.lower()}",\n' for n in used_aliases)
    if "    probes = [\n" in block:
        pi = block.index("    probes = [\n"); pj = block.index("    ],\n", pi)
        block = block[:pj] + add + block[pj:]
    elif add:
        ti = block.index("    template = "); tj = block.index("\n", ti) + 1
        block = block[:tj] + "    probes = [\n" + add + "    ],\n" + block[tj:]
    # unresolved block removed
    ui = block.index("    unresolved = [\n"); uj = block.index("    ],\n", ui) + len("    ],\n")
    block = block[:ui] + block[uj:]
    # values: build the new entries
    entries = []
    names = [n for n in unknown[kind] if n not in ALIASES] + [n for n in resolved if n in unknown[kind] or True]
    seen = set()
    lines = []
    for n in unknown[kind]:
        if n in ALIASES or n in seen: continue
        seen.add(n)
        lines.append((n, "1" if n in ONE else ""))
    for n, v in resolved.items():
        if n in seen or n in ALIASES: continue
        # only names this header's item listed
        item = glob.glob(f"{M}/needs_attention/{'001-*private*' if kind=='private' else '002-*hwloc-autogen*'}")[0]
        if f"- `{n}`" not in open(item).read(): continue
        seen.add(n)
        lines.append((n, v))
    body = "".join(f"        {sl(n)}: {sl(v)},\n" for n, v in lines)
    note = '''        # ---- resolved by the agent stage (bzl-7r9.4), grouped by why:
        # "" = undefined. Other platforms' facts (HWLOC_*_SYS, the Windows
        # HAVE_*RELATIONSHIP*/KAFFINITY/PSAPI/PROCESSOR_* type checks, AIX's
        # pthread_getthrds_np, HAVE_LIBGDI32/LIBKSTAT/LIBLGRP/LIBIBVERBS,
        # HAVE_SYSCTLBYNAME) — this module targets Linux/x86_64 (the llvm
        # toolchain it declares), and every one is what configure decides
        # there. Components disabled by the recorded configure options
        # (HWLOC_*_COMPONENT_BUILTIN, HWLOC_HAVE_CUDART/GL/LIBUDEV/CAIRO/LTDL/
        # LINUXPCI, HAVE_CUDA, HWLOC_HAVE_32BITS_PCI_DOMAIN, LSTOPO_HAVE_X11,
        # X_DISPLAY_MISSING, NETLOC_SCOTCH). Compile-time checks that are
        # false on every C99/glibc toolchain (HWLOC_HAVE_BROKEN_FFS,
        # HWLOC_HAVE_OLD_SCHED_SETAFFINITY). HWLOC_HAVE_GCC_W_* only add
        # warning flags in the Makefile. HWLOC_DEBUG: --enable-debug not
        # given. HWLOC_X86_32_ARCH: the toolchain is 64-bit (its sibling
        # HWLOC_X86_64_ARCH is 1). _MINIX/_POSIX_SOURCE/_POSIX_1_SOURCE/
        # _XOPEN_SOURCE: AC_USE_SYSTEM_EXTENSIONS leaves them for other
        # platforms. HWLOC_HAVE_ATTRIBUTE and HWLOC_HAVE_PLUGINS are 1: the
        # compiler supports __attribute__ (every HWLOC_HAVE_ATTRIBUTE_* above
        # is 1). HWLOC_HAVE_PLUGINS stays 0: a default configure builds no
        # dlopen plugins, and nothing here depends on them.
        # Values in the second group are what configure resolved, quoted from
        # the escalation (versions, symbol prefix, typedefs, LT_OBJDIR).
'''
    # find the trailing plain dict of values (after any select() chain)
    if "    }) | {\n" in block:
        vi = block.index("    }) | {\n") + len("    }) | {\n")
    elif "    values = {\n" in block:
        vi = block.index("    values = {\n") + len("    values = {\n")
    else:
        # no values at all: add one before the closing
        block = block + "    values = {\n    },"
        vi = block.index("    values = {\n") + len("    values = {\n")
    block = block[:vi] + note + body + block[vi:]
    # frozen values that are decisions here
    for n, v in DECIDED.items():
        block = re.sub(r'        "%s": "[^"]*",\n' % n, f'        "{n}": "{v}",  # decision, see the note above lstopo-no-graphics\n', block)
    # frozen HWLOC_HAVE_STDINT_H / the mis-paired with_x select
    block = block.replace('        "HWLOC_HAVE_STDINT_H": "1",\n', "")
    if kind == "private":
        block = block.replace('''    }) | select({
        ":with_x_on": {"HWLOC_HAVE_PLUGINS": "1"},
        "//conditions:default": {},
''', "")
        pass
    return s[:ci] + block + s[ce:]

s = resolve_header(s, "include_private_autogen_config_h", "private")
s = resolve_header(s, "include_hwloc_autogen_config_h", "public")

# assertions that pinned the frozen decisions now undefined
for n in DECIDED:
    s = s.replace(f'        "/* #undef {n} */",\n', "")
    s = s.replace(f'        "@{n}@",\n', "")
s = s.replace('        "/* #undef HWLOC_HAVE_STDINT_H */",\n', "").replace('        "@HWLOC_HAVE_STDINT_H@",\n', "")

# ---- 4. static-components.h into libhwloc.la; the libxml2 backend out of it
li = s.index('    name = "libhwloc.la",'); si = s.index("    srcs = [\n", li)
s = s[:si+len("    srcs = [\n")] + '        ":static_components_h",\n' + s[si+len("    srcs = [\n"):]
s = s.replace('        "hwloc/topology-xml-libxml.c",\n', "")

# ---- 5. tests
item = glob.glob(f"{M}/needs_attention/003-*registered-test*")[0]
present = [re.match(r"^- `([^`]+)`", l).group(1) for l in open(item) if l.startswith("- `") and "present in this module" in l]
topo = []
for e in present:
    if e.startswith("tests/hwloc/linux/allowed/"): topo.append(("allowed", e))
    elif e.startswith("tests/hwloc/linux/"): topo.append(("linux", e))
    elif e.startswith("tests/hwloc/x86+linux/"): topo.append(("x86+linux", e))
    elif e.startswith("tests/hwloc/x86/"): topo.append(("x86", e))
    elif e.startswith("tests/hwloc/xml/"): topo.append(("xml", e))
    elif e.startswith("utils/hwloc/test-hwloc-dump-hwdata/"): pass
    else: raise SystemExit(f"unexpected present entry {e}")
def tname(e):
    return re.sub(r"[^A-Za-z0-9_.+-]", "_", e.replace("tests/hwloc/", "").replace("/", "_"))
out = ["\n# ---- recorded-topology tests: one per automake TESTS entry, see run_topology_test.sh\n"]
for kind, e in topo:
    d = os.path.dirname(e)
    out.append(f'''sh_test(
    name = "{tname(e)}_test",
    srcs = ["run_topology_test.sh"],
    args = [
        "$(rootpath :lstopo-no-graphics)",
        "{kind}",
        "$(rootpath {e})",
    ],
    data = [":lstopo-no-graphics"] + glob(["{d}/*"]),
)
''')
# utility scripts
BIN = {"hwloc-calc": "utils/hwloc/hwloc-calc", "hwloc-annotate": "utils/hwloc/hwloc-annotate", "hwloc-info": "utils/hwloc/hwloc-info",
       "hwloc-distrib": "utils/hwloc/hwloc-distrib", "hwloc-diff": "utils/hwloc/hwloc-diff", "hwloc-patch": "utils/hwloc/hwloc-patch",
       "hwloc-dump-hwdata": "utils/hwloc/hwloc-dump-hwdata", "lstopo-no-graphics": "utils/lstopo/lstopo-no-graphics"}
def binargs(names):
    return "".join(f'        "bin={BIN[n]}=$(rootpath :{n})",\n' for n in names)
def bindata(names):
    return "".join(f'        ":{n}",\n' for n in names)
SCRIPTS = [
    ("utils/hwloc/test-hwloc-annotate.sh.in", ["hwloc-annotate"], ["utils/hwloc/*"], ""),
    ("utils/hwloc/test-hwloc-calc.sh.in", ["hwloc-calc"], ["utils/hwloc/*", "tests/hwloc/linux/*", "tests/hwloc/xml/*"], ""),
    ("utils/hwloc/test-hwloc-compress-dir.sh.in", ["hwloc-diff", "hwloc-patch"], ["utils/hwloc/*"], '        "gen=utils/hwloc/hwloc-compress-dir=utils/hwloc/hwloc-compress-dir.in",\n'),
    ("utils/hwloc/test-hwloc-diffpatch.sh.in", ["hwloc-diff", "hwloc-patch"], ["utils/hwloc/*"], ""),
    ("utils/hwloc/test-hwloc-distrib.sh.in", ["hwloc-distrib"], ["utils/hwloc/*"], ""),
    ("utils/hwloc/test-hwloc-info.sh.in", ["hwloc-info"], ["utils/hwloc/*", "tests/hwloc/linux/*"], ""),
    ("utils/hwloc/test-build-custom-topology.sh.in", ["hwloc-calc", "hwloc-annotate", "lstopo-no-graphics"], ["utils/hwloc/*"], ""),
    ("utils/hwloc/test-parsing-flags.sh.in", [], ["utils/hwloc/*", "include/**"], ""),
    ("utils/lstopo/test-lstopo.sh.in", ["lstopo-no-graphics"], ["utils/lstopo/*"], ""),
    ("utils/lstopo/test-lstopo-shmem.sh.in", ["lstopo-no-graphics"], ["utils/lstopo/*"], ""),
    ("utils/hwloc/test-hwloc-dump-hwdata/test-hwloc-dump-hwdata.sh.in", ["hwloc-dump-hwdata"], ["utils/hwloc/test-hwloc-dump-hwdata/*"], '        "--",\n        "$(rootpath utils/hwloc/test-hwloc-dump-hwdata/knl-snc4h50.tar.bz2)",\n'),
]
out.append("\n# ---- configure-generated test scripts, run through run_hwloc_script_test.sh\n")
for script, bins, globs, extra in SCRIPTS:
    name = os.path.basename(script).replace(".sh.in", "").replace("-", "_") + "_test"
    # Every utility script may reach into the recorded topologies (annotate
    # reads tests/hwloc/xml/power8gpudistances.xml; calc and info read the
    # linux ones), so all three data directories ride along with each.
    globs = sorted(set(globs) | {"tests/hwloc/xml/*", "tests/hwloc/linux/*"})
    gl = ", ".join(f'"{g}"' for g in globs)
    out.append(f'''sh_test(
    name = "{name}",
    srcs = ["run_hwloc_script_test.sh"],
    args = [
        "$(rootpath {script})",
{binargs(bins)}{extra}    ],
    data = [
{bindata(bins)}    ] + glob([{gl}]),
)
''')
# tests/hwloc's check programs run through its LOG_COMPILER, wrapper.sh
# (Makefile.am): it exports HWLOC_TOP_SRCDIR and runs xmlbuffer four times
# with its libxml on/off arguments. The frontend's generated sh_tests run the
# binary bare, which is right for every other directory and wrong for this
# one; each is rewritten to go through the wrapper the way automake does.
import re as _re
check_programs = []
for m in _re.finditer(r'cc_binary\(\n    name = "([^"]+)",\n    srcs = \[\n        "tests/hwloc/([^"/]+)\.c"', s):
    check_programs.append(m.group(1))
for prog in check_programs:
    i = s.find(f'sh_test(\n    name = "{prog}_test",\n    srcs = ["run_registered_test.sh"],')
    if i < 0: continue
    j = s.index("\n)\n", i) + len("\n)\n")
    s = s[:i] + f'''sh_test(
    name = "{prog}_test",
    srcs = ["run_hwloc_script_test.sh"],
    args = [
        "$(rootpath tests/hwloc/wrapper.sh.in)",
        "bin=tests/hwloc/{prog}=$(rootpath :{prog})",
        "--",
        "@build@/tests/hwloc/{prog}",
    ],
    data = [":{prog}"] + glob(["tests/hwloc/*", "tests/hwloc/xml/*", "tests/hwloc/linux/*", "include/**"]),
)
''' + s[j:]
print("wrapped check programs:", len(check_programs))
s = s.rstrip("\n") + "\n" + "".join(out)
open(p, "w").write(s)
for r in ["run_topology_test.sh", "run_hwloc_script_test.sh"]:
    shutil.copy(f"{R}/{r}", f"{M}/{r}"); os.chmod(f"{M}/{r}", 0o755)
for f in glob.glob(f"{M}/needs_attention/*.md"):
    os.remove(f)
print("topology tests:", len(topo), "script tests:", len(SCRIPTS), "aliases:", len(ALIASES))
