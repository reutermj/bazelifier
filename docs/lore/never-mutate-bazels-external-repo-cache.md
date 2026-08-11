# Never edit anything under Bazel's external repo cache

## The rule

`~/.cache/bazel/_bazel_*/*/external/<repo>+/` is **read-only in practice**.
Read it freely — that is what
[the CLAUDE.md convention about investigating fetched rulesets locally]
recommends — but never delete, edit or create a file there. To experiment,
copy the tree somewhere else and work on the copy.

## Why it bites harder than an ordinary mistake

Three properties compound:

- **It is a symlink into a SHARED content cache.** The path under
  `external/` points at
  `.cache/bazel/_bazel_*/cache/repos/v1/contents/<hash>/<uuid>`, keyed by
  the archive's hash. Every workspace that fetches the same archive gets the
  same directory.
- **`bazel clean` does not restore it.** Nor does `bazel clean --expunge`
  reliably, if the mutation is in the shared content store rather than the
  output base.
- **`git status` cannot see it.** The tree is outside the repo, so nothing
  in the normal review loop reports the change.

So a one-line `rm` becomes a persistent, invisible modification to a source
tree that later builds treat as pristine.

## The instance

libidn2 stopped converting with:

```
autom4te: error: cannot open autom4te.cache/requests: Read-only file system
make: *** [Makefile:1771: .../Makefile.in] Error 1
```

which reads like a stale checkout, a maintainer-mode rebuild, or a
timestamp-ordering problem. It was none of those. `aminclude_static.am` had
been deleted from the cache by hand, on the theory that it was pollution
left by an earlier run — and `Makefile.in` lists it as a prerequisite, so
make tried to regenerate `Makefile.in` and failed against the read-only
sandbox.

Three separate diagnoses were written and committed before the real one.

## The tell that misled, and the check that settles it

The file's mtime was "today", which looked like proof it had been generated
during a conversion. **A recent mtime says nothing about origin** — Bazel
sets extraction times when it unpacks.

The one command that answers it:

```sh
tar tzf <the archive> | grep <filename>
```

If the file is in the tarball, it is a source file. `aminclude_static.am`
is: autoconf generates it at `autoreconf` time and the release tarball
ships it.

## Recovering

```sh
bazel clean --expunge     # then re-fetch, which costs a full toolchain rebuild
```

Removing the marker file and the content-cache directory by hand does NOT
work and leaves a worse state — `The repository's path ... does not exist or
is not a directory` — because Bazel still has the repo mapped. Expunge is
the supported route.
