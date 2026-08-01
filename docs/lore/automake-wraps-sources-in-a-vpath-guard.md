# automake wraps sources in a VPATH guard that looks like two sources

## The symptom

An Autotools conversion succeeds. The generated `BUILD.bazel` has a
`cc_library` whose `includes` list is empty, and the build fails with a header
that cannot be found — often `config.h`, sometimes a header sitting right next
to the source that includes it.

Nothing points at the compile command. The rule has the right sources, the
right name, the right dependencies. It is only missing its flags.

## The cause

With `subdir-objects` (which automake recommends and most projects enable), a
compile command does not name its source plainly. `make -n` emits:

```sh
gcc -DHAVE_CONFIG_H -I. -c -o lzmainfo.o \
  `test -f 'lzmainfo.c' || echo '/src/src/lzmainfo/'`lzmainfo.c
```

The backticked expression is a **VPATH guard**. It exists so one command works
for both an in-tree and an out-of-tree build: if `lzmainfo.c` is in the current
directory, `test -f` succeeds, `echo` never runs, and the substitution is
empty, leaving a bare `lzmainfo.c`. Otherwise the guard echoes the source
directory, which is prepended to the filename.

Tokenised as a shell would, that line ends in several arguments, and **two of
them look like a C source file**:

| token | what it is |
|---|---|
| `` `test `` | the start of the guard |
| `-f` | `test`'s flag |
| `'lzmainfo.c'` | **a decoy** — the file `test` is probing for |
| `\|\|` | shell |
| `echo` | the fallback |
| `` '/src/src/lzmainfo/'`lzmainfo.c `` | **the real source**, with the closing backtick glued in |

Picking the last token that *looks like* a source picks the decoy. The compile
is then attributed to `lzmainfo.c`, which no target owns, so its `-I` and `-D`
flags are attached to nothing — and the target that really owns that object
renders with no flags at all.

## The fix, and why it is the last argument rather than a smarter parse

Take the **last argument, unconditionally**. Everything before it is either a
flag or one of the guard's own words; the source is always final.

The closing backtick arrives glued to the path
(`` '/src/src/lzmainfo/'`lzmainfo.c ``), so splitting on that backtick and
rejoining the halves recovers the real path. A source with no guard has no
backtick and passes through untouched.

Note what this deliberately does *not* do: it never evaluates the guard. The
echoed directory is exactly the path wanted, so the "fallback" branch is the
one to keep. Actually running `test -f` would make the frontend's answer depend
on the state of the filesystem it happens to be run against.

## Why it has no CMake analogue

CMake+Ninja resolves the source path at generate time and writes it into the
build file. There is no VPATH, no guard, and no branch left in the command —
`cmake_api` reads a source path out of a structured reply and never has to
tokenise a shell fragment.

## How to notice it quickly

Grep the dry-run stream for a backtick:

```sh
make -n -B 2>/dev/null | grep -c '`test -f'
```

A non-zero count means every compile in that project carries a guard. If a
generated rule has empty `includes` while the original build clearly passed
`-I` flags, this is the first thing to check.
