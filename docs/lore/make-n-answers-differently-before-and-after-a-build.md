# `make -n` answers differently before and after a build

## The symptom

The Autotools frontend reads `make -n` — the resolved command stream is its
whole input. Two orderings suggest themselves, and both fail:

- Run `make -n` right after `configure`, before building. On a recursive
  project this **exits 2**:

  ```
  make[2]: *** No rule to make target '../../src/liblzma/liblzma.la',
    needed by 'xzdec'.  Stop.
  ```

- Run `make -n` after the build instead. It **succeeds and prints nothing**:

  ```
  make[1]: Nothing to be done for 'all'.
  ```

So the input is either an error or empty, and the obvious fix for one is the
other.

## The cause

`make -n` is not "print the build plan." It is "run the build, but echo
commands instead of executing them" — and *everything else* about make's
behaviour is unchanged. In particular make still consults timestamps, and it
still recurses.

Before the build, recursion is what bites. `make` descends into `src/xzdec`
and asks how to build `xzdec`, which needs `../../src/liblzma/liblzma.la`.
That file does not exist yet, and the rule that would make it lives in a
*different* makefile that this sub-make has not read. Ordinarily that is fine,
because `SUBDIRS` builds `src/liblzma` first and the file is simply there by
the time anything asks. With `-n`, nothing is ever created, so the dependency
that the ordering was supposed to satisfy is still missing when it is needed.

After the build, timestamps are what bite. Every target is now up to date, so
the honest answer to "what would you run" really is "nothing."

## The fix

Build first, then dry-run with `-B`:

```sh
make            # real build: also produces the ground-truth artifacts
make -n -B      # what a full rebuild WOULD run
```

`-B` (`--always-make`) tells make to ignore timestamps and consider everything
out of date. That answers the question the frontend is actually asking — *what
commands build this project* — rather than *what is left to do right now*. And
a fully built tree is the only state in which every subdirectory can answer it,
because the inter-directory artifacts the recursive case tripped over now
exist.

The build is not overhead added for this: ground-truth artifacts for the
equivalence check come from it either way.

## The cost, which is most of the conversion

`-B` forces the **maintainer** rules out of date along with the build rules,
and that is where nearly all the time goes. Measured on xz 5.4.7:

| | |
|---|---|
| total conversion | 264s |
| `make -n -B` | **258s** |
| `config.status` invocations | **1,404** |
| lines emitted | 15,402, of which 735 unique |
| lines that are actual compile/link commands | **26** |

Each of 135 recursive `make` calls re-runs `config.status --recheck` to
regenerate `configure` and every `Makefile`, none of which the frontend reads.
That preamble is also why `parse_commands` recognises only the handful of
programs that build something and ignores every other line.

It also means the stream is **not byte-identical between runs**, only stable in
the command ordering that matters. A claim that it is byte-identical was
written down before `-B` was needed and is wrong.

## A fourth state, which is the cheap one

`-B` is not the only way to make a built tree answer. Deleting just the
**object files** — leaving the libraries in place — makes exactly the build
rules out of date and nothing else:

```sh
make                                          # build (ground truth too)
find . \( -name '*.o' -o -name '*.lo' \) -delete
make -n                                       # no -B needed
```

On xz that is **0s instead of 258s**, and it is equivalent rather than merely
faster: all 26 compile/link commands are byte-identical to the `-B` stream,
the object sets match (106 vs 106 — `-B`'s extra `-o file.o` is a `configure`
probe artifact, not a build output), `make -p -n`'s database is unaffected,
and `config.status` runs 0 times instead of 1,404.

It works because the cross-directory artifacts survive. `make clean` does
**not** work, for exactly the reason the fresh tree does not: it removes
`src/liblzma/liblzma.la`, and `src/xzdec` stops with `No rule to make target`.

So there are four states, not two:

| tree | `-B`? | objects reported | |
|---|---|---|---|
| built | no | 0 | nothing to do |
| fresh | no | 83 of 106 | rc=2, cross-dir dep missing |
| built | yes | 106 | correct, 258s |
| built minus `*.o` | no | 106 | correct, **0s** |

**Not yet adopted** — the frontend still passes `-B` (bzl-ccv.6). Verified
only on xz, and a project that emits artifacts which are neither `.o`/`.lo`
nor the final target (generated sources, a code generator built and run
mid-build) could still report "nothing to do" for those steps. The check
before adopting it is the object-set and command comparison above, per
project — never wall time, which is how `-j16` briefly looked like a 13x win
while actually exiting 2 having emitted no build commands at all.

## How to notice it quickly

If the frontend produces targets with no sources, or a graph with no targets at
all, check whether the dry run returned anything:

```sh
make -n -B 2>/dev/null | grep -cE '^(gcc|ar|ranlib|/bin/bash \./libtool)'
```

Zero means the stream is empty or is all preamble, not that the project has
nothing to build.
