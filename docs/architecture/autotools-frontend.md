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
gcc -DHAVE_CONFIG_H -I. -Isrc -g -O2 -c -o src/greet.o src/greet.c
ar cru libgreet.a src/greet.o
libtool --mode=link gcc -o greeter src/main.o libgreet.a libshout.la
```

It is also **byte-identical between runs** — verified, and more deterministic
than the CMake File API, which reports dependency order unstably (bzl-sjp).

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
| `bin_PROGRAMS`, `check_PROGRAMS` | `TargetKind::Executable` |
| `noinst_LIBRARIES`, `lib_LIBRARIES` | `TargetKind::Library`, `is_shared = false` |
| `lib_LTLIBRARIES` | `TargetKind::Library`, `is_shared = true` |

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

This needs no `Makefile.am` parsing: `make -p` reports the primary, and
`make -n install` shows the install rule if confirmation is wanted.

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

`discover` configures, then interrogates, then builds — and the interrogation
must come **before** the build, unlike the CMake frontend where the order does
not matter.

`make -n` on a fully built tree prints "nothing to do". The command stream is
the entire input, so taking the dry run after the build yields nothing at all.

## Known gaps

- **Libtool wrapper scripts.** A program linking a `.la` gets a shell wrapper
  at its output path, not the binary; the real one is in `.libs/` and the
  wrapper re-execs it after setting `LD_LIBRARY_PATH`. Ground-truth capture
  copies the wrapper, which cannot run once moved. This is libtool's
  uninstalled-binary mechanism and has no CMake analogue — see
  [../lore/libtool-puts-a-wrapper-script-where-the-binary-goes.md](../lore/libtool-puts-a-wrapper-script-where-the-binary-goes.md)
  (bzl-yjn.4).
- **No escalations.** The frontend models what it can see and does not yet
  detect the gaps worth escalating. Honest now, wrong as soon as it meets a
  project it cannot fully convert.
- **External libraries are recognised but not reported.** A library the project
  links and does not build (a system `libintl`) is collected and currently
  discarded; it is an input the generated module cannot satisfy, so it deserves
  an escalation.
- **An already-configured source tree fails**, because `configure` refuses to
  run twice. Converting a tree someone has built in place is a normal thing to
  attempt.
- **Recursion is untested.** `SUBDIRS` appears in the database, so a recursive
  project is *detectable*; GNU hello is non-recursive for target purposes
  (its one subdirectory builds no C), so nothing has exercised the recursive
  path.
