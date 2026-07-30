# TODO

Open items not yet tracked elsewhere. Keep entries actionable: what's
unknown, why it matters, and what would settle it.

## Decide pass criteria for header-visibility resolutions

**Status:** open, deliberately deferred.

Because the build goes green whether or not `hdrs` is populated correctly,
"empty `needs_attention/` + runtime equivalence" cannot distinguish an
agent that actually resolved the item from one that deleted the markdown
file. Both pass. Neither toolchain in use enforces the split at compile
time either — see
[docs/lore/bazel-does-not-enforce-hdrs-vs-srcs.md](docs/lore/bazel-does-not-enforce-hdrs-vs-srcs.md)
— so there's no shortcut where the compiler does this work for us; an
explicit decision is required.

Options considered: a structural assertion on the generated `BUILD.bazel`
(e.g. an `expected_hdrs` attr on `convert_cmake_project`), a full
generated-output golden diff, or leaving the two existing gates as-is.

Note this interacts with the decision that resolutions are **ephemeral**
(made in the unpacked workspace, not checked in): the *resolution* isn't
persisted, but any *expectation* used to check it would have to be. Those
aren't in conflict — expectation ≠ resolution — but it's the wrinkle to
think through.

## Derive `module_name`/`expected_targets` instead of declaring them

**Status:** open. Previously blocked on network egress preventing
`bazel build`; that's resolved as of 2026-07-30, so this is now
actionable.

Every fixture's `BUILD.bazel` hand-declares `module_name` and
`expected_targets`, duplicating facts the translator already computed.
`convert_cmake_project.bzl` explains why: the validation workspace's root
`MODULE.bazel`/`BUILD.bazel` are generated at **analysis** time, and the
converted module doesn't exist until **execution** time.

That reasoning holds for `ctx.actions.write`, but not for the packaging as
a whole — `_validation_tree` already runs a shell action over the fixture
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

## No fixture exercises two of the four escalations, or a project version

**Status:** open.

Fixtures cover header visibility (`003`) and unsupported target types
(`005`). Neither `generated_sources_needs_attention` nor
`sources_outside_deliverable_needs_attention` has one — `006` covers the
*non*-escalating sibling-source case, which is the opposite branch. No
fixture declares `project(... VERSION ...)`, so `read_project_version` and
the `MODULE.bazel` version line have never run against real CMake output
either.

**Why it matters:** the fixture tier is the only one that can contradict
`cmake_api.rs`'s serde structs about the File API. A wrong
`#[serde(rename)]` deserializes to a default in silence: rename
`isGenerated` and generated sources stop being detected, with `srcs`
quietly gaining an absolute path into a build directory. The unit tests
cannot catch it — they construct `TargetReply` in Rust, so they only prove
the code agrees with itself.

**How to settle it:** add a fixture with an `add_custom_command()`-produced
source, one that references a file outside its `deliverable_root`, and a
`VERSION` on some existing fixture's `project()`. The first two are
red-until-the-agent-stage, like `003`/`005`. Expect the generated-source
escalation to list a phantom `<output>.rule` entry — see
[docs/lore/cmake-file-api-generated-source-shape.md](docs/lore/cmake-file-api-generated-source-shape.md),
which probably wants filtering out before a fixture makes an agent read it.

## Nothing tests the code that reads a File API reply

**Status:** open.

`read_codemodel_reply` is where the tested pieces are wired together: it
builds `translated_names` and `dependents_of`, decides `is_depended_on`,
filters dropped edges down to *translated* dependents, and raises
`SourceDirOutsideDeliverableRoot`. Every unit below it is tested; the
wiring is not, and neither are `read_project_version` or `find_reply_file`.
`.claude/skills/test-review/scripts/coverage_map.py` reports the full list.

**Why it matters:** `to_target`'s tests take `is_depended_on` as a
parameter, so the computation that decides it is covered nowhere. Today
that logic is only exercised by the fixture tier, which was blocked on
network egress (resolved 2026-07-30) and is runnable again.

**How to settle it:** `read_codemodel_reply` takes a reply directory path —
the seam is already there. A test can write captured File API JSON into a
temp directory and call it, which also pins the serde schema against real
CMake output rather than against our own constructors. Needs either a
`tempfile` dev-dependency (and a `Cargo.lock` regen, see runbook 001) or a
hand-rolled temp directory.

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
