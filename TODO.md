# TODO

Open items not yet tracked elsewhere. Keep entries actionable: what's
unknown, why it matters, and what would settle it.

## Validate `layering_check` under the hermetic `llvm` toolchain

**Status:** open, blocked on network egress.

Bazel does not enforce the `hdrs`/`srcs` split by default — a header in a
dependency's `srcs` is still propagated as an input to dependents' compile
actions, so a consumer can `#include` it and the build goes green. This was
verified directly; see
[docs/lore/bazel-does-not-enforce-hdrs-vs-srcs.md](docs/lore/bazel-does-not-enforce-hdrs-vs-srcs.md)
for the experiment and results.

**The gap:** that experiment ran against the **autodetected host toolchain**,
which resolved to `gcc`. Fixtures actually build under the hermetic **`llvm`**
toolchain, which is clang-based and could plausibly enable `layering_check`
(the feature that *does* enforce the split, via module maps).

**Why it matters:** if `llvm` enables `layering_check`, then
`003-library-no-file-set` fails to *compile* rather than building with
degraded encapsulation. That's a materially different failure mode and it
changes the stated rationale for the `needs_attention/` gate in
[docs/architecture/build-verification.md](docs/architecture/build-verification.md#header-visibility-is-not-enforced-by-default).

**How to settle it:**

1. Reproduce the three-case matrix from the lore doc, but in a workspace
   whose `MODULE.bazel` depends on `llvm` and registers
   `@llvm//toolchain:all` (i.e. what a generated module looks like).
2. Or read the fetched ruleset directly —
   `~/.cache/bazel/_bazel_*/*/external/llvm+/**` — for whether
   `layering_check` is in the toolchain's enabled feature set. (Per
   CLAUDE.md: read the actual `.bzl` source, don't trust a README.)
3. Update the lore doc's "Open" section and the build-verification section
   with the answer either way.

**Blocked by:** `github.com` archive downloads return **403** through the
session's egress proxy, so `rules_rs` can't fetch `rules_rust` and the
translator can't be built. `bcr.bazel.build` is reachable, so BCR-only
dependency graphs do work. Needs `github.com` allowlisted in the
environment's network policy.

## Decide pass criteria for header-visibility resolutions

**Status:** open, deliberately deferred.

Because the build goes green whether or not `hdrs` is populated correctly,
"empty `needs_attention/` + runtime equivalence" cannot distinguish an
agent that actually resolved the item from one that deleted the markdown
file. Both pass.

Options considered: a structural assertion on the generated `BUILD.bazel`
(e.g. an `expected_hdrs` attr on `convert_cmake_project`), a full
generated-output golden diff, or leaving the two existing gates as-is.

Note this interacts with the decision that resolutions are **ephemeral**
(made in the unpacked workspace, not checked in): the *resolution* isn't
persisted, but any *expectation* used to check it would have to be. Those
aren't in conflict — expectation ≠ resolution — but it's the wrinkle to
think through.

Partly contingent on the `layering_check` item above: if `llvm` enforces
the split, the compiler itself forces a correct resolution and no extra
assertion is needed.

## Derive `module_name`/`expected_targets` instead of declaring them

**Status:** open, blocked on the same missing `bazel build`.

Every fixture's `BUILD.bazel` hand-declares `module_name` and
`expected_targets`, duplicating facts the translator already computed.
`convert_cmake_project.bzl` explains why: the validation workspace's root
`MODULE.bazel`/`BUILD.bazel` are generated at **analysis** time, and the
converted module doesn't exist until **execution** time.

That reasoning holds for `ctx.actions.write`, but not for the packaging as
a whole — `_combined_mtree` already runs a shell action over the fixture
tree artifacts. The same trick applies to the root files: generate them in
an execution-time action that reads each fixture's actual
`MODULE.bazel` (for the module name) and `ground_truth/` (for the target
list). Both attrs then disappear, along with `_root_build_bazel`.

**Why it matters:** both attrs can drift from what the translator really
emitted, and both fail badly when they do. A stale `module_name` breaks
`local_path_override` with a bzlmod error pointing nowhere near the
fixture; a fixture that gains a target simply never gets a comparison
test, silently — which is the failure mode this whole pipeline exists to
catch.

**Note:** the tempting alternative — having the translator emit the
comparison `sh_test` into the module's own `ground_truth/BUILD.bazel` —
should be rejected. It would put `bazel_dep(name = "rules_shell")` into
every converted module's `MODULE.bazel`, which is user-facing output, for
a validation-only reason.

## Wire up the agent stage of the fixture loop

**Status:** open, design settled, mechanics not.

Settled: the loop is convert → agent triages `needs_attention/` → rebuild,
iterating until green. Resolutions are made in the unpacked validation
workspace and are ephemeral. A clean checkout requires an agent to reach
green — the pipeline is intentionally non-hermetic.

Still needed:

- The agent is invoked as a **skill**; its invocation contract (inputs,
  outputs, how the driver calls it) needs to be pinned down before the
  runner can be written.
- Iteration bound before the loop declares failure.
