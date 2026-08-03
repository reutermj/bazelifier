---
name: test-review
description: Mechanics for a test-quality pass over the bazelifier repo — a coverage-map script, a recipe for asking CMake what it actually reports, and the repo-specific things that look like test problems but must not be touched. Use whenever the user asks about test quality, coverage, gaps, flakiness, or whether the suite proves what it claims; whenever they ask why a fixture is red or whether a test is worth keeping; and before adding a fixture, an escalation, or a translator capability, since what tier a thing has to be tested at is not obvious from the code.
---

# Test review

**The standard lives in `CLAUDE.md`'s working conventions** — the three-tier
bullet and "green has to be earned," with its corollaries. Read that first.
It is the spec; this file is only the mechanics.

Fix as you go and report at the end. Stop and ask before anything that
changes *what a tier proves*: deleting a fixture, relaxing the
`needs_attention/` gate, adding an `#[ignore]`, or making a red fixture green
by narrowing its input. Those are not test fixes, and three of them are
explicitly forbidden by conventions elsewhere in `CLAUDE.md`.

## Dispatching this as a subagent

`/review` runs this pass in a subagent: **report-only**, since every edit
listed above needs a question first and a subagent cannot ask one.

**Lane.** What the suite proves and fails to prove. Not comment accuracy,
not duplication.

**The baseline is mandatory, not context.** Give the dispatch the red-fixture
list below verbatim. A cold agent that does not have it reports five
correctly-failing fixtures as regressions, and the report is then worse than
useless because the real findings are buried in noise.

**Ask for mutation evidence, not reasoning.** This pass is uniquely able to
check itself: for a test that claims to pin something, break the thing and
watch it go red. A finding backed by "I made this edit and the suite stayed
green" is worth more than any amount of argument, and it is the only way to
catch a gate wired to nothing. Require it for every P1.

**Severity, for this review type:**

- **P1** — a test that cannot fail, or a gate looking at nothing. Also a
  claim in a comment or bead that the suite is asserted to cover and does
  not.
- **P2** — a real gap at the wrong tier: something unit-tested that only a
  fixture can contradict, or fixture-only where a unit test would be cheap.
- **P3** — a missing negative case, duplicated scaffolding.

**Require coverage and non-verification.** "What I read and found sound", and
"what I could not run" — a pass that skipped the unpacked workspace because
it is slow must say so rather than implying a clean bill.

**Critique this skill when you are done.** Say plainly whether anything here
was wrong, missing, or misleading; whether it led you to what you found or
you found it anyway; and whether its do-not-report list suppressed something
it should not have. That feedback is the mechanism by which this file stops
being wrong — a stale motivating example teaches the next agent to look
backwards, and an over-broad exemption silently costs findings. Report it
alongside the findings, not instead of them.


## Know the baseline before calling anything a regression

**Derive the baseline, don't trust this list.** In an unpacked workspace:

```bash
cd <workspace>/fixtures
for d in */; do n=$(ls "$d/needs_attention"/*.md 2>/dev/null | wc -l)
  [ "$n" -gt 0 ] && echo "$n  ${d%/}"; done | sort -rn
```

No count is written here on purpose. Three previous versions of this file
enumerated the red projects and all three went stale — most recently one
dated the same day it was already wrong. Run the command; a number you
computed cannot be out of date, and a number you read can only be checked by
running it anyway. Each red project emits
a `needs_attention/` item and the gate fails loud by design; these fixtures
exist to test the *whole* pipeline including the agent stage, whose job is to
resolve the item (in the generated output) and turn the fixture green.
Red-with-an-open-item is the expected starting state of that cycle — see
`CLAUDE.md`, "A red fixture is unfinished work, not a terminal state."
Everything else should be green. Sort failures into "known open" (an
escalation-firing fixture with its item still open) and "new" *before*
touching anything, or you will file a bug against the pipeline's own design.
An escalation-firing fixture must still *compile* (fail via the gate, like
these five) rather than fail to build — a build failure aborts the whole
comparison suite and hides every other result; see 015's `main.c`.

## Run what actually runs here

**`bazel test //...` and `bazel query //...` do not work from the repo
root** — name every tier explicitly. There is no `.bazelignore`, so the
wildcard walks `cc_config/`, which is a nested module, and dies loading its
packages:

```
ERROR: error loading package 'cc_config/catalog': cannot load
'//cc_config:config_header.bzl': no such file
```

Two consequences worth knowing before you start. There is no CI here (no
`.github/`, empty `.bazelrc`), so the "run everything" invocation *is* the
workflow and it errors instead of running. And `cc_config`'s own tests —
the Python expander tier, `spliced_test`, the probe assertions — are
reachable only by `cd cc_config && bazel test //cc_config:all`. Nothing at
the root reaches them, which makes them orphaned in practice however sound
they are.

```sh
# Tests ALWAYS run through Bazel — never `cargo test` (see CLAUDE.md: cargo's
# toolchain/resolution differs, so it can pass while the Bazel build is red).
# A reviewer reporting the suite RED from `cargo test` has already been wrong
# once; confirm against Bazel with --nocache_test_results before believing it.
bazel test //translator:bazelifier_test
# Run clippy FIRST and read the WHOLE output as findings, not as lint noise.
# Two detectors, and the second has the better hit rate:
#   `duplicated attribute` — a doc block orphaned from the test it describes.
#   `unused variable` in a test module — a destructured result NOBODY
#     ASSERTED ON. That is a test silently covering less than its name says,
#     and on 2026-08-03 it was the P1 while the duplicated attribute two
#     lines above it was cosmetic.
# Not a test runner.
cd translator && cargo clippy --all-targets && cargo fmt --check
python3 .claude/skills/test-review/scripts/coverage_map.py
bash -n translator/build_defs/compare_runtime_output.sh   # if touched
```

Building in the unpacked workspace needs the catalog supplied by flag —
`--override_module=cc_config=<bazelifier-checkout>/cc_config`. Without it
you get "module cc_config not found in registries", which reads like blocked
egress and is not; `cc_config` is deliberately never staged into the
tarball.

**There is a FOURTH gate, and it is the one with the highest stakes.**
`tools/sweep/sweep.py --post-agent <project>` decides `green: true` and
writes it to a page served publicly. It is a pass criterion like any other
and belongs under review — the three tiers plus buildifier are not the whole
set. Ask of it what you would ask of any gate: what edit would make it pass
when it should fail? The answer was "delete the failing tests", and it is
now **half fixed** — which is worse to read than either state, so both
halves are stated here:

- **Ground-truth comparisons: frozen, and the freeze works.** Verified
  2026-08-03 by deleting a comparison target from an unpacked workspace —
  the sweep reported it GONE, counted it failed, and exited 1. Re-verify
  rather than re-report.
- **The module's own tests: still unfrozen.** They are counted by regex over
  whatever survives in `@module//:all`. Deleting a fixture's only test moved
  `1 passed` → `0 passed, 0 failed` with no failure. The `ran > 0` guard is
  a count, not an expectation, so it fires only when *everything* vanishes.

The second is the bigger surface, because `validation_workspace.bzl` skips
the comparison for any binary that has its own test — libmicrohttpd is 110
binaries, 94 tests, 16 real comparisons, and those 94 rest entirely on the
deletable half.

Run the Bazel tiers directly — `bazel test //translator:bazelifier_test`
(the Rust-unit authority), the fixture conversions, the validation
workspace, `//:buildifier_check`. If a fetch of a ruleset (`rules_rust`,
`llvm`, ...) fails with a proxy **403** in a restricted session, that is an
egress limit, not a test failure: report the tier as not-run rather than
implying a clean pass, and never work around it by disabling TLS
verification. When the network is available (the common case), run every
tier and don't hand-wave the two — CMake and Bazel — that can actually
contradict you.

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

Break what an assertion guards, run the test through Bazel — `bazel test
//translator:bazelifier_test --test_filter=<name>` narrows to one test —
confirm *that* test (ideally only that one) fails, restore. Never reach for
`cargo test` even for the mutation loop; cargo's green is meaningless here
(see CLAUDE.md). Do it for every test you add and any you suspect. One trap
specific to this repo: the escalation strings in
`needs_attention.rs` are line-wrapped Rust literals, so a `sed` pattern
containing a phrase that spans a wrap matches nothing, the mutation never
lands, and the green run reads as "the test doesn't bite" when it means
"you didn't change anything." Confirm the edit applied before believing the
result.

**Generalise that: confirm the mutation reached the CODE PATH, not just the
file.** A textual edit can land perfectly and still change nothing the test
observes. Worked case: breaking splice ordering by sorting inside
`sed_file_splices` came back green and looked exactly like a finding — that
function runs per line, so it sorted a one-element vector every time. The
mutation had to target the accumulation site to bite. When a mutation you
expected to be caught comes back green, suspect your mutation first.

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

To re-validate the script after editing it: delete **an `autotools/`
fixture and a `corpus/` project** from the list in
`translator/tests/BUILD.bazel` and confirm each reports `NOT ENROLLED`, and
rename a tested function and confirm it appears. Read the "Scanned N" and
"Projects on disk" lines — an N of 0 means it matched nothing, not that
everything is covered.

Those two specific victims, not any fixture: the check was blind to all
nine nested projects for months while passing this recipe, because deleting
a top-level CMake fixture exercised the one path that worked. A mutation
that only ever probes the easy case certifies the bug.

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
- **A unit test and a fixture covering the same construct.** Deliberate,
  twice over: `004-binary-private-include` exists at both tiers on purpose
  (`renders_cc_binary_with_own_includes` says so in a comment), and any
  unit test built on a captured File API reply is meant to sit alongside
  the fixture that produced it, never instead of it. Why is the fourth
  corollary under "Green has to be earned" — don't re-derive it, and don't
  collapse the pair.
- **A library artifact that is never compared.** `expected_targets` lists
  executables only, so a fixture's `.a` reaches `ground_truth/` and is
  exported but never diffed. You cannot run a static library; symbol-table
  comparison is the deferred answer, already recorded under "Equivalence
  checks" in `docs/architecture/build-verification.md`.

  **This exemption covers libraries and nothing else — do not let it absorb
  the larger exclusion beside it.** `validation_workspace.bzl` also skips any
  binary that has its own registered test, on the sound reasoning that the
  test is the better check. But the two together mean a project can be far
  less compared than its target count suggests: libmicrohttpd compares 16 of
  110 executables, and the other 94 rest entirely on tests that the agent
  stage is free to delete. Reading "16 of 110" as this bullet's library
  carve-out and moving on is the wrong call — check which exclusion applies
  and what is left holding the line.

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
should exist at all) belongs in a **beads issue** (`bd create`) with what
would settle it — this repo tracks work in beads, not a `TODO.md`. A
surprising CMake or Bazel behavior found while probing belongs in
`docs/lore/`. A recurring procedure belongs in `docs/runbooks/`. Keeping
those in their own homes is what stops a test file growing comments that
drift from the assertions under them.
