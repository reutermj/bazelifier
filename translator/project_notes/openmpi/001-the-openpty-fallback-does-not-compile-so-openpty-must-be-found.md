# The non-`openpty` branch of `opal_pty.c` does not compile, so `HAVE_OPENPTY` has to be 1

**Applies to:** Open MPI 5.0.11 (`opal/util/opal_pty.c`, linked into `libopen-pal`)

The same shape as PMIx's `pmix_pty.c` (PMIx's own project note 001): Open
MPI inherited the file. Under `HAVE_OPENPTY` it is a one-line call to the C
library's `openpty`; otherwise it is a BSD-style implementation of its own,
which calls `opal_string_copy` without including its header and fails under
any C99-or-later compiler with "call to undeclared function". Nobody builds
that branch: every host that builds Open MPI has `openpty`.

The converted module builds against the llvm toolchain's own sysroot, a glibc
2.28, where `openpty` is declared by `pty.h` but defined only in libutil. The
catalog's link probe, linking no extra library, honestly answers "absent" —
and that is the branch that does not compile. What configure does on such a
system is `AC_SEARCH_LIBS([openpty], [util])`: find it in libutil and add
`-lutil`. Replicate that decision rather than probing: record `HAVE_OPENPTY`
and `OPAL_HAVE_OPENPTY` as 1 and give the library that compiles
`opal/util/opal_pty.c` `linkopts = ["-lutil"]`, which is harmless wherever
`openpty` already lives in libc (glibc 2.34 and later ship an empty libutil
for exactly this reason; so does musl).

Do not "fix" this by probing `openpty` and taking whatever comes back: on this
toolchain the answer is the broken branch.

## Upstream

`opal/util/opal_pty.c`'s `#else` branch (the `ptym_open`/`ptys_open`
implementation) needs `#include "opal/util/opal_string_copy.h"`. The
conversion keeps the build's behaviour — it finds `openpty` — rather than
patching the source.
