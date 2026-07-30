# TODO

Open items not yet tracked elsewhere. Keep entries actionable: what's
unknown, why it matters, and what would settle it.

## `read_project_version` and `find_reply_file` are still untested

**Status:** open, narrowed 2026-07-30.

Fixture coverage for the two previously-untested escalations and a
project `VERSION` is done: `007-generated-source` exercises
`generated_sources_needs_attention` (red-until-the-agent-stage, like
`003`/`005` — see
[build-verification.md#fixtures](docs/architecture/build-verification.md#fixtures)),
`008-sources-outside-deliverable-root` exercises
`sources_outside_deliverable_needs_attention` (the mirror of `006`'s
non-escalating case), and `001-hello-world`'s `project()` now declares
`VERSION`. `to_target` also now filters the phantom `<output>.rule`
sibling CMake reports alongside a real generated source (see
[docs/lore/cmake-file-api-generated-source-shape.md](docs/lore/cmake-file-api-generated-source-shape.md)),
verified by deliberately breaking the filter and confirming the new test
catches it.

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

**How to settle it:** add a `cache-v2-*.json` (real capture from
`001-hello-world`, whose `project()` now declares `VERSION`) and call
`read_project_version` directly, plus a `find_reply_file` test covering
its multiple-prefix-candidates and not-found cases.

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
