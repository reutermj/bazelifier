# gnulib generates system headers, and only the build says which

## What gnulib is

gnulib is the GNU Portability Library. Unlike a normal library you do not
link it — you copy its source into the project (`gl/`, sometimes `lib/`),
and `configure` decides per function whether your platform needs a
replacement.

The part that matters for conversion: for each system header it cares about,
gnulib ships `string.in.h` and *generates a real `string.h`* into the build
tree, placed ahead of `/usr/include`. So `#include <string.h>` finds
gnulib's, and its `#include_next <string.h>` reaches the platform's behind
it.

## Do not decide this by measuring whether it matters here

On glibc/Linux the replacements are very nearly inert. Measured on libidn2:
11 headers generated, and preprocessing a file that includes gnulib's
`string.h` yields **zero** active `rpl_` redirects. Every `gl/*.h` can be
deleted and both the library and the CLI still build, with byte-identical
runtime output.

That is a fact about the conversion host, not the project. gnulib exists
because the answer differs on musl, macOS and Windows. See
[overview.md](../architecture/overview.md#replicate-the-builds-behaviour-not-this-hosts-outcome).

The dependence also varies wildly by project, so a per-platform argument
would not even be consistent:

| | replacement headers | gnulib objects built | genuine `rpl_` replacements |
|---|---|---|---|
| libidn2 2.3.7 | 11 | 10 of 31 | 1 |
| gzip 1.13 | 18 | 55 of 118 | 10 |

gzip's stdio *is* gnulib's — `rpl_fopen`, `rpl_fclose`, `rpl_printf` — and
its own sources call `error()` 27 times, which has no libc equivalent.

## Where the evidence is: the generation recipe, nowhere else

Two sources look authoritative and are not. Both were measured on libidn2:

- **`<HEADER>_H` variables in `make -p`.** `ALLOCA_H = alloca.h`, `ASSERT_H`
  empty — this looks exactly like a per-header decision. It **misses six of
  eleven** (there is no `STRING_H` at all) and **invents `iconv.h`**, which
  is set but never generated. The variable states intent, not outcome.
- **The `<name>.h:` rules in the generated Makefile.** 16 exist, 11 fire.
  Counting rules over-reports.

What works is the build's own `V=1` output, which carries the template, the
output name and every substituted value:

```
sed -e 's|@''NEXT_STRING_H''@|<string.h>|g' ... < gl/string.in.h > string.h-t1
mv string.h-t1 string.h
```

631 substitutions across 11 headers on libidn2, all present. Requires `V=1`
— these recipes are silenced by `AM_SILENT_RULES`, see
[am-silent-rules-empties-the-command-stream.md](am-silent-rules-empties-the-command-stream.md).

## Three things about the recipe that hand-written fixtures got wrong

Each of these passed a unit test written from my assumption and failed
against the real stream:

1. **The redirect names a TEMP file.** `> string.h-t1`, and a later
   `mv string.h-t1 string.h` names the header. Keying on the redirect yields
   a file nobody includes.
2. **Two recipe forms.** Four of libidn2's headers pass the template on
   stdin (`< foo.in.h >`), seven pass it positionally. Handling one found
   4 of 11 and looked like it worked.
3. **Both sed delimiters in one recipe** — `s|...|` where a value may contain
   a slash (`<string.h>`), `s/.../` elsewhere. Knowing one silently drops
   half the substitutions.

## Reproducing it needs no new Bazel capability

The two things that sound hard are not:

- **Generation** is the existing `config_header` rule, unchanged. gnulib
  templates are plain `@VAR@`, the dialect the translator already expands.
- **Shadowing** is plain `includes` on a `cc_library`. Bazel puts a dep's
  `includes` ahead of the toolchain's own search path, so `#include_next`
  resolves correctly with no `copts` or `-isystem` ordering. Verified end to
  end before any of this was built.

A real `gl/limits.in.h` reproduced through that path came out **byte-identical**
to what autotools generated, once `#undef` handling was fixed (see below).

## Two translator bugs it exposed, both pre-existing

- **A spaced `# undef GUARD` was treated as a declaration** and consumed, in
  both `config_header.rs` and the Starlark-side expander. gnulib writes it
  that way inside an `#if` to undefine a guard it set itself; deleting the
  line breaks the split double-inclusion guard `#include_next` depends on.
  autoheader always emits `#undef NAME` unspaced at column 0, so requiring
  the exact form is what separates a declaration from ordinary C.
- **Target names derived from the output filename collide.** libidn2 vendors
  *two* gnulib trees — `gl/` and `unistring/`, the latter from libunistring —
  each generating `limits.h`, `stddef.h`, `string.h` and four more. Two
  rules of one name is a hard analysis error, so the module could not build
  at all.
