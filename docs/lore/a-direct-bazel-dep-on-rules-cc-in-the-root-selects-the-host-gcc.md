# A direct `bazel_dep` on rules_cc in the root selects the host gcc

Adding `bazel_dep(name = "rules_cc", ...)` to the validation workspace's
root `MODULE.bazel` made every fixture compile with `/usr/bin/gcc` instead
of the llvm toolchain. Nothing failed at resolution or analysis; the only
tell was `gcc failed: error executing CppCompile command` in the output of a
fixture that was expected to fail anyway, and `bazel aquery` showing
`exec /usr/bin/gcc` where `external/llvm+.../bin/clang++` had been.

Why: Bazel orders registered toolchains by the depth of the module that
registered them — the root's registrations first, then its direct
dependencies' in declaration order, then theirs. rules_cc's own
`MODULE.bazel` registers the auto-configured host toolchain
(`@local_config_cc_toolchains//:all`). Every generated module registers
`@llvm//toolchain:all`. With rules_cc only a transitive dependency (through
the fixtures), the fixtures' llvm registration came first and won. With
rules_cc promoted to a direct dependency of the root, its host-toolchain
registration came first instead, and the hermeticity claim in CLAUDE.md was
silently false for the whole workspace.

It happened on 2026-09-10 while adding a `git_override` for rules_cc to the
root (bzl-ti9). The override was assumed to need a matching `bazel_dep`; it
does not — an override applies to the named module wherever it sits in the
graph, and the workspace resolved rules_cc from the override with the
`bazel_dep` line removed. Measured on the same unpacked tree: gcc with the
line, clang without it.

What keeps it from recurring:

- `validation_workspace.bzl` emits the override and no `bazel_dep`, and
  says why beside it.
- `check_root_module_rules_cc_override.sh` fails if the root gains a direct
  `bazel_dep` on rules_cc.

The general lesson is wider than rules_cc: any direct dependency added to
the validation root that registers a cc toolchain outranks the fixtures'
llvm one. The compiler a fixture is built with is a claim worth checking
with `aquery` after touching the root module, since nothing else reports it.
