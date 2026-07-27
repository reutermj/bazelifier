# Bazel does not enforce `hdrs` vs `srcs` for C++ headers

## What we hit

While reasoning about `003-library-no-file-set` (a `cc_library` whose
header lands in `srcs` because CMake never declared it in a `FILE_SET`),
the natural assumption is that a consumer can't `#include` a header the
dependency didn't put in `hdrs` — that's what the attribute is *for*, and
under sandboxing an undeclared file shouldn't even be present.

That assumption is wrong, and it's wrong in a way that quietly changes how
much a green build proves.

## What's actually true

**A header listed in a dependency's `srcs` is still propagated as an input
to dependents' compile actions.** The consumer can `#include` it and the
build succeeds. `hdrs` vs `srcs` is documentation, not enforcement.

Verified on Bazel 9.2.0, autodetected host toolchain (`gcc`), sandboxed:

| dep declares header | `includes` set | consumer's `#include` | result |
| --- | --- | --- | --- |
| in `srcs` | yes | `"a.hpp"` | builds |
| in `srcs` | **no** | `"case_b/b.hpp"` | builds |
| **in no target at all** | yes | `"c.hpp"` | **fails** |

Third row's error:

```
case_c/main_c.cpp:1:10: fatal error: c.hpp: No such file or directory
```

And `bazel aquery` on the consumer's compile action lists the dependency's
`srcs` header directly among its inputs:

```
action 'Compiling src/main.cpp'
  Inputs: [..., src/greet.hpp, src/main.cpp]
```

## Why it's surprising

The intuitive mental model — "`includes` adds a `-I` path, so that's what
exposes the directory's contents" — gets the causality backwards. Rows two
and three are the ones that matter:

- Row 2 builds with **no `includes` attribute at all**, so propagation
  isn't coming from `includes`.
- Row 3 fails **with** `includes` set, because a `-I` path to a file that
  isn't an action input is useless.

What puts a file in a dependent's sandbox is being declared in *some*
target's `srcs`/`hdrs`. `includes` only decides how the `#include` is
spelled (`"a.hpp"` vs `"case_b/b.hpp"`).

Bazel does ship enforcement — the **`layering_check`** feature — but it
needs module maps and a clang-based toolchain, and it is **off by
default**.

## Why it matters here

A `needs_attention` item about header visibility can be "resolved" without
actually being fixed, and every gate still goes green: the build works, the
binary runs, and the runtime output matches ground truth byte for byte. An
agent that deleted the `needs_attention` markdown without touching `hdrs`
would be indistinguishable from one that did the work.

So for this class of gap, **a green build is not evidence the conversion is
right** — which is precisely why the `needs_attention/` gate exists ahead
of the equivalence comparison rather than relying on it.

## Open

The table above used the autodetected **host** toolchain. Fixtures actually
build under the hermetic **`llvm`** toolchain, which is clang-based and
could plausibly enable `layering_check`. If it does, a
`srcs`-header-with-consumer would fail to compile outright instead of
building with degraded encapsulation — a materially different failure mode.
Unverified: the check needs `github.com` archive downloads, which the
sandbox's egress policy currently blocks (`bcr.bazel.build` is fine).

## See also

- [build-verification.md](../architecture/build-verification.md#header-visibility-is-not-enforced-by-default)
- [cmake-frontend.md](../architecture/cmake-frontend.md) — why only
  `FILE_SET`-declared headers become `hdrs` in the first place.
