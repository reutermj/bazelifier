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

### Adding a third

The pattern exists twice now, so it is worth writing down rather than
re-derived. Five places, none of which are obvious from any one file:

1. `main.rs` — a `Frontend` enum variant.
2. `main.rs::detect_frontend` — the file that marks a project of that kind.
   Detection deliberately does NOT fall back to a default: a directory with
   no marker is not a project this tool converts, and guessing produces a
   confusing failure deep in a frontend instead of at the entry point.
3. `main.rs::run` — one dispatch arm calling your `discover`.
4. `build_defs/convert_cmake_project.bzl` — a `_FRONTENDS` entry (marker
   file + display label) and the value in the rule's `values` list.
5. The same file — a `convert_<x>_project` wrapper, which is a thin call
   passing `frontend = "<x>"`.

`--frontend` is passed explicitly by the rule rather than left to detection,
because a project can ship both: xz has `CMakeLists.txt` and `configure.ac`,
and which to convert is the BUILD author's choice rather than something to
infer.

What a frontend owes the rest of the pipeline is one function:

```rust
pub fn discover(source_dir: &Path, build_dir: &Path, deliverable_root: &Path)
    -> Result<Discovery, Error>
```

Everything downstream consumes `Discovery` without knowing which frontend
produced it. Two obligations are easy to miss and both have bitten:

- **Honour `deliverable_root`.** The module root is DERIVED — it widens from
  the project directory to cover anything the build references from inside
  the deliverable, and `deliverable_root` caps that widening. Accepting the
  parameter and ignoring it made two equivalent projects convert differently
  with nothing reporting it (bzl-kga).
- **Escalate rather than drop.** Anything you cannot express must produce a
  `needs_attention` item. A silent drop surfaces as an undefined symbol at
  link time, several steps from the cause — and if the CMake frontend
  handles the same case, agree with it deliberately rather than by accident
  (bzl-7nd).

Two things you will probably NOT need, based on both existing frontends:
a new `model` field, and any change to `codegen`. If you find yourself
adding either, that is worth pausing on — it may be a genuine gap in the
model, or it may be the frontend's shape leaking past the boundary this
document exists to check.

## Source of truth: the command stream, not `Makefile.am`

**Decision:** the frontend reads the build system's own resolved output, the
same principle [cmake-frontend.md](cmake-frontend.md) states for the File API.
Three candidates were compared against GNU hello and a purpose-built project:

| candidate | resolved? | practical |
|---|---|---|
| `Makefile.am` | **no** | closest in intent, 137 lines |
| generated `Makefile` | yes | 4666 lines of make syntax |
| the build's own stdout | yes | the command stream |

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

make echoes every command as it runs it, fully expanded, so the build's own
stdout is the resolved stream:

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
(bzl-sjp). It is not byte-identical — a build interleaves make's own progress
chatter with the commands — which is why the frontend recognises only the
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

`discover` configures, then **builds** — and the build's own stdout is the
command stream. make echoes every command as it runs it, so there is no
separate interrogation pass:

```sh
configure
make -j16 --output-sync=recurse      # capture stdout; this IS the stream
```

The build has to happen anyway, since ground-truth artifacts come from it (see
[build-verification.md](build-verification.md)), so the stream is free.

`--output-sync=recurse` is load-bearing rather than tidiness: it keeps each
sub-make's `Entering directory` attached to the commands it encloses, and
`parse_commands` reads those as a stack. Interleaved output would silently
attribute a compile to the wrong directory — a wrong graph, not an error.

A `make -n` dry run was used for this and is a dead end in both orderings
(before the build it exits 2 on a recursive project; after it, it prints
nothing). `make -n -B` resolves that and costs 258 of xz's 264 conversion
seconds, because `-B` marks the *maintainer* rules out of date too. See
[../lore/make-n-answers-differently-before-and-after-a-build.md](../lore/make-n-answers-differently-before-and-after-a-build.md).

**The build directory must be empty.** make reports only work it actually
does, so a second run over a built tree yields no commands at all. That is now
a reachable failure rather than a hypothetical, so `discover` checks the
parsed stream and fails loudly rather than emitting a graph with no targets.

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
