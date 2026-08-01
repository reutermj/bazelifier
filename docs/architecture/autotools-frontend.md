# Autotools frontend

Covers how bazelifier reads Autotools (autoconf + automake + libtool)
projects. The CMake frontend is documented separately in
[cmake-frontend.md](cmake-frontend.md); this one exists to be read alongside
it, because the two make the same decisions for the same reasons and differ
mainly in what evidence is available.

## Why this frontend exists

To test the claim the model has always made. `model.rs` says it is neutral
between build systems, `codegen.rs` imports nothing but `model`, and both were
true — but untested, because CMake was the only frontend. A second one either
renders through unmodified codegen or reveals that the boundary was CMake's
shape all along.

It rendered through unmodified. `model::Target` needed **no new fields**:
automake's primaries map onto what `TargetKind` and `is_shared` already
described, and `is_shared` — added weeks earlier for zlib's shared/static
split — turned out to be exactly what libtool needed.

## Source of truth: `make -n`, not `Makefile.am`

**Decision:** the frontend reads the build system's own resolved output, the
same principle [cmake-frontend.md](cmake-frontend.md) states for the File API.
Three candidates were compared against GNU hello and a purpose-built project:

| candidate | resolved? | practical |
|---|---|---|
| `Makefile.am` | **no** | closest in intent, 137 lines |
| generated `Makefile` | yes | 4666 lines of make syntax |
| `make -n` | yes | the command stream |

`Makefile.am` is closest in *intent* — it names targets, sources and
dependencies directly — and is the wrong choice for the same reason
`CMakeLists.txt` is. It is not resolved:

```makefile
hello_LDADD = $(LIBINTL) $(top_builddir)/lib/lib$(PACKAGE).a
```

Those variables only `configure` fills in, and automake conditionals plus
`SUBDIRS` recursion mean the declared graph is not the built one. Parsing it is
the "read the source build files" path the CMake frontend explicitly rejects.

The generated `Makefile` *is* resolved but is thousands of lines of make syntax
with recursive expansion; consuming it means implementing enough of make to be
correct — the same trap as re-implementing CMake-the-language.

`make -n` prints exactly what the build would run, fully expanded:

```
gcc -DPACKAGE_NAME=\"greeter\" ... -I. -g -O2 -c -o src/greet.o src/greet.c
ar cru libgreet.a src/greet.o
/bin/bash ./libtool --tag=CC --mode=link gcc -o greeter src/main.o libgreet.a libshout.la
```

(Elided in the middle: `configure` puts its whole substitution block —
`PACKAGE_NAME`, `VERSION`, every `HAVE_*` it probed — on every compile line.
Those quotes are why generated Starlark strings have to be escaped.)

It is also stable between runs in the ordering that matters — the command
sequence — where the CMake File API reports dependency order unstably
(bzl-sjp). It is not byte-identical: `-B` forces a `config.status --recheck`
preamble whose content varies, which is why the frontend recognises only the
handful of programs that build something and ignores every other line.

## Second source: `make -p`, for identity

The command stream carries no target **names**. automake knows a program is
called `greeter` because `bin_PROGRAMS` says so; the stream shows only
`-o greeter`. So identity comes from `make -p`, which prints make's resolved
variable database:

```
bin_PROGRAMS = greeter$(EXEEXT)
noinst_LIBRARIES = libgreet.a
lib_LTLIBRARIES = libshout.la
include_HEADERS = src/greet.h
EXEEXT =
```

Two sources, each answering what the other cannot — the same shape as the
CMake frontend, where the File API is primary and the configure trace is a
documented exception for `configure_file`.

**Database:** target identity, kind, install destination, declared sources,
public headers.
**Command stream:** resolved `-I`/`-D` per compile, and the actual link inputs.
**Joined on:** the artifact each target produces, which both name.

The split is not arbitrary. Each source is authoritative for what it can see
and unreliable for the rest:

- `hello_LDADD` in the database is *unexpanded*, so dependency edges taken from
  it silently vanish. The link line has the resolved `./lib/libhello.a`.
- `*_CPPFLAGS` in the database is pre-expansion and misses what `configure` and
  `AM_CPPFLAGS` contributed. The compile line has everything.

## What the frontend produces

### Targets, from automake's primaries

| automake | model |
|---|---|
| `bin_PROGRAMS` | `TargetKind::Executable` |
| `noinst_LIBRARIES`, `lib_LIBRARIES` | `TargetKind::Library`, `is_shared = false` |
| `lib_LTLIBRARIES` | `TargetKind::Library`, `is_shared = true` |
| `check_PROGRAMS` | *skipped* — see below |

`check_PROGRAMS` are declared but **not built by `make`** (that is `make
check`), so no link command exists and nothing says which directory their
sources are relative to. Skipping is the honest option: a target whose sources
were resolved against a guessed directory is worse than an absent one, and the
guess fails loudly on copy, which is how this was found. They are recorded for
the escalation they deserve (bzl-yjn.5).

The prefix before the underscore is the **install destination** and carries
real meaning — `noinst_` means built but never installed. That is closer to
CMake's `install()` than to `add_library()`, and it is where the public/private
signal lives.

Linkage comes from the primary, not from a filename: `LTLIBRARIES` is libtool's
form, `LIBRARIES` a plain archive. Guessing from a `.la` suffix would be
inference where a declaration is available.

### Target names are kept, not prettified

`lib/libhello.a` becomes `lib_libhello.a`, not `hello`. An earlier version
stripped the `lib` prefix and the suffix, which reads better right up until a
project names its library after its program — GNU hello does exactly that
(`bin_PROGRAMS = hello`, `noinst_LIBRARIES = lib/libhello.a`), and both
collapsed to one name, producing a module that could not load.

The directory is kept for the same reason: two `SUBDIRS` can each declare a
`libhello.a`.

### Public headers, from `include_HEADERS`

`include_HEADERS = src/greet.h` is automake's statement that a header is
installed — the same claim CMake makes with
`install(FILES ... DESTINATION include)`, and the same signal the CMake
frontend already models. A header listed in `_SOURCES` but not installed stays
private.

This needs no `Makefile.am` parsing: `make -p` reports the primary directly.

### Config headers, in autoconf's dialect

autoconf's `config.h.in` writes a bare `#undef FOO` where CMake writes
`#cmakedefine FOO`. **They are the same statement** — "define this if the probe
succeeded" — spelled as the false case rather than the true one.

`cc_config`'s expander resolves both identically, so the same probe results
produce the same header whichever build system a project used. That shared
machinery is a large part of why Autotools was the right second frontend:
it reuses the toolchain-probing design rather than reinventing it. See
[configure-file-and-toolchain-probes.md](configure-file-and-toolchain-probes.md).

The `#undef` form is matched only at line start, unlike the CMake directive.
A mid-line `#undef` is ordinary C undefining a macro, and rewriting it would
corrupt a header rather than configure it.

## Ordering, and why it is load-bearing

`discover` configures, then **builds**, then interrogates — and the dry run
passes `-B`. Both halves are load-bearing, and each obvious alternative fails
on a real project:

- **Interrogating before the build** fails outright on a recursive project.
  xz's `src/xzdec` needs `../../src/liblzma/liblzma.la`, which no subdirectory
  has built yet, so `make` reports "No rule to make target" and exits 2.
- **A plain `make -n` after the build** succeeds and prints *nothing*: every
  target is up to date, and the command stream is the entire input.

`-B` asks what a full rebuild *would* run, which is the question this frontend
actually needs, and a built tree is the only state in which every subdirectory
can answer it. The build has to happen anyway — ground-truth artifacts come
from it (see [build-verification.md](build-verification.md)) — so the ordering
costs nothing beyond the forced re-listing.

## Recursive make changes two things, both silently

`SUBDIRS` recursion is not a corner case — xz uses it, and it breaks two
assumptions that hold for CMake+Ninja. Both were found the same way: a
conversion that succeeded while producing the wrong thing.

**Commands run from the subdirectory, so paths are relative to it.** `make`
descends into each `SUBDIRS` entry and runs that directory's commands from
there. xz's `src/xz` compiles `../common/tuklib_mbstr_width.c`; taken at face
value against the build root, that path reaches above the module, which no
Bazel label can express. `BuildCommand::dir` tracks make's `Entering`/`Leaving`
announcements as a **stack** (they nest — `make[2]` inside `make[1]`), and
every path is resolved against the directory its own command ran in.

CMake+Ninja needs no equivalent: it runs every command from the build root, so
its paths are already root-relative. This is a structural difference between
the build systems, not an oversight in the CMake frontend.

**`make -p` concatenates every subdirectory's database, so one name is defined
several times.** For a per-target variable that is harmless — `xz_SOURCES`
belongs to whichever directory declares `xz` and no other. For a **primary** it
is not: `bin_PROGRAMS` is declared once per directory, so xz emits four
definitions of it. Keeping the last one dropped the project's namesake `xz`
binary and reported success. Primaries therefore accumulate across definitions;
everything else keeps make's last-assignment-wins.

The top-level definition is usually a list of automake internals
(`$(am__EXEEXT_1)`), unexpanded because make never had to expand it. Those are
skipped rather than taken as target names — the real names arrive from the
subdirectory definitions merged in alongside them.

## The module root is derived, not the project directory

A Bazel label cannot reach above its own module, so a project that compiles a
source from outside its own directory needs a module root wide enough to
contain both. The root therefore **widens** from the project directory to the
deepest directory containing everything the build references — and
`deliverable_root` caps that widening.

The three outcomes, which have to be read together:

| the build references | module root | result |
|---|---|---|
| nothing outside the project | the project directory | unchanged |
| a sibling **inside** the deliverable | widened to contain both | shipped, no escalation |
| a file **outside** the deliverable | not widened | dropped and escalated |

What decides the second row from the third is the **declared deliverable**,
not where the file sits on disk. The same project converts either way
depending on `--deliverable-root`, which is why "re-run with a wider root" is
the escalation's own first suggested resolution.

This is the same rule the CMake frontend applies in
`rebase_to_module_root`, and it has to be: two equivalent projects, one CMake
and one Autotools, must produce the same module. Before this the Autotools
frontend accepted `deliverable_root` and ignored it, so its sibling source was
silently dropped where CMake's was shipped (bzl-kga).

One structural difference in how it is reached. The CMake frontend rebases
*after* building the graph, so it can survey every path and then pick a root.
The Autotools frontend rebases *inline*, because each path has to be resolved
against the directory its own command ran in — so the survey is a separate
pass over the declared sources, run before anything else, and the root is
already decided by the time the graph is built.

## Known gaps

- **Libtool wrapper scripts.** A program linking a `.la` gets a shell wrapper
  at its output path, not the binary; the real one is in `.libs/` and the
  wrapper re-execs it after setting `LD_LIBRARY_PATH`. Ground-truth capture
  copies the wrapper, which cannot run once moved. This is libtool's
  uninstalled-binary mechanism and has no CMake analogue — see
  [../lore/libtool-puts-a-wrapper-script-where-the-binary-goes.md](../lore/libtool-puts-a-wrapper-script-where-the-binary-goes.md)
  (bzl-yjn.4).
- **Two gaps are recovered but not yet escalated.** The frontend escalates
  unmapped config macros and sources that escape the module; it collects two
  more and discards them (bzl-yjn.5):
  - **External libraries.** A library the project links and does not build (a
    system `libintl`) is an input the generated module cannot satisfy.
  - **Declared targets `make` never produced.** `check_PROGRAMS` are the live
    case — see the target table above.

  Both fail *loudly* (an unresolved library, an absent target), which is why
  the silent one — a dropped source, surfacing as an undefined symbol several
  steps from its cause — was escalated first.
- **An already-configured source tree fails**, because `configure` refuses to
  run twice. Converting a tree someone has built in place is a normal thing to
  attempt.
