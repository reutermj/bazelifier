# TODO

Open items not yet tracked elsewhere. Keep entries actionable: what's
unknown, why it matters, and what would settle it.

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

## `read_project_version` and `find_reply_file` are still untested

**Status:** open, narrowed 2026-07-30.

`read_codemodel_reply`'s own wiring — `translated_names`, `dependents_of`,
`is_depended_on`, dropped-edge filtering, `SourceDirOutsideDeliverableRoot`
— is now covered: `cmake_api::tests::read_codemodel_reply_wires_real_capture_into_a_build_graph`
and `..._rejects_source_dir_outside_deliverable_root` call it directly
against real CMake File API JSON captured from `002-with-library`
(two targets, a real dependency edge, a real `FILE_SET PUBLIC HEADERS`),
written into a hand-rolled scratch directory (`ScratchDir`, no new
dependency). Verified the capture actually pins the schema, not just the
translator's own idea of it: temporarily corrupting the `fileSets` rename
made the test fail exactly as expected — `greet.hpp` silently stopped
being recognized as file-set-declared and got escalated instead of
classified as public.

`read_project_version` and `find_reply_file` are not exercised by that
capture (no `cache-v2-*.json` was included, and neither function is called
directly) — `.claude/skills/test-review/scripts/coverage_map.py` still
lists both as uncovered.

**How to settle it:** add a `cache-v2-*.json` (real capture, e.g. from a
fixture with `project(... VERSION ...)` once one exists — see the fixture
item above) and call `read_project_version` directly, plus a
`find_reply_file` test covering its multiple-prefix-candidates and
not-found cases.

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
