# `make -n` answers differently before and after a build

**Resolved: the frontend no longer dry-runs at all.** It captures the real
build's stdout, because make echoes every command as it runs it. This entry is
kept because the dead end below is the obvious first design, and the states it
documents are why.

## The symptom

The Autotools frontend needs the resolved command stream. Reaching for
`make -n` gives two orderings, and both fail:

- Right after `configure`, before building. On a recursive project this
  **exits 2**:

  ```
  make[2]: *** No rule to make target '../../src/liblzma/liblzma.la',
    needed by 'xzdec'.  Stop.
  ```

- After the build instead. It **succeeds and prints nothing**:

  ```
  make[1]: Nothing to be done for 'all'.
  ```

So the input is either an error or empty, and the obvious fix for one is the
other.

## The cause

`make -n` is not "print the build plan." It is "run the build, but echo
commands instead of executing them" — and everything else about make's
behaviour is unchanged. It still consults timestamps, and it still recurses.

Before the build, recursion bites. `make` descends into `src/xzdec` and asks
how to build `xzdec`, which needs `../../src/liblzma/liblzma.la`. That file
does not exist yet, and the rule that would make it lives in a *different*
makefile this sub-make has not read. Ordinarily fine, because `SUBDIRS` builds
`src/liblzma` first and the file is simply there by the time anything asks.
With `-n` nothing is ever created, so the dependency the ordering was meant to
satisfy is still missing.

After the build, timestamps bite. Every target is up to date, so the honest
answer to "what would you run" is "nothing."

## The dead end: `-B`

`-B` (`--always-make`) ignores timestamps and considers everything out of
date, which answers the right question — *what commands build this project*
— and a built tree is the only state where every subdirectory can answer it.

It works, and it is **98% of the conversion's runtime**. Measured on xz 5.4.7:

| | |
|---|---|
| total conversion | 264s |
| `make -n -B` | **258s** |
| `config.status` invocations | **1,404** |
| lines emitted | 15,402, of which 735 unique |
| lines that are actual compile/link commands | **26** |

`-B` marks the **maintainer** rules out of date along with the build rules, so
each of 135 recursive `make` calls re-runs `config.status --recheck` to
regenerate `configure` and every `Makefile` — none of which the frontend reads.

Two narrower escapes also fail, and one is a trap:

- **`make clean` first, then `make -n`.** Identical to the fresh-tree failure:
  `clean` removes `src/liblzma/liblzma.la`, so `src/xzdec` stops with *No rule
  to make target*. Deleting only `*.o`/`*.lo` and keeping the libraries does
  work — 106 objects, 0s — but see below for why it is moot.
- **`make -n -B -j16`.** Finishes in 19s and looks like a 13x win. It is how
  long xz takes to **fail**: rc=2, zero compile or link commands emitted, all
  926 lines `config.status` chatter. `-B` marks `configure` itself out of date
  and parallel jobs race regenerating it, clobbering a shared scratch file
  (`cat: confdefs.h: No such file or directory`). Judge any change here by the
  object set and the command diff, **never by wall time**.

## The fix: don't dry-run

The build already runs — ground-truth artifacts come from it — and make
**echoes every command as it executes it**. That output *is* the stream:

```sh
configure
make -j16 --output-sync=recurse      # capture stdout; this IS the stream
```

Verified on xz: **byte-identical** compile/link commands to the `-B` stream,
the same 9 directories announced, and identical directory attribution for all
26 outputs. Conversion went **264s → 6s**, with byte-identical generated
`BUILD.bazel`, `MODULE.bazel`, `TARGETS` and escalations.

`--output-sync=recurse` keeps each sub-make's `Entering directory` attached to
the commands it encloses. `parse_commands` reads those as a stack, so
interleaved output would silently attribute a compile to the wrong directory.
It costs nothing measurable.

## What it costs, and what enforces it

The build directory must be **empty**. A second `make` over a built tree
prints nothing — the same "nothing to do" that started all this — so a graph
with no targets is now a reachable failure. `discover` therefore checks the
parsed stream and fails loudly rather than emitting an empty conversion.

## How to notice it quickly

If a conversion produces targets with no sources, check whether the stream
had anything:

```sh
make -j16 --output-sync=recurse 2>/dev/null \
  | grep -cE '^(gcc|ar|ranlib|/bin/bash \./libtool)'
```

Zero means the build directory was not clean, not that the project has nothing
to build.
