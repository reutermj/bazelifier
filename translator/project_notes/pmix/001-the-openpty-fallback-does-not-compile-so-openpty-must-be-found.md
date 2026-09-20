# The non-`openpty` branch of `pmix_pty.c` does not compile, so `HAVE_OPENPTY` has to be 1

**Applies to:** PMIx 5.0.11 (`src/util/pmix_pty.c`, `libpmix.la`)

`pmix_pty.c` has two implementations of `pmix_openpty`: under
`HAVE_OPENPTY` a one-line call to the C library's `openpty`, otherwise a
BSD-style implementation of its own. The second one calls
`pmix_string_copy` without including `src/util/pmix_string_copy.h`, so
under any C99-or-later compiler it fails with "call to undeclared
function". Nobody has compiled that branch in years: every host that
builds PMIx has `openpty`.

The converted module builds against the llvm toolchain's own sysroot, a
glibc 2.28, where `openpty` is declared by `pty.h` but defined only in
libutil. A link probe for it, linking no extra library, honestly answers
"absent" — and that is the branch that does not compile. What configure
does on such a system is `AC_SEARCH_LIBS([openpty], [util])`: find it in
libutil and add `-lutil`. The module replicates that decision rather than
probing: `HAVE_OPENPTY` and `PMIX_HAVE_OPENPTY` are recorded as 1 and
`libpmix.la` carries `linkopts = ["-lutil"]`, which is harmless everywhere
`openpty` already lives in libc (glibc 2.34 and later ship an empty
libutil for exactly this reason; so does musl).

Do not "fix" this by probing `openpty` and taking whatever comes back: on
this toolchain the answer is the broken branch.

## Upstream

`src/util/pmix_pty.c`'s `#else` branch (the `ptym_open`/`ptys_open`
implementation) needs `#include "src/util/pmix_string_copy.h"`. The
conversion does not take that branch, so the missing include is not the
module's problem; it is why the module cannot let a probe decide.
