#!/usr/bin/env python3
"""Harvest cc_config catalog entries from a project's own `configure` script.

    python3 tools/catalog/harvest.py CONFIGURE [--template config.h.in ...] [--apply]

The catalog (cc_config/catalog/BUILD.bazel) is hand-maintained, and each
project onboarded so far grew it by a script written for that project. This
is that script, once: it reads the check sites autoconf leaves in `configure`
— every one of them, including the facts the Autotools frontend later freezes
as values and so never lists in an `unmapped_config_macros` item (bzl-kba) —
and prints the catalog entries the project needs that the catalog lacks.

Why `configure` and not the escalation, and not `configure.ac`: the item is
incomplete (above), and `configure.ac` is unexpanded m4 whose AC_CHECK_*
arguments can be variables. `configure` is the resolved text, and it states
the one thing a harvester must never guess: the header path behind a
HAVE_*_H macro (netinet/ip_icmp.h reads equally as netinet/ip/icmp.h
backwards) and the includes a type, member or declaration is probed under.

What `configure` does NOT state is the header for an AC_CHECK_FUNCS symbol,
which autoconf links against an implicit declaration. The catalog's
check_symbol_exists needs one, and a wrong header reports the symbol ABSENT —
indistinguishable from a platform that lacks it. So every symbol header here
is verified by compiling the catalog's own probe program against the host
toolchain; one no candidate satisfies is reported as `HEADER?` and never
applied. A symbol absent on this host altogether takes its header from a
small table of known platform-specific ones (kqueue -> sys/event.h) and is
marked unverifiable, which the report says out loud.

Every entry's headers are minimised the same way (the first single header the
probe compiles under, else the full list `configure` used), so an entry reads
like the hand-written ones. The host verdicts also give the smoke test
(testdata_catalog_smoke.h.in + catalog_smoke_test) an assertion per entry —
the only gate that checks a header PATH — and `--apply` writes all four
places a catalog entry lives: the catalog, the translator's CATALOG_DEFINES
mirror, the smoke template and its assertions. Then run `bazel run
//:buildifier` and the checks the report names.

The scripts beside this one are its parsers for the two shapes that predate
it: harvest_autoconf_headers (header paths, three forms) and
harvest_autoconf_flags (macros a --enable/--with flag sets, which are NOT
probes and are reported as skipped).
"""
import argparse
import datetime
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harvest_autoconf_flags import flag_macros  # noqa: E402
from harvest_autoconf_headers import harvest as harvest_header_paths  # noqa: E402

# autoconf's $ac_includes_default, in its order. Each is `#ifdef HAVE_x`
# guarded there; on any platform this tool targets they all exist.
DEFAULT_INCLUDES = [
    "stddef.h", "stdio.h", "stdlib.h", "string.h", "inttypes.h", "stdint.h",
    "strings.h", "sys/types.h", "sys/stat.h", "unistd.h",
]

# Headers tried, in order, for an AC_CHECK_FUNCS symbol. Specific headers
# come before the catch-alls (unistd.h declares a great deal) so the chosen
# header is the canonical one, not merely one that works here.
SYMBOL_HEADER_CANDIDATES = [
    "sys/mman.h", "sys/epoll.h", "sys/eventfd.h", "sys/timerfd.h",
    "sys/signalfd.h", "sys/sendfile.h", "sys/random.h", "sys/socket.h",
    "sys/uio.h", "sys/select.h", "sys/time.h", "sys/resource.h",
    "sys/wait.h", "sys/utsname.h", "sys/sysinfo.h", "sys/ioctl.h",
    "sys/file.h", "sys/prctl.h", "sys/inotify.h", "sys/statvfs.h",
    "sys/mount.h", "sys/param.h", "sys/ipc.h", "sys/shm.h", "sys/sem.h",
    "sys/msg.h", "sys/timeb.h", "sys/times.h", "sys/un.h", "sys/xattr.h",
    "sys/stat.h", "netdb.h", "arpa/inet.h", "netinet/in.h", "net/if.h",
    "ifaddrs.h", "poll.h", "sched.h", "pthread.h", "signal.h", "dlfcn.h",
    "dirent.h", "fcntl.h", "time.h", "locale.h", "langinfo.h", "iconv.h",
    "wchar.h", "wctype.h", "math.h", "malloc.h", "execinfo.h",
    "ucontext.h", "grp.h", "pwd.h", "syslog.h", "utime.h", "termios.h",
    "libgen.h", "getopt.h", "strings.h", "string.h", "stdlib.h", "stdio.h",
    "unistd.h", "stdint.h", "inttypes.h", "errno.h", "ctype.h",
]

# Symbols this host (glibc/Linux) does not have, with the header the platform
# that has them declares them in. Unverifiable here by construction; the
# report marks them so and the smoke test asserts the `absent` answer.
KNOWN_SYMBOL_HEADERS = {
    "kqueue": "sys/event.h",
    "kevent": "sys/event.h",
    "port_create": "port.h",
    "port_associate": "port.h",
    "mach_absolute_time": "mach/mach_time.h",
    "issetugid": "unistd.h",
    "sysctl": "sys/sysctl.h",
    "sysctlbyname": "sys/sysctl.h",
    "_gmtime64": "time.h",
    "_gmtime64_s": "time.h",
    "pledge": "unistd.h",
    "shl_load": "dl.h",
    "fls": "strings.h",
    "flsl": "strings.h",
    "flsll": "strings.h",
    "strlcpy": "string.h",
    "strlcat": "string.h",
    "arc4random": "stdlib.h",
    "arc4random_buf": "stdlib.h",
    "arc4random_addrandom": "stdlib.h",
    "getentropy": "unistd.h",
    "cpuset_setaffinity": "sys/cpuset.h",
    "sched_setaffinity": "sched.h",
    "pthread_setaffinity_np": "pthread.h",
    "pthread_getaffinity_np": "pthread.h",
    "pthread_set_qos_class_self_np": "pthread/qos.h",
    "get_nprocs": "sys/sysinfo.h",
    "getpagesize": "unistd.h",
    "getrandom": "sys/random.h",
    "posix_memalign": "stdlib.h",
    "memalign": "malloc.h",
    "clock_gettime": "time.h",
    "timegm": "time.h",
    "usleep": "unistd.h",
    "nanosleep": "time.h",
}

KINDS = ("header", "symbol", "type_exists", "struct_member", "sizeof")


@dataclass
class Entry:
    kind: str
    macro: str
    subject: str  # header path, symbol, type, or "struct x.member"
    headers: list = field(default_factory=list)  # as configure stated them
    member: str = ""
    present: object = None  # True / False / None (not checked)
    value: object = None  # sizeof result on the host
    source: str = "configure"  # where the headers came from
    note: str = ""


# --- parsing ---------------------------------------------------------------

_Q = r'"((?:[^"\\]|\\.)*)"'
_FUNC = re.compile(r'ac_fn_c_check_func\s+"\$LINENO"\s+' + _Q)
_FUNC_LOOP = re.compile(r'^\s*for ac_func in (.+)$', re.M)
_TYPE = re.compile(r'ac_fn_c_check_type\s+"\$LINENO"\s+' + _Q + r'\s+' + _Q + r'\s+' + _Q, re.S)
_MEMBER = re.compile(r'ac_fn_c_check_member\s+"\$LINENO"\s+' + _Q + r'\s+' + _Q + r'\s+' + _Q + r'\s+' + _Q, re.S)
# autoconf 2.69 spells it ac_fn_c_check_decl, 2.71 ac_fn_check_decl.
_DECL = re.compile(r'ac_fn_(?:c_)?check_decl\s+"\$LINENO"\s+' + _Q + r'\s+' + _Q + r'\s+' + _Q, re.S)
_SIZEOF = re.compile(r'ac_fn_c_compute_int\s+"\$LINENO"\s+"\(long int\) \(sizeof \(([^)]+)\)\)"\s+' + _Q + r'\s+' + _Q, re.S)
_INCLUDE = re.compile(r'#\s*include\s*<([^>]+)>')
_UNDEF = re.compile(r'^#\s*undef\s+([A-Z_][A-Z0-9_]*)', re.M)
_GENERATED_FOR = re.compile(r'Generated by GNU Autoconf [\d.]+ for (.+?)\.$', re.M)


def as_tr_cpp(text):
    """autoconf's AS_TR_CPP: `*` to P, then uppercase, every other non-alphanumeric to `_`."""
    return re.sub(r'[^A-Za-z0-9]', '_', text.replace("*", "P")).upper()


def join_continuations(text):
    """`for ac_header in  \\` runs over several lines; join them first."""
    return re.sub(r'\\\n\s*', ' ', text)


def includes_of(arg):
    """The headers an INCLUDES argument brings in, in order, defaults expanded."""
    headers = []
    if "$ac_includes_default" in arg:
        headers.extend(DEFAULT_INCLUDES)
    for h in _INCLUDE.findall(arg):
        if h not in headers:
            headers.append(h)
    return headers


def _literal(s):
    return "$" not in s and "`" not in s


def harvest(text):
    """Every check site in `configure`, as catalog-shaped entries, unfiltered."""
    text = join_continuations(text)
    entries = {}

    def add(entry):
        entries.setdefault(entry.macro, entry)

    for macro, path in sorted(harvest_header_paths(text).items()):
        add(Entry("header", macro, path))

    funcs = set()
    for m in _FUNC.finditer(text):
        if _literal(m.group(1)):
            funcs.add(m.group(1))
    for m in _FUNC_LOOP.finditer(text):
        # `for ac_func in a b; do` keeps the loop keyword on the line.
        for tok in m.group(1).split(";")[0].split():
            if _literal(tok) and re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', tok) and tok != "do":
                funcs.add(tok)
    for f in sorted(funcs):
        add(Entry("symbol", "HAVE_" + as_tr_cpp(f), f, source="search"))

    for m in _DECL.finditer(text):
        sym, inc = m.group(1), m.group(3)
        if _literal(sym):
            add(Entry("symbol", "HAVE_DECL_" + as_tr_cpp(sym), sym, includes_of(inc)))

    for m in _TYPE.finditer(text):
        typ, inc = m.group(1), m.group(3)
        if _literal(typ):
            add(Entry("type_exists", "HAVE_" + as_tr_cpp(typ), typ, includes_of(inc)))

    for m in _MEMBER.finditer(text):
        aggr, member, inc = m.group(1), m.group(2), m.group(4)
        if _literal(aggr) and _literal(member):
            add(Entry("struct_member", "HAVE_" + as_tr_cpp(aggr) + "_" + as_tr_cpp(member),
                      aggr, includes_of(inc), member=member))

    for m in _SIZEOF.finditer(text):
        typ, inc = m.group(1), m.group(3)
        if _literal(typ):
            add(Entry("sizeof", "SIZEOF_" + as_tr_cpp(typ), typ, includes_of(inc)))

    return list(entries.values())


def project_of(text):
    m = _GENERATED_FOR.search(text)
    return m.group(1) if m else "this project"


def template_macros(paths):
    names = set()
    for p in paths:
        with open(p, errors="ignore") as fh:
            names.update(_UNDEF.findall(fh.read()))
    return names


def catalog_macros(catalog_path):
    with open(catalog_path) as fh:
        return set(re.findall(r'"((?:HAVE_|SIZEOF_)[A-Z0-9_]+)"', fh.read()))


# --- host verification ------------------------------------------------------


class HostCompiler:
    """Compiles the catalog's own probe programs with the host `cc`.

    The catalog probes under the module's llvm toolchain, not this compiler;
    the host answers are used to CHOOSE and CHECK headers and to write the
    smoke assertions, which then verify the real probe under Bazel.
    """

    def __init__(self, cc):
        self.cc = cc
        self._cache = {}

    def _run(self, source, args, run=False):
        key = (source, tuple(args), run)
        if key in self._cache:
            return self._cache[key]
        with tempfile.TemporaryDirectory() as d:
            exe = os.path.join(d, "probe")
            cmd = [self.cc, *args, "-x", "c", "-", "-o", exe]
            r = subprocess.run(cmd, input=source, capture_output=True, text=True)
            out = None
            if r.returncode == 0 and run:
                p = subprocess.run([exe], capture_output=True, text=True)
                out = p.stdout.strip() if p.returncode == 0 else None
            result = (r.returncode == 0, out)
        self._cache[key] = result
        return result

    def compiles(self, source, link=False, defines=()):
        args = [f"-D{d}" for d in defines] + ([] if link else ["-c"])
        return self._run(source, args)[0]

    def run_int(self, source, defines=()):
        ok, out = self._run(source, [f"-D{d}" for d in defines], run=True)
        return int(out) if ok and out is not None else None


def _inc(headers):
    return "".join(f"#include <{h}>\n" for h in headers)


def probe_source(entry, headers):
    """The probe program cc_config/probe.bzl compiles for this kind, verbatim."""
    if entry.kind == "header":
        return _inc(headers)
    if entry.kind == "symbol":
        return _inc(headers) + (
            "\nint main(void) {\n#ifndef %s\n  (void)((void *)(&%s));\n#endif\n  return 0;\n}\n"
            % (entry.subject, entry.subject))
    if entry.kind == "type_exists":
        return _inc(headers) + "int main(void) { return (int) sizeof(%s) ? 0 : 0; }\n" % entry.subject
    if entry.kind == "struct_member":
        return _inc(headers) + "int main(void) { %s s; return (int) sizeof(s.%s) ? 0 : 0; }\n" % (
            entry.subject, entry.member)
    if entry.kind == "sizeof":
        return _inc(headers) + "int probe[sizeof(%s) > 0 ? 1 : -1];\n" % entry.subject
    raise ValueError(entry.kind)


def probe_args(entry):
    """(link, defines) as probe.bzl passes them for this kind."""
    if entry.kind == "symbol":
        return True, ("_GNU_SOURCE",)
    if entry.kind == "struct_member":
        return False, ("_GNU_SOURCE",)
    return False, ()


def verify(entries, compiler, header_overrides=None):
    """Chooses and checks each entry's headers on the host; sets `present`."""
    header_overrides = header_overrides or {}
    for e in entries:
        link, defines = probe_args(e)
        ok = lambda hs: compiler.compiles(probe_source(e, hs), link, defines)  # noqa: E731
        if e.kind == "header":
            e.present = ok([e.subject])
            continue
        if e.kind == "symbol" and not e.macro.startswith("HAVE_DECL_"):
            if e.subject in header_overrides:
                candidates, e.source = [header_overrides[e.subject]], "supplied"
            elif e.subject in KNOWN_SYMBOL_HEADERS:
                candidates, e.source = [KNOWN_SYMBOL_HEADERS[e.subject]], "table"
            else:
                candidates, e.source = SYMBOL_HEADER_CANDIDATES, "search"
            chosen = next((h for h in candidates if ok([h])), None)
            if chosen is not None:
                e.headers, e.present = [chosen], True
            elif e.source in ("table", "supplied"):
                e.headers, e.present = candidates, False
                e.note = "absent on this host; header from the %s, unverifiable here" % (
                    "known-platform table" if e.source == "table" else "command line")
            else:
                e.headers, e.present = [], False
                e.note = "HEADER? no candidate header declares it; pass --header %s=<h>" % e.subject
            continue
        # Kinds whose includes configure states: minimise to the first single
        # header that satisfies the probe, else the whole set, else absent.
        stated = list(e.headers)
        if e.kind in ("sizeof", "type_exists") and ok([]):
            e.headers, e.present = [], True
        else:
            single = next((h for h in stated if ok([h])), None)
            if single is not None:
                e.headers, e.present = [single], True
            elif stated and ok(stated):
                e.present = True
            else:
                e.present = False
                # Keep only the stated headers this host can include at all, so
                # the probe on another platform is not sunk by a header meant
                # for a third one; what remains is unverifiable here.
                e.headers = [h for h in stated if compiler.compiles(_inc([h]))] or stated
                e.note = "absent on this host; headers as configure stated them, unverifiable here"
        if e.kind == "sizeof" and e.present:
            e.value = compiler.run_int(
                _inc(e.headers) + '#include <stdio.h>\nint main(void){printf("%%zu\\n", sizeof(%s));return 0;}\n'
                % e.subject)
    return entries


# --- selection and rendering ------------------------------------------------


def select(entries, wanted, existing, flagged):
    """Splits harvested entries into (new, skipped_reasons).

    `skipped["no check site"]` is the rest of the template: macros neither the
    catalog nor any harvested site covers (a project's own AC_DEFINE under a
    custom test, or a flag). Those stay on the escalation path, and listing
    them is what tells the agent stage how much of the template that is.
    """
    new, skipped = [], {"in catalog": [], "not in template": [], "flag-driven": [], "no check site": []}
    harvested = {e.macro for e in entries}
    if wanted is not None:
        skipped["no check site"] = sorted(m for m in wanted if m not in harvested and m not in existing)
    for e in entries:
        if e.macro in existing:
            skipped["in catalog"].append(e.macro)
        elif wanted is not None and e.macro not in wanted:
            skipped["not in template"].append(e.macro)
        elif e.macro in flagged:
            skipped["flag-driven"].append(e.macro)
        else:
            new.append(e)
    return new, skipped


def catalog_line(e):
    """One catalog tuple; buildifier reflows it."""
    if e.kind == "header":
        return '("%s", "%s"),' % (e.subject, e.macro)
    hs = "[" + ", ".join('"%s"' % h for h in e.headers) + "]"
    if e.kind == "struct_member":
        return '("%s", "%s", %s, "%s"),' % (e.subject, e.member, hs, e.macro)
    return '("%s", %s, "%s"),' % (e.subject, hs, e.macro)


SECTION_OF = {"header": "headers", "symbol": "symbols", "type_exists": "type_exists",
              "struct_member": "struct_members", "sizeof": "types"}


def template_line(e):
    if e.macro.startswith("HAVE_DECL_"):
        return "#undef %s" % e.macro
    if e.kind == "sizeof":
        return "#define %s @%s@" % (e.macro, e.macro)
    return "#cmakedefine %s" % e.macro


def assertion_line(e):
    if e.macro.startswith("HAVE_DECL_"):
        return '"#define %s %d",' % (e.macro, 1 if e.present else 0)
    if e.kind == "sizeof":
        return '"#define %s %s",' % (e.macro, e.value)
    return ('"#define %s",' if e.present else '"/* #undef %s */",') % e.macro


def unresolved(entries):
    return [e for e in entries if e.note.startswith("HEADER?")]


def render_report(project, new, skipped, verified):
    out = ["catalog entries %s needs that the catalog lacks: %d" % (project, len(new))]
    for kind in KINDS:
        group = [e for e in new if e.kind == kind]
        if not group:
            continue
        out.append("")
        out.append("%s (%s): %d" % (SECTION_OF[kind], kind, len(group)))
        for e in group:
            verdict = "" if not verified else (
                "present" if e.present else "absent") + ("=%s" % e.value if e.value is not None else "")
            if e.headers:
                hdrs = ", ".join(e.headers)
            elif e.kind == "header" or not verified:
                hdrs = "-"
            else:
                hdrs = "HEADER?"
            subj = e.subject + ("." + e.member if e.member else "")
            out.append("  %-40s %-28s %-10s %s%s" % (
                e.macro, subj, verdict, hdrs, ("  # " + e.note) if e.note else ""))
    out.append("")
    for reason, names in skipped.items():
        if names:
            out.append("skipped, %s: %d" % (reason, len(names)))
            if reason in ("flag-driven", "no check site"):
                out.append("  " + " ".join(sorted(names)))
    return "\n".join(out)


# --- apply ------------------------------------------------------------------


def _insert_before(text, anchor, block, after=None):
    """Inserts `block` before the first `anchor` (after `after` if given)."""
    start = text.index(after) + len(after) if after else 0
    i = text.index(anchor, start)
    return text[:i] + block + text[i:]


def apply_catalog(text, new, banner):
    for kind in KINDS:
        group = [e for e in new if e.kind == kind]
        if not group:
            continue
        opener = "    %s = [\n" % SECTION_OF[kind]
        lines = ["        # %s\n" % banner]
        for e in group:
            comment = ""
            if e.present is False:
                comment = "  # absent on this host"
            lines.append("        %s%s\n" % (catalog_line(e), comment))
        text = _insert_before(text, "    ],\n", "".join(lines), after=opener)
    return text


def apply_defines(text, new, banner):
    block = "    // %s\n" % banner + "".join('    "%s",\n' % e.macro for e in new)
    return _insert_before(text, "];", block, after="const CATALOG_DEFINES")


def apply_smoke_template(text, new):
    if not text.endswith("\n"):
        text += "\n"
    return text + "".join(template_line(e) + "\n" for e in new)


def apply_smoke_build(text, new, banner):
    probes = "        # %s\n" % banner + "".join('        ":%s",\n' % e.macro.lower() for e in new)
    text = _insert_before(text, "    ],\n    template =", probes, after='name = "catalog_smoke_h"')
    asserts = "        # %s\n" % banner + "".join("        %s\n" % assertion_line(e) for e in new)
    return _insert_before(text, "    ],\n    must_not_contain", asserts, after='name = "catalog_smoke_test"')


def apply_all(root, new, banner):
    paths = {
        "catalog": os.path.join(root, "cc_config/catalog/BUILD.bazel"),
        "defines": os.path.join(root, "translator/src/configure_file.rs"),
        "template": os.path.join(root, "cc_config/catalog/testdata_catalog_smoke.h.in"),
    }
    texts = {k: open(p).read() for k, p in paths.items()}
    # Compute every new text before writing any, so a failed anchor leaves
    # nothing half-applied.
    out = {
        "catalog": apply_smoke_build(apply_catalog(texts["catalog"], new, banner), new, banner),
        "defines": apply_defines(texts["defines"], new, banner),
        "template": apply_smoke_template(texts["template"], new),
    }
    for k, p in paths.items():
        with open(p, "w") as fh:
            fh.write(out[k])
    return list(paths.values())


# --- main -------------------------------------------------------------------


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("configure")
    ap.add_argument("--template", action="append", default=[],
                    help="config.h.in the project consumes; only its #undef macros are reported (repeatable)")
    ap.add_argument("--all", action="store_true", help="report every check site, template or not")
    ap.add_argument("--header", action="append", default=[], metavar="SYMBOL=HEADER",
                    help="header for an AC_CHECK_FUNCS symbol no candidate declares")
    ap.add_argument("--cc", default=os.environ.get("CC", "cc"))
    ap.add_argument("--no-verify", action="store_true", help="parse only; no host compiles")
    ap.add_argument("--apply", action="store_true", help="write the entries into the four catalog files")
    ap.add_argument("--root", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
    args = ap.parse_args(argv)

    with open(args.configure, errors="ignore") as fh:
        text = fh.read()
    project = project_of(text)
    entries = harvest(text)
    wanted = None if (args.all or not args.template) else template_macros(args.template)
    existing = catalog_macros(os.path.join(args.root, "cc_config/catalog/BUILD.bazel"))
    flagged = set(flag_macros(join_continuations(text)))
    new, skipped = select(entries, wanted, existing, flagged)
    if not args.no_verify:
        overrides = dict(h.split("=", 1) for h in args.header)
        verify(new, HostCompiler(args.cc), overrides)
    print(render_report(project, new, skipped, verified=not args.no_verify))
    if not args.apply:
        return 0
    if args.no_verify:
        print("\n--apply needs the host verdicts; drop --no-verify", file=sys.stderr)
        return 2
    bad = unresolved(new)
    if bad:
        print("\nnot applied: %d symbol(s) have no verified header (%s)" % (
            len(bad), ", ".join(e.subject for e in bad)), file=sys.stderr)
        return 2
    if not new:
        return 0
    banner = "Harvested from %s's configure by tools/catalog/harvest.py, %s." % (
        project, datetime.date.today().isoformat())
    for p in apply_all(args.root, new, banner):
        print("wrote", os.path.relpath(p, args.root))
    print("next: bazel run //:buildifier && bazel test //:catalog_sync_check //:buildifier_check "
          "&& (cd cc_config && bazel test //catalog:catalog_smoke_test)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
