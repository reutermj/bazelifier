---
name: onboard-project
description: Drives a real open-source project to building and testing end-to-end in Bazel through the full pipeline — selecting a candidate by measurement, pinning and converting it, FIXING the translator bugs it surfaces rather than reporting them, RUNNING THE AGENT STAGE to resolve every escalation, and proving the result against a seven-point definition of success. Commits at each milestone so any step can be rolled back. Use whenever the user wants to onboard, add, or evaluate a project for the corpus. This is a goal, not a checklist: it is unfinished until the module is green from the unpacked workspace and the metrics record it.
---

# Onboarding a corpus project

**This is a GOAL, not a checklist.** The goal is: *the project builds and
tests end-to-end in Bazel, from the unpacked validation workspace, after the
full pipeline — INCLUDING the agent stage that resolves its escalations.*
Every section below is a means to that end. Onboarding is not finished when
the project converts, not finished when the translator bugs are fixed, and
not finished when you have described what is wrong. It is finished when the
escalations are resolved, the module is green, and the metrics say so.

**Fix the translator; do not route around it.** Most of what onboarding finds
is a translator bug, and fixing it is the work rather than a distraction from
it. The corpus exists to surface exactly these. Two things are forbidden
outright: editing the project's own build files (CLAUDE.md — they are
immutable input), and hand-patching generated output to paper over a
translator defect that will hit the next project identically.

**Bail early only to confirm the DESIGN of a fix**, never to hand back a
diagnosis. Stopping is warranted when the fix is genuinely ambiguous in a way
that costs real rework if guessed wrong:

- Two defensible designs with different blast radius (a new model field vs.
  widening an existing one; deterministic rule vs. escalation).
- The fix would change an interface other projects depend on — an escalation
  `kind`, a `model` field's meaning, the catalog schema.
- The evidence genuinely underdetermines it: the input does NOT state the
  answer, so making it deterministic would be the guessing CLAUDE.md forbids.

Everything else — a missing rebase, an unpopulated field, a predicate that
does not know an extension — you fix, test at the lowest tier that can fail
on it, and keep going. "I found N bugs, tell me which to fix" is not a
deliverable. Fix them, then say what you fixed and what you deliberately
left.

Fixtures and corpus projects answer different questions. A fixture is written
to exercise one construct and can be made to say anything. A corpus project
cannot be edited at all, so it is the only tier that can contradict us.
Onboard for the contradiction.

## Commit at every milestone

**Commit each intermediate success as it happens**, rather than landing the
onboarding as one change at the end. Onboarding is exploratory — a fix design
gets reverted, a candidate turns out to be wrong three steps in — and the
value of a commit here is the rollback point, not the history.

The milestones below are each independently good: at every one the tree is
green for what it claims, so `git revert` of a later one leaves a working
repo. Commit when you reach one, even if the next is minutes away.

| # | Commit when | Contains |
|---|---|---|
| 1 | The project is pinned and converts | `MODULE.bazel` entry, `corpus/<p>/BUILD.bazel`, enrollment |
| 2 | Each translator fix, separately | the fix, its test, its fixture |
| 3 | The module builds in the unpacked workspace | whatever closed the last build failure |
| 4 | The agent stage has resolved the escalations | the resolutions, in the GENERATED output |
| 5 | Green, measured | metrics row, regenerated page, corpus comment |

**One fix per commit at milestone 2.** Onboarding tends to surface several
unrelated bugs, and bundling them is what makes a bisect useless later. Each
should name the project that surfaced it — that is how the next person
learns which corpus project is load-bearing for a capability.

Do not commit a red tree without saying so in the message. A commit whose
message claims more than the tree delivers is worse than no commit.

## 1. Select by measurement, not reputation

Every candidate gets measured before any of them gets pinned. The measurements
below each rejected a real candidate; run them on 4-6 projects and compare.

```sh
# In a scratch dir, per candidate tarball:
tar xzf $t.tar.gz
test -f $t/Makefile.am && echo automake || echo "NO Makefile.am"
ls $t/lib/*.in.h 2>/dev/null | wc -l          # gnulib replacement headers
find $t -name '*.c' -not -path '*/lib/*' | wc -l   # its own code
test -f $t/configure && echo "configure shipped" || echo "needs autoreconf"
```

**Disqualifiers, each learned the hard way:**

- **gnulib replacement system headers** (`lib/*.in.h`, usually 25-38 of them).
  gnulib GENERATES replacements for `stdio.h`, `string.h` and friends that
  shadow the real ones. Converting that faithfully means building a
  gnulib-shaped feature no other project wants. This rejected GNU hello — the
  canonical autotools project — and then gzip, patch, nano and libunistring.
  It is the single highest-yield check.
- **No `Makefile.am`.** A hand-written `Makefile.in` has no automake primaries
  for `make -p` to report, so the frontend's second source of truth is empty.
  Rejected GNU units, which looks like a normal autotools project until you
  look.
- **Vendored code outnumbering its own.** The ratio is the point: hello is 170
  lines of its own around 72 vendored files. You would be converting the
  vendor.
- **Too big to read.** xz's config header escalates 143 macros in one item. It
  is a legitimate corpus project but a bad one to develop against, because you
  cannot tell a good item from a bad one at that size. Prefer a project whose
  first escalation fits on a screen.

**What you WANT is a shape the corpus lacks**, not a project that will convert
cleanly. Write down, before starting, what this project brings that no
existing one does — recursive make across N directories, a C++ target, a
versioned shared library. If you cannot name it, onboarding it proves little.

Verify the candidate configures and builds on this machine before pinning it.
A project that does not build natively cannot produce ground truth.

## 2. Pin it

`MODULE.bazel`, as INPUT to the translator only — never a `bazel_dep`.

```python
http_archive(
    name = "<project>",
    build_file_content = """filegroup(
    name = "all_srcs",
    srcs = glob(["**"]),
    visibility = ["//visibility:public"],
)
""",
    integrity = "sha256-...",   # openssl dgst -sha256 -binary f | openssl base64 -A
    strip_prefix = "<project>-<version>",
    url = "...",
)
```

**Autotools projects take a release TARBALL (`http_archive`); CMake projects
take `git_repository`.** Not a style preference — a tarball ships `configure`
pre-generated, and a git checkout would need `autoreconf -i`, making the
conversion depend on the host's autotools. Both Autotools corpus projects are
tarballs and all four CMake ones are checkouts; follow that.

**The selection rationale goes in this comment**, including the candidates you
rejected and the number that rejected them. It is the only place the next
person can re-derive the choice, and the xz entry proved its worth by making
expat's selection a ten-minute job instead of a rediscovery.

## 3. Wire it

`translator/tests/corpus/<project>/BUILD.bazel` calls
`convert_autotools_project` or `convert_cmake_project` with
`srcs = ["@<project>//:all_srcs"]` and
`visibility = ["//translator/tests:__pkg__"]`. Leave `source_dir` empty so the
rule derives it.

Then **enroll it** in `translator/tests/BUILD.bazel`. A project that is not in
that list is converted by nothing and reported by nothing — and until
recently the tool that checks enrollment was itself blind to `corpus/`. Verify:

```sh
python3 .claude/skills/test-review/scripts/coverage_map.py .   # expect N/N complete
```

## 4. Convert, and read what comes back

```sh
bazel build //translator/tests/corpus/<project>:converted
```

Read `CONVERSION.json`, `TARGETS`, and every file in `needs_attention/`. Then
sort each escalation into one of two piles, because they have different
owners — and then **work both piles**, rather than reporting the split:

- **Agent work** — the translator was right to escalate. A project value only
  a human can decide (`XML_DTD`, `Z_PREFIX`), a test whose command is not a
  target we build.
- **A translator gap** — the escalation should not exist, or the thing was
  dropped silently. The test is CLAUDE.md's: *does the input state it?* Not
  "is the answer usually the same," but "is the fact present in what we
  read?"

Expat produced one of each within ten minutes:

- 3 of its 19 escalated macros were `const`, `off_t`, `size_t` — autoconf's
  compiler-workaround idiom, which must stay undefined. The template states
  this (lowercase name, `#undef`, "Define to empty if..."), so escalating them
  asks the agent to resolve something with no right answer.
- `lib/xmltok.c` textually `#include`s `xmltok_impl.c`, which was never copied
  into the module. `_SOURCES` declares five `.c` files and only three compile
  — the difference IS the textual-include set, stated plainly by the input.

**A silent drop is worse than a noisy escalation** and will not appear in
`needs_attention/` by definition. Check `CONVERSION.json` for a confident zero
(`"tests": 0` on a project with `check_PROGRAMS`) and diff the declared
sources against the compiled ones.

### The fix loop

For each translator gap, in this order — it is the tier discipline from
CLAUDE.md applied to onboarding:

1. **Write the failing test first, at the lowest tier that can fail on it.**
   A unit test if the decision is expressible over inputs we write; a fixture
   if it needs a real build to contradict us. Watch it go red before fixing.
2. **Fix the translator**, never the project and never the generated output.
3. **Re-run the unit suite through Bazel** (`bazel test
   //translator:bazelifier_test` — never `cargo test`).
4. **Re-convert and confirm the symptom is gone** in the real output, not
   just in the test.
5. **Re-run the plain sweep.** A fix that helps this project can move another
   one, and that is invisible until someone converts it by hand.

A capability is not finished until a FIXTURE exercises it. Unit tests
agreeing with each other only proves self-consistency — and a corpus project
is not a substitute, because the fixture is what keeps the capability pinned
when the corpus project is later removed or changes version.

## 5. Prove independence

Never validate in place — a fixture building inside this repo may just be
inheriting bazelifier's own toolchains.

```sh
bazel build //translator/tests:validation_workspace
mkdir -p /tmp/ws && tar xf bazel-bin/translator/tests/validation_workspace.tar -C /tmp/ws
cd /tmp/ws && bazel build @<project>//:all \
    --override_module=cc_config=<bazelifier-checkout>/cc_config
```

The `--override_module` is required and is not a workaround: `cc_config` is
deliberately never staged into the tarball. Without it you get "module
cc_config not found in registries", which reads like blocked egress.

**Red here is the expected first state**, not a failure — an open escalation
means unfinished agent work. What matters is that you can name WHY each
failure happens and which pile it belongs to. "It doesn't build" is not a
diagnosis.

## 5b. Run the agent stage — it is part of onboarding, not after it

Every remaining escalation now gets resolved. This is the pipeline's second
stage and the thing onboarding is ultimately testing: an escalation no agent
can act on is worse than one that never fired, and you only learn which kind
you emitted by trying to close it.

```sh
/resolve <project>
```

That command owns the mechanics — follow
`.claude/skills/resolve-escalations/SKILL.md`, which it points at. Do not
restate the procedure here. Two things matter from the onboarding side:

- **Resolutions land in the GENERATED output**, never in the project's own
  build files and never by narrowing what the fixture tests.
- **Resolving is also a review of the escalation.** If the item was hard to
  act on, said something false for this project, or asked for a decision the
  translator could have made from the input, that is a finding — file it.
  The escalation did its job by making the gap visible; leaving the text
  wrong means the next project inherits it.

Resolutions are deliberately ephemeral (bzl-b9b): re-converting discards
them, and that is the design rather than a gap. So commit the resolved state
as its own milestone, and expect to re-run this stage after any later
translator change.

If a resolution turns out to be impossible from inside the module, that is
the strongest possible signal about the escalation — the item is asking for
something the agent cannot supply. Fix the translator or the escalation
text; do not reach outside the module to get green, with the single
documented exception of the `cc_config` catalog branch.

## 6. Definition of success

Converting is the halfway point. Onboarding succeeds when **all** of these
hold — each is a command with an answer, not a judgement:

| # | Criterion | How it is checked |
|---|---|---|
| 1 | The module builds from the unpacked workspace | `bazel build @<p>//:all` there, exit 0 |
| 2 | Its tests pass there | `bazel test @<p>//:all` — and if the project HAS tests, `"tests": 0` fails this |
| 3 | Runtime output matches ground truth | the generated `*_matches_ground_truth` targets pass |
| 4 | No `needs_attention/` item is still open | `needs_attention/MANIFEST` empty after the agent stage |
| 5 | Every translator gap found is fixed and pinned | a test at the lowest failing tier, plus a fixture for any new capability |
| 6 | The rest of the corpus did not move | plain sweep before/after, target and escalation counts explained |
| 7 | The result is recorded | post-agent row written, page regenerated, both in the same commit |

```sh
python3 tools/sweep/sweep.py                        # 6 — did anything else move?
python3 tools/sweep/sweep.py --post-agent <project> # 1-4, 7 — writes the row
```

`--post-agent` records `green`, `resolved`, `open_items`,
`comparisons_passed/failed` and `module_tests_passed/failed` keyed by
(commit, project). **`green: true` is the criterion**; the rest say why when
it is false.

### Also record what the onboarding COST

The post-agent row says whether the project ended green. It does not say what
getting there took, and that is the number that tells you whether a project
was worth onboarding. Record alongside it, in the epic and in the corpus
`BUILD.bazel` comment:

- **translator bugs found, and how many were fixed** — the real yield. A
  project that converts cleanly on the first try taught nothing; one that
  surfaced three genuine gaps paid for itself.
- **which were deterministic fixes vs. genuine escalations** — the ratio
  tracks whether stage one is getting smarter over time. Expat: 2 translator
  bugs, 16 genuine escalations.
- **escalation count at first conversion vs. at green** — how much the
  translator absorbed versus handed to the agent.
- **whether any fix moved another project**, from the before/after sweep.

These are per-onboarding facts, not per-commit ones, which is why they belong
with the epic rather than in `history.jsonl`. If a later onboarding wants
them trended, that is a schema change to argue for on its own evidence — do
not quietly add a column to a file whose rows are documented as a property of
the commit.

Record the end state in the corpus `BUILD.bazel` comment: what converted,
what is still red, and which pile each red thing is in. A red project with a
diagnosis is work in progress; a red project without one is rot.

## What is frontend-specific here

Most of this is neutral. These are not:

- The gnulib and `Makefile.am` disqualifiers are Autotools-only. The CMake
  equivalent is checking what the File API actually reports — a project whose
  targets are all `INTERFACE` or `UTILITY` has nothing to convert.
- Tarball-vs-checkout is Autotools-only, for the `configure` reason above.
- `check_PROGRAMS` not being built by plain `make` is Autotools-only, and is
  why `"tests": 0` needs checking there specifically.

## Re-validating this skill

Delete a corpus project from `translator/tests/BUILD.bazel` and confirm
`coverage_map.py` reports it `NOT ENROLLED`; that check was blind to every
`corpus/` project until 2026-08-02, so do not assume it works. Then confirm
the unpacked-workspace command above still needs `--override_module` — if it
stops needing it, `cc_config` got staged into the tarball and something
larger changed.

## Critique this skill when you are done

Say plainly whether the selection measurements actually discriminated, whether
the two-pile split was obvious or ambiguous in practice, and whether anything
here sent you looking for a problem that no longer exists. A stale example
teaches the next agent to look backwards.
