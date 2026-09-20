# A discarded address-of is not a link test

**Found on:** PMIx, 2026-09-20. **Where it lives now:** `cc_config/cc_config/probe.bzl`,
`_check_symbol_exists_impl`; pinned by the `crypt` probe in that package's BUILD.

`check_symbol_exists` is documented as a LINK probe: a symbol that is
declared but that no library supplies must answer false. Its body was

    (void)((void *)(&symbol));

which reads as "reference the symbol so the linker must resolve it" and
does not: the expression has no side effect, the compiler drops it, and
the object carries no relocation. Every "link" answer the catalog had ever
given was compile-only — anything a header declared answered true.

It surfaced only when the two could differ. The converted modules link
against the llvm toolchain's own sysroot, a glibc 2.28, where `openpty`
is declared by `pty.h` but defined in libutil; the probe said present,
PMIx took its `HAVE_OPENPTY` branch, and twenty binaries failed to link
on the one symbol. On the conversion host (glibc 2.39, openpty in libc)
the same probe would have been right by accident, which is why nothing
before PMIx noticed: every earlier symbol the corpus asked about was
either a macro or lived in libc.

CMake's `check_symbol_exists` avoids this with

    return ((int *)(&symbol))[argc];

— the address indexed by a runtime value cannot be folded, so the
reference stays and the link has to resolve it. That is the body now, in
the probe and in the harvester's host mirror. The general lesson: a probe
that is supposed to fail on some platform needs a test that fails on
THIS one, or it is a probe that answers yes.
