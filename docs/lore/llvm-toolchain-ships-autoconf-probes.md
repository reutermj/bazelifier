# The llvm toolchain already ships an autoconf-in-Bazel implementation

## What we found

Building `cc_config` (Bazel-native `configure_file` config headers — see
[../architecture/configure-file-and-toolchain-probes.md](../architecture/configure-file-and-toolchain-probes.md))
started with the question "can a Bazel rule run an autoconf-style compile
probe against the resolved toolchain, and capture failure as *data* rather
than aborting the build?" Investigating the fetched `@llvm` repo (per
CLAUDE.md's "read the fetched repo's source" rule) turned up that the
toolchain **already contains a complete, working implementation** of exactly
this, under:

```
@llvm//3rd_party/gcc/libstdcxx/autoconf/
  cc_configure_probe.bzl   # compile/link probe execution
  checks.bzl               # header/function/type/decl check shapes + AC_* aliases
  autoconf_config.bzl      # a config-header emitter consuming probe results
  autoconf_hdr.bzl
  providers.bzl
```

It exists because building GCC's libstdc++ hermetically requires running
libstdc++'s own `configure` logic, and there is no upstream autoconf-in-Bazel
ruleset — so the toolchain grew one.

## The mechanics it confirms (the useful part)

Whatever we build, this is the proven shape, worth not re-deriving from
scratch:

- Resolve the toolchain with `find_cc_toolchain(ctx)` +
  `cc_common.configure_features(...)`; declare the dep via `use_cc_toolchain()`
  and `toolchain = CC_TOOLCHAIN_TYPE` on the action.
- Reconstruct the real compiler invocation with
  `cc_common.create_compile_variables` →
  `cc_common.get_memory_inefficient_command_line` /
  `get_environment_variables` / `get_tool_for_action` (and the
  `create_link_variables` equivalents for a link probe).
- **The key trick:** a failed probe must not fail the build. So the compiler
  runs inside a `ctx.actions.run_shell` wrapper that captures its exit status
  and writes `true`/`false` to a `.result` output file (and the compiler
  output to a `.log`). The rule's *action* always succeeds; the probe's
  *answer* is a file. A source snippet is written with `ctx.actions.write`,
  compiled/linked, and the exit code is the answer.

## Why we are NOT depending on it

The `autoconf/` subsystem is **private to the llvm module**, deliberately:

- Its README scopes it to libstdc++ and says "keep these files free of
  libstdc++ source-policy decisions; source-counterpart files should only
  declare checks through that local API" — i.e. it is an internal API for
  one consumer, not a published general-purpose ruleset.
- It is materialized per-GCC-version and gated on
  `3rd_party/gcc/version.bzl`; its support scope is stated as "Linux with GNU
  libc" only.
- The config-header emitter (`autoconf_config.bzl`) is entangled with
  llvm-internal runtimes machinery (`@llvm//toolchain/runtimes:...`,
  `@with_cfg.bzl`), so it is not liftable in isolation even if we wanted it.

Depending on `@llvm//3rd_party/.../autoconf/...` would reach into another
module's unversioned internals, which could break on any llvm bump and ties
our config generation to that toolchain specifically. The decision (recorded
in the architecture doc) is to **build `cc_config` fresh**, using this
implementation only as a correctness reference for the `cc_common`/toolchain
mechanics above — not copying its files. The probe *execution* mechanics are
general; the parts we write ourselves (the `#cmakedefine`/`@VAR@` config-header
emitter, the shared-target catalog, the translator wiring) are our own needs
and simpler than libstdc++'s.

## How to look at it again

```
bazel fetch @llvm//... >/dev/null 2>&1   # if not already fetched
CACHE=$(bazel info output_base)
ls   "$CACHE/external/llvm+/3rd_party/gcc/libstdcxx/autoconf/"
less "$CACHE/external/llvm+/3rd_party/gcc/libstdcxx/autoconf/cc_configure_probe.bzl"
less "$CACHE/external/llvm+/3rd_party/gcc/libstdcxx/docs/autoconf.README.md"
```
