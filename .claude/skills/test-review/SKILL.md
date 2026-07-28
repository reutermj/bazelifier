---
name: test-review
description: Mechanics for a test-quality pass over the bazelifier repo — a coverage-map script, a recipe for asking CMake what it actually reports, and the repo-specific things that look like test problems but must not be touched. Use whenever the user asks about test quality, coverage, gaps, flakiness, or whether the suite proves what it claims; whenever they ask why a fixture is red or whether a test is worth keeping; and before adding a fixture, an escalation, or a translator capability, since what tier a thing has to be tested at is not obvious from the code.
---

# Test review

**The standard lives in `CLAUDE.md`'s working conventions** — the three-tier
bullet and "green has to be earned," with its corollaries on both
directions, printing the evidence, and checkable claims. Read that first. It
is the spec; this file is only the mechanics.

Fix as you go and report at the end. Stop and ask before anything that
changes *what a tier proves*: deleting a fixture, relaxing the
`needs_attention/` gate, adding an `#[ignore]`, or making a red fixture green
by narrowing its input. Those are not test fixes, and three of them are
explicitly forbidden by conventions elsewhere in `CLAUDE.md`.

## Know the baseline before calling anything a regression

`003-library-no-file-set` and `005-unsupported-target-type` are red in the
unpacked validation workspace today. That is an unfinished pipeline, not a
broken test: both fixtures emit a `needs_attention/` item, the gate fails
loud by design, and the agent stage that resolves them isn't wired up yet
(see `TODO.md`, "Wire up the agent stage of the fixture loop"). Everything
else should be green. Sort failures into "known open" and "new" *before*
touching anything, or you will fix the pipeline's own to-do list.

## Run what actually runs here

```sh
cd translator && cargo test && cargo fmt --check && cargo clippy --all-targets
python3 .claude/skills/test-review/scripts/coverage_map.py
bash -n translator/build_defs/compare_runtime_output.sh   # if touched
```

Everything Bazel-side — `//translator:bazelifier_test`, every fixture
conversion, the validation workspace, `//:buildifier_check` — needs
`rules_rust`, whose archive comes from `github.com` and returns **403**
through the session proxy (tracked in `TODO.md`). So the two tiers that can
contradict you about CMake and about Bazel are the two you most likely
cannot run. Say so in the report rather than implying a clean pass, and
don't work around it by disabling TLS verification.

`cargo` here is the local rustup toolchain. It runs the same test bodies as
`bazel test //translator:bazelifier_test`; the Bazel target is the authority
when they can both run.

## Ask CMake instead of guessing

Half of `cmake_api.rs` is a claim about what the File API reports, and those
claims are cheap to check directly — no build, no Bazel:

```sh
mkdir -p /tmp/probe/build/.cmake/api/v1/query
touch /tmp/probe/build/.cmake/api/v1/query/codemodel-v2   # and/or cache-v2
cmake -G Ninja -B /tmp/probe/build -S <project> > /dev/null
python3 -c "import json,glob;print(json.load(open(glob.glob('/tmp/probe/build/.cmake/api/v1/reply/target-*.json')[0])))"
```

Queries must exist *before* configure or there is no reply at all — the
same ordering `cmake_api::discover` documents. Two traps met while writing
this: `project(p VERSION 1.2.3 CXX)` is an error (`VERSION` forces
`LANGUAGES CXX`), and putting the build directory inside the source tree
changes generated-source paths from absolute to relative, which is enough
to make a probe disagree with the translator for reasons that have nothing
to do with the translator. See
[docs/lore/cmake-file-api-generated-source-shape.md](../../../docs/lore/cmake-file-api-generated-source-shape.md).

## Mutate the line, not the test

The way to find out whether an assertion bites is to break what it guards,
run `cargo test`, confirm *that* test — and ideally only that test — fails,
then restore. Do this for every test you add and any you suspect.

One trap specific to this repo: the escalation strings in
`needs_attention.rs` are line-wrapped Rust literals, so a `sed` pattern
containing a phrase that spans a wrap matches nothing, the mutation never
lands, and the green run reads as "the test doesn't bite" when it means
"you didn't change anything." Confirm the edit applied before believing the
result.

## Run the coverage map

```sh
python3 .claude/skills/test-review/scripts/coverage_map.py
```

Two questions no one can answer by eye: which functions no test module
names, and whether every fixture directory on disk is enrolled in
`translator/tests/BUILD.bazel` (an unenrolled fixture is never converted,
never tested, and nothing reports it).

Candidates, not verdicts. `render_cc_rule`, `render_path_list`,
`render_string_list`, `render_deps`, `rule_name` and the two
`render_*_bazel` halves are all reached through `codegen::render` and are
genuinely covered; `configure`/`build` are one-line `Command` wrappers whose
behavior is CMake's. The list earns its keep on the case nobody decided —
a function that grew logic after its caller's test was written.

To re-validate the script after editing it: delete a fixture from the list
in `translator/tests/BUILD.bazel` and confirm it reports `NOT ENROLLED`,
and rename a tested function and confirm it appears. Read the "Scanned N"
line — an N of 0 means it matched nothing, not that everything is covered.

## What looks like a finding but isn't

This is the part worth having written down; the rest of a test pass is
judgement a careful reader already has.

- **Fixture `CMakeLists.txt` files are immutable input.** A conversion that
  escalates is fixed in the translator or in the *generated* output, never
  by adding the `FILE_SET` that 003 deliberately omits or deleting the
  custom target 005 exists for. Both files carry DO-NOT-EDIT comments for
  exactly this reason.
- **`INTERFACE_LIBRARY` in `target_kind_rejects_types_with_no_bazel_rule_yet`**
  is covered defensively and can't be reached: CMake never puts an interface
  library in the codemodel reply at all (verified, and the test comment says
  so). Don't delete the case for having no fixture, and don't write the
  fixture — it would exercise a different, still-unhandled gap.
- **Assertions on escalation text** look like testing string literals. That
  text is the interface: it ships to an agent that cannot see this repo, and
  `CLAUDE.md`'s `needs_attention/` bullet requires substantive guidance to
  carry a test. Pin the guidance, not the wording — and prefer a phrase that
  would have to be *meant* differently to break.
- **`main.rs`'s two `report_*` tests** look like tests of `format!`. They
  guard `Display` vs `Debug` in the termination path, which silently turns
  every `Display` impl in the crate into dead code — a real regression that
  nothing else in the suite would notice.
- **Overlap between a unit test and a fixture.** `004-binary-private-include`
  exists at both tiers on purpose, and `renders_cc_binary_with_own_includes`
  says so in a comment. Codegen can drop an attribute the frontend resolved
  correctly; only one of the two tiers tells you which half broke.
- **A fixture that a unit test appears to duplicate.** Once captured File
  API JSON is deserialized in a unit test, the fixture that first caught
  the same bug reads as redundant — same construct, same assertion, one of
  them slow and one of them fast. It isn't: the captured reply is frozen
  the day it was captured, so it can only catch us regressing, while the
  fixture is what notices CMake behaving differently from the day we
  looked. Removing the fixture keeps the assertion and throws away the
  only thing that could ever contradict it. See the fourth corollary under
  "Green has to be earned" in `CLAUDE.md`.
- **A library artifact that is never compared.** `expected_targets` lists
  executables only, so a fixture's `.a` reaches `ground_truth/` and is
  exported but never diffed. You cannot run a static library; symbol-table
  comparison is the deferred answer, already recorded under "Equivalence
  checks" in `docs/architecture/build-verification.md`.

## Verify and report

Two things matter more than the length of the finding list. **Say what you
could not run** — with the Bazel tier blocked, that is most of the pipeline,
and a report that reads as a full pass is worse than one that reads as
partial. And **don't inflate**: a pass that turns up three real gaps is a
good pass. Adding tests to look productive is the main way this skill could
do harm, because a test asserting the wrong thing is worse than no test —
it makes the next person confident. Hold a new test to the same bar as a new
comment: it is a claim about behavior, and it will be believed.

## When a finding is worth more than a test

A gap that needs a decision (what a resolution has to prove, whether a tier
should exist at all) belongs in `TODO.md` with what would settle it. A
surprising CMake or Bazel behavior found while probing belongs in
`docs/lore/`. A recurring procedure belongs in `docs/runbooks/`. Keeping
those in their own homes is what stops a test file growing comments that
drift from the assertions under them.
