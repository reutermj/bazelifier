# Libtool puts a wrapper script where the binary goes

## The symptom

An Autotools project converts, the generated module builds hermetically, and
`bazel run //:greeter` prints the right thing. Then the ground-truth
comparison fails:

```
../greeter+/ground_truth/greeter: error:
  '/.../execroot/_main/external/greeter+/ground_truth/.libs/greeter' does not exist
```

The error names a path nobody wrote, inside a directory the conversion did
copy. Every part of the setup looks right.

## The cause

When a program links a **libtool** library (`lib_LTLIBRARIES`, a `.la`), the
file `make` leaves at the program's output path is not the program:

```
$ file ground_truth/greeter
#! /bin/bash
# greeter - temporary wrapper script for .libs/greeter
```

The real executable is `.libs/greeter`. The wrapper exists because that
executable is linked against an **uninstalled** shared library — one still
sitting in `.libs/` rather than in `/usr/local/lib` — so it cannot run until
something points the loader at it. The wrapper sets `LD_LIBRARY_PATH` and
re-execs the real binary. After `make install`, the installed program needs no
wrapper because the library is on the system path by then.

So `copy_ground_truth_artifacts` faithfully copies the path the build reported,
and the result is a shell script whose only job is to find a sibling directory
that was not copied with it.

## Why it has no CMake analogue

CMake links against the build-tree `.so` directly and sets `RPATH` on the
binary, so the executable at the output path *is* the executable. There is
nothing between the build and the artifact.

This is the one genuinely new concept libtool contributes, and it was predicted
before it bit: a `.la` is not a library at all but a control file, and it can
stand for a shared library, a static one, or both.

## The near-miss

It is tempting to read this as the same bug as
[ground-truth-so-lives-at-the-root-not-beside-the-binary.md](ground-truth-so-lives-at-the-root-not-beside-the-binary.md),
or as the Bazel-side problem where a dynamically linked test binary could not
find its `.so` once staged into a writable tree. Both are about a binary
failing to locate a shared library, and both were fixed by changing what got
staged where.

They are different in the way that matters:

- Those were about a **real binary** that could not find a real library.
- This is about a **shell script** standing where a binary was expected. No
  amount of staging the `.so` correctly helps, because the thing being compared
  is not a program.

The fix therefore is not "also copy `.libs/`" by reflex — it is first deciding
*what the ground-truth artifact should be* for a libtool program: the wrapper
plus enough of its surroundings to run, the real binary from `.libs/`, or a
recorded transcript of the wrapper's output. Each changes what "ground truth"
means slightly, which is why it is a decision rather than a patch.

## How to notice it quickly

`file` on the captured artifact, or `head -3`. A libtool wrapper announces
itself in a comment on line 2. If the ground-truth comparison ever reports a
missing path under `.libs/`, this is why.
