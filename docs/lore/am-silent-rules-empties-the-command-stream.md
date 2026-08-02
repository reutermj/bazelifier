# AM_SILENT_RULES converts a project to nothing, without failing

## The symptom

An Autotools project converts. The translator reports success, writes a
`BUILD.bazel`, and the module contains no targets — or trips the empty-stream
guard and fails with a message about the build producing no commands.

Nothing in the project looks unusual. It configures, it builds natively, its
`Makefile.am` declares ordinary primaries.

## The cause

The project has `AM_SILENT_RULES([yes])` in `configure.ac`. automake then
prints a progress line instead of the command:

```
  CC       lzmainfo.o
```

The Autotools frontend's entire input is the command stream — the build's own
stdout, which it reads as the resolved graph. Silent rules make that stream
empty of compile commands, so there is nothing to read.

Measured on libidn2 2.3.7, clean tree:

```
make -j8         ->   0 compile/link commands
make -j8 V=1     -> 226
```

## Why it stayed invisible for four onboardings

None of xz, expat, jansson or libmicrohttpd uses the macro. jansson emits 36
compile commands with or without `V=1`, so nothing in the corpus
distinguished the two modes. The first project that enabled silent rules was
also the first to expose it.

That is the general shape worth remembering: a frontend that reads *build
output* is sensitive to build *verbosity settings*, and those are per-project
and invisible until one differs.

## The fix, and why it is unconditional

`build()` passes `V=1` always. The narrower version — read `configure.ac`,
pass the flag only when the macro is present — was rejected because it means
reading an INPUT file, which this frontend deliberately does not do (see
[autotools-frontend.md](../architecture/autotools-frontend.md) on reading
resolved output rather than declarations).

The broad version measured free. All four Autotools corpus projects convert
byte-identically with it, with identical target, test and escalation counts,
and zero unresolvable source paths — the last being the real risk, since
`V=1` also unsilences libtool and `mkdir`, and `parse_commands` reads
`Entering directory` announcements as a stack that interleaving under `-j`
could corrupt.

Fixture `autotools/007-silent-rules` pins it. Two sources on purpose: with an
empty stream a target still renders from its declaration, so a one-source
fixture looks fine and only a *missing* source shows the loss.

## The side benefit

`V=1` is also what makes gnulib's replacement headers visible — their
generation recipes are silenced by the same mechanism. See
[gnulib-replacement-headers.md](gnulib-replacement-headers.md).
