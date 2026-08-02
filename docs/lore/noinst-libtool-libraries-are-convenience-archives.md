# A noinst_ libtool library is not a shared library

## The symptom

An Autotools project converts, and Bazel refuses the module at analysis:

```
Error in fail: Two shared libraries in dependencies link the same
library statically. Both //:libgnu.la_shared and //:libidn2.la_shared
link statically //:libgnu.la
```

## The cause

`is_shared` came from the primary suffix alone — `LTLIBRARIES` is libtool's
form, so it must be shared. The destination prefix overrides that:

| declaration | libtool builds |
|---|---|
| `lib_LTLIBRARIES = libidn2.la` | `libidn2.so`, installed |
| `noinst_LTLIBRARIES = libgnu.la` | `libgnu.a` only — **no `.so` at all** |

`noinst_` means built but never installed, and for a libtool library that
makes it a *convenience archive*: something to be absorbed into whatever
links it. Verified on libidn2's own build — `gl/.libs/` holds `libgnu.a` and
nothing else, while `lib/.libs/` holds `libidn2.so`.

Emitting a `cc_shared_library` for the convenience archive AND absorbing it
into the real one is a contradiction Bazel catches.

## What the absorption actually is, which is subtler than it looks

Worth writing down because the obvious model is wrong in two ways, and both
happen to give the right answer on this one project:

- **It is selective, not wholesale.** `libidn2.so` contains `cloexec` and
  `malloca` symbols but not `basename-lgpl`, `getprogname`, `version_etc` or
  `stat-time`. The linker pulls only the archive members actually
  referenced. "Absorbed" is not a whole-library relationship.
- **It is transitive.** Those members arrive via `libunistring`, not
  directly: `libidn2.la <- libunistring.la <- libgnu.la`. `lib/*.c`
  references none of them. A model treating `LIBADD` as a direct edge is
  right here by luck.
- **The artifact records nothing.** `libidn2.la` has `dependency_libs=''`
  and the `.so` NEEDs only `libc.so.6`. The only evidence is the
  `Makefile.am` and the symbols themselves.

## How common the failing shape is

Surveyed seven projects. The question is not "does it have a convenience
archive" but "does one get absorbed into an INSTALLED library":

| project | `noinst_LT` | `lib_LT` | absorbs into an installed library? |
|---|---|---|---|
| libidn2 2.3.7 | 4 | 1 | **yes**, 3 `LIBADD` lines |
| libmicrohttpd 1.0.1 | 1 | 2 | no — `libmicrohttpd2.la` is standalone |
| wget 1.21.4 | 1 | 0 | no — absorbs into a **program** |
| libunistring 1.2 | 0 | 1 | no |
| gzip / xz / jansson | 0 | 0–1 | no |

One in seven. The common shapes need nothing special: absorbing into a
program needs no `cc_shared_library`, and a standalone `noinst` library is
just a `cc_library`.

## Fixed, and deliberately not fixed

Fixed: `is_shared` now requires `destination != "noinst"`. That is the whole
of the "libtool library" question and it is well-evidenced — the destination
prefix is the input stating it, and libmicrohttpd exercises the negative.

**Not** fixed: expressing the absorption itself. Bazel wants a static dep
absorbed into a shared library named in `static_deps`, and codegen emits
none, so libidn2 still fails one error further on. Left open on purpose —
`static_deps` is a whole-library declaration and the real relationship is
selective and transitive, so it is a coarser claim than one project's
evidence supports. Modelling a Bazel attribute from a single witness is
exactly what the corpus rule about measuring a shape across five projects
exists to prevent.
