---
name: beads-review
description: Mechanics for auditing the beads issue tracker against the code — finding issues already fixed, whose stated diagnosis has been overtaken, or that were decided and superseded. Use whenever the user asks whether the backlog is accurate, before planning from `bd ready`, and after a run of work that fixed things without referencing bead IDs. Not `bd stale`, which measures activity; this measures whether an issue still describes reality.
---

# Beads review

**`bd stale` does not do this.** It reports issues with no recent activity
and currently returns zero — while a hand audit the same day found six beads
that no longer described reality. Activity and accuracy are different
questions, and only one of them is tooled.

**The output is usually not a closure.** The description frames this as
finding issues already fixed, and that undersells it: on 2026-08-03 the four
highest-value findings were beads that should all stay OPEN and were all
actively misdirecting — a retracted plan still in the acceptance criteria, a
count wrong by 7×, a diagnosis half-solved by a commit that didn't name it.
"Still open, wrong description" is the characteristic result. Report those
with the same weight as a closure.

## Why it goes stale, from the real cases seen so far

Each is a distinct shape, and only the first is obvious:

1. **Fixed by a commit that never named it.** The work happened; the bead
   was never closed. Most common.
2. **Diagnosis overtaken.** `bzl-dc9` said three predicates had diverged on
   case. True when filed — and already fixed in one of the three modules,
   while a *fourth* copy had appeared elsewhere. Closing it as "done" and
   reopening it as filed would both have been wrong.
3. **Solved a different way than proposed.** `bzl-yjn.9` asked to speed up
   `make -n -B`; the fix deleted the dry run entirely. The bead's acceptance
   criterion never happened and the problem is gone.
4. **Decided, then superseded.** `bzl-yjn.1` chose `make -n` as the
   discovery source. The decision was correct and later replaced.
5. **Blocker cleared, blocked bead not revisited.** Fixture 001 was
   unenrolled because of libtool wrappers; that was fixed and the fixture
   stayed out — for a *different* reason nobody had filed.
6. **Acceptance test now passes** and nobody re-ran it.
7. **The world moved under a diagnosis that was right when written.** Not
   shape 2 — the diagnosis was never wrong *about its own world*. `bzl-i4i.7`
   said staging fires for four projects and the corpus contained exactly one
   angled project-local include; both were true, and both are now false
   because three projects landed since. `bzl-fxa.21` said "a second frontend
   would want this"; that frontend arrived and now imports the leak.

   These are the most dangerous beads in the tracker because they read as
   *unusually rigorous* — they cite counts and measurements, which is exactly
   what makes a reader trust them and skip re-measuring. **Any bead
   containing a number, a count, or the word "measured" is a re-measurement
   candidate**, and the more careful it looks the more it needs one.

   **Re-measuring is not the remedy, though — it is the diagnosis.** A
   corrected count goes stale exactly as fast as the original and carries a
   false patina of freshness. `bzl-i4i.7` was corrected 4→7 on 2026-08-02
   and was wrong again at 12 the next day; `bzl-oek` went 14→102,
   `bzl-7s2` 9→43, `bzl-k1z` 17→29. When you re-measure, **replace the
   integer with the command that produces it** —
   `grep -l _staged_hdrs …/fixtures/*/BUILD.bazel | wc -l` — so the next
   reader re-runs it in two seconds instead of trusting a frozen number.
   Same move CLAUDE.md already makes for comments; nobody had applied it to
   beads.

   And check whether the count still supports the bead's *severity*.
   `bzl-oek`'s P3 rests on "the cost is 14 spurious names"; at 102 the
   argument no longer follows from its own evidence, which is a finding in
   its own right.
8. **A close reason asserting a measurement that was never taken.** Shapes
   1-7 are about *open* beads drifting. This one is about a **closed** bead
   whose stated justification is checkable and false — and it is strictly
   more dangerous, because a close reason is what stops anyone re-examining
   it and nothing will ever re-surface it.

   `bzl-oek` was closed with "the fixes resolved every `$(am__EXEEXT_N)` to
   real names." Twenty literal entries ship. The closing agent had written
   the fix minutes earlier and the least incentive of anyone to re-run the
   check — and the check it did run globbed a filename containing a count
   that the fix had changed, so it matched nothing and returned zero.

   **Audit close reasons that assert an output state.** "X is now Y" is a
   claim about the pipeline, not a summary of intent. `bzl-b6m` is the
   control: same shape, and its numbers held when re-measured.
9. **The retraction that didn't reach the acceptance criteria.** A bead has
   three fields that can disagree, not two. When NOTES contradict
   DESCRIPTION, **read ACCEPTANCE CRITERIA third** — it is rendered last,
   revised least, and is the field an implementer treats as the contract.

   `bzl-yjn.10` proposes reading `config.status`'s `D[]` table for the
   `GNULIB_*` group; its own notes then measure that group at zero in four
   of five projects and say "scratch that framing" — while the acceptance
   criteria still read "parsed and used for the `GNULIB_*` group only". Not
   a stale half beside a fresh half: a live, self-refuted *instruction*, and
   acting on it encodes one project as universal. Distinct from the
   DESCRIPTION-vs-NOTES case because the retraction landed and still missed
   the field that matters.

   Same shape resolves benignly when the work is done — `bzl-07v`'s notes
   void its recommendation and the capability shipped anyway. Check which
   way it resolves before recommending anything.

## The method

Do not read beads and reason about them. **Check each against the code**, in
this order — cheapest disqualifier first:

```sh
bd list --status=open
bd list --status=in_progress   # NOT optional — see below
```

**`--status=open` alone misses a whole class.** An `in_progress` bead whose
work has actually completed is invisible to it; three such beads were found
in one pass, all closeable. Claimed-and-finished is at least as common here
as never-started.

**Read DESCRIPTION and NOTES as separate artifacts that can disagree.**
`bd show` renders the description first, so when someone appends an accurate
correction on top of a stale body, the stale half is what a reader hits.
`bzl-b9b`'s notes record a deliberate reframe while its description still
cites a file deleted three commits earlier; `bzl-yjn`'s notes literally open
"CORRECTION to this epic's description". A bead can be simultaneously
well-maintained and misleading.

**Several beads make claims about generated OUTPUT, not source.** Grepping
`translator/src/` confirms a trigger condition while missing that the
project count is wrong. Build and unpack the validation workspace, or use
one already unpacked, and check those against it.

**Building there needs `--override_module=cc_config=<checkout>/cc_config`.**
Without it Bazel fails at repo-mapping with "module cc_config@0.0.0 not
found in registries", which reads like a broken tarball or blocked egress
and is neither: `cc_config` is deliberately never staged into it. A 2026-08-03
reviewer hit this, concluded the independence tier was broken, and lost half
its planned checks. If a build fails *before analysis*, suspect the flag
before the pipeline.

For each: grep the identifiers, files and symptoms the bead names. Then:

- Does the thing it describes still exist?
- If it names a symptom, reproduce it. A bead claiming a conversion drops a
  target is checkable in one run.
- If it names an acceptance test ("this target going green"), run it.
- Does a closed bead's fix cover this one too? Shapes cluster.

**Report P1 findings even when the bead should stay open.** A bead that is
still real but whose *stated diagnosis* is wrong is the most dangerous kind:
someone picks it up, follows the description, and fixes the wrong thing.
`bzl-dc9`'s update was worth more than its closure would have been.

## What is NOT staleness

- **A red fixture with an open bead.** 003, 005, 015 escalate on purpose;
  the bead tracking the agent-loop work is correct, not stale.
- **A deliberately deferred bead.** `bzl-ccv.4` says in its own text why it
  waits for data. Deferral with a stated reason is a decision.
- **An epic whose children are open.** That is an epic working.
- **A bead describing an accepted limitation** — the host `cmake`
  dependency, the non-hermetic pipeline. Those are documented choices, not
  unfixed bugs.
- **Low priority.** P3 is not stale; it is P3.

## Dispatching this as a subagent

**Report-only.** Closing someone's issue is theirs to decide, and the
interesting output is often "still open, wrong description" rather than a
closure.

**Lane.** Whether open issues match the code. Not whether the code is good —
a bug the audit notices in passing belongs to whichever review lane owns it.

**Give it the recent history.** `git log --oneline -30` plus what landed.
Shape 1 is only findable by knowing what changed, and commit messages here
often describe a fix without naming the bead it closes.

**Severity:**

- **P1** — still open and its stated diagnosis is WRONG. Actively
  misdirects whoever picks it up.
- **P2** — already resolved; should be closed with a reason.
- **P3** — accurate but overtaken in scope, or duplicated by another bead.

**Require** a per-bead verdict with the evidence that produced it — the grep
that found nothing, the run that passed. "Looks done" is not a finding. And
require "what I could not verify", which for this pass usually means beads
whose symptom needs a full corpus conversion to reproduce.

**Critique this skill when you are done.** Say whether the shapes above
covered what you found, or whether there is a seventh. That list is the whole
value of this file and it came from a single afternoon's audit — it is
certainly incomplete.
