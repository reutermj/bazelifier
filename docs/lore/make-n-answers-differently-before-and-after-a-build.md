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

## The cost, and the one wrinkle

`-B` forces the maintainer rules to re-run too, so the stream is prefixed by
~90 lines of `config.status --recheck` output before any compile appears. That
is why `parse_commands` recognises only the handful of programs that build
something and ignores every other line — the preamble is noise that must not be
mistaken for build commands.

It also means the stream is **not byte-identical between runs**, only stable in
the command ordering that matters. A claim that it is byte-identical was
written down before `-B` was needed and is wrong.

## How to notice it quickly

If the frontend produces targets with no sources, or a graph with no targets at
all, check whether the dry run returned anything:

```sh
make -n -B 2>/dev/null | grep -cE '^(gcc|ar|ranlib|/bin/bash \./libtool)'
```

Zero means the stream is empty or is all preamble, not that the project has
nothing to build.
