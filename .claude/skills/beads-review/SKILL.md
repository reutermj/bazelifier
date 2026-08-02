---
name: beads-review
description: Mechanics for auditing the beads issue tracker against the code — finding issues already fixed, whose stated diagnosis has been overtaken, or that were decided and superseded. Use whenever the user asks whether the backlog is accurate, before planning from `bd ready`, and after a run of work that fixed things without referencing bead IDs. Not `bd stale`, which measures activity; this measures whether an issue still describes reality.
---

# Beads review

**`bd stale` does not do this.** It reports issues with no recent activity
and currently returns zero — while a hand audit the same day found six beads
that no longer described reality. Activity and accuracy are different
questions, and only one of them is tooled.

## Why it goes stale, from the six real cases

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

**Critique this skill when you are done.** Say whether the six shapes above
covered what you found, or whether there is a seventh. That list is the whole
value of this file and it came from a single afternoon's audit — it is
certainly incomplete.
