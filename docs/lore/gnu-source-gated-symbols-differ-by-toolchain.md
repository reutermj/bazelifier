# _GNU_SOURCE-gated symbols, and why the toolchain matters

glibc hides a set of functions behind the `_GNU_SOURCE` feature-test macro:
`vasprintf`, `strdup` (historically), `uselocale`, `duplocale`, `vsyslog`,
`arc4random`, and others. Their declarations in the system headers are wrapped
in `#ifdef __USE_GNU`, which `<features.h>` sets only when `_GNU_SOURCE` (or a
broad enough `_XOPEN_SOURCE`/`_DEFAULT_SOURCE`) is defined. A translation unit
that doesn't define `_GNU_SOURCE` never sees the declaration.

`check_symbol_exists` compiles a snippet that references the symbol, so a gated
symbol probes as **absent** unless the probe compiles with `_GNU_SOURCE`. Most
autoconf-style projects therefore set it — json-c does, via
`set(CMAKE_REQUIRED_DEFINITIONS -D_GNU_SOURCE)`. When cc_config's shared
catalog probe compiled a bare snippet, `HAVE_VASPRINTF` came out false;
json-c's `vasprintf_compat.h` then defined its own `vasprintf`, which collided
with glibc's (`static declaration of 'vasprintf' follows non-static
declaration`) because the consuming `.c` *does* define `_GNU_SOURCE` and so
sees glibc's declaration anyway. This is bzl-fxa.7; the fix is that the
catalog's symbol probes compile under `_GNU_SOURCE`.

**The trap that cost real time:** whether a symbol is gated depends on the
TOOLCHAIN, and the two toolchains in play here disagree.

- The **host `cc`** (gcc) that cc_config's own tests run against defaults to
  `-std=gnu*`, which implies `_GNU_SOURCE`. So `strdup`, `vasprintf`, etc.
  probe as *present even bare* under gcc.
- The **llvm/libc++ toolchain** that the CONVERTED modules build with does not
  imply `_GNU_SOURCE`, so the gating is real there.

Two consequences:

1. A cc_config unit test (host gcc) **cannot** show the bare→false direction —
   the bare probe is already true. The negative direction has to be a
   translator fixture that builds under llvm in the unpacked validation
   workspace (`020-gnu-source-symbol`).
2. `strdup` specifically is a **bad choice** for the fixture: it is not gated
   under this llvm/libc++, so the fixture would pass with or without the fix
   and prove nothing. `vasprintf` (the actual json-c symbol) *is* gated under
   llvm and is the one to use. Always confirm gating against the llvm
   toolchain, not the host gcc, before relying on it in a test.

Because `_GNU_SOURCE` only ever *widens* what is declared — it never hides a
standard symbol — compiling all the catalog's symbol probes under it is safe:
a symbol available without it is still available with it.
