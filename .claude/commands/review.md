---
description: Run the detailed review suite as parallel subagents and collate the findings. Optionally pass one type (comment, test, architecture, duplication) to run just that one.
---

Run the repo's detailed review passes and collate what they find.

`$ARGUMENTS` names which to run. Empty means all of them.

## What to launch

Each pass is a **separate subagent, launched in parallel, report-only**. The
lanes below are what keeps concurrent agents from covering the same ground —
state each agent's lane in its prompt explicitly.

| type | skill | lane |
|---|---|---|
| `comment` | `comment-review` | does a comment still match the code |
| `test` | `test-review` | what the suite proves and fails to prove |
| `architecture` | `architecture-review` | layering and boundaries |
| `duplication` | `duplication-review` | **two agents**: rationale, and code |

`duplication` is two agents because the skill says so — the lanes find
different things and one agent given both does one of them properly.

## Building each prompt

**Read the skill first.** Each has a "Dispatching this as a subagent" section
carrying its standing prompt, its severity ladder, and its do-not-report
list. That section is the prompt. Do not paraphrase it into this file or into
the dispatch — a dispatcher carrying its own copy of five prompts is exactly
the drift shape these reviews exist to find.

Add per-run context the skill cannot know:

- **What changed recently.** `git log --oneline -20` and name the capability
  changes. A cold agent reviewing 12k lines finds whatever it happens to
  open; naming what moved turns sampling into targeting.
- **Relevant open beads, as hypotheses to test rather than accept.** A bead's
  stated diagnosis can be overtaken.
- **Report-only, and do not edit.** Uniform across all types.

## Collating

This is the part that does not exist without the command, and it is where the
value is. Separate reports hide cross-cutting signal: one pass found a
diverged predicate, another had a bead describing the same area whose
diagnosis was already stale, and only reading them together showed the bead
was both right and incomplete.

Produce one report:

1. **Merge and dedupe.** Two passes describing one problem from different
   angles is a stronger finding, not two findings — say so.
2. **Check each against open beads** (`bd list --status=open`). New, or
   already filed? If filed, has the review overtaken the bead's stated
   diagnosis? That is worth flagging on its own.
3. **Rank across passes**, not within them. A P1 from one is a P1.
4. **One merged "what was not verified"** section. Every report has one;
   collapsing them into the volume is how a gap gets lost.

Then ask before filing beads or fixing anything. A review pass produces
findings; acting on them is a separate decision.

## Improve the skill that ran

**Every dispatch asks its agent to critique its own skill**, and this is not
a formality — on the first run of `architecture-review` the critique was more
valuable than several of the findings. Add to each prompt:

> Alongside your findings, tell me plainly: was anything in this skill wrong,
> missing, or misleading? Did its standing questions lead you somewhere
> useful, or did you find what you found in spite of them? Did its
> do-not-report list stop you from reporting something you would otherwise
> have flagged — and was that correct? Be blunt.

The three shapes that came back, all worth acting on:

- **A stale motivating example.** A question cited a bug that had since been
  fixed, and the agent re-verified the fix before finding the live problem. A
  skill that cites a fixed bug teaches the next agent to look backwards.
- **An exemption that was too broad.** "Asymmetry is not imbalance" reads as
  excusing any behavioural gap, since every gap first looks like one side
  having code the other lacks. It nearly suppressed two real findings.
- **A question that recognises but cannot search.** Fine for validating a
  candidate you already have, useless for generating one.

Fold the corrections in **before** filing beads for the findings. A skill
edit is cheap now and expensive to remember later, and the next run inherits
it. Findings age; a bad question keeps costing.

Two cautions. A critique is one agent's experience of one run — a suggestion
that would make the skill longer without making a future finding likelier is
not an improvement, and skills earn their length. And when a run comes back
clean, ask whether the skill's exemptions did that, not just whether the code
is fine.

## Reporting back

Lead with what is actionable now. Say how many passes ran and whether any
came back clean — a review that found nothing is a real result, and
indistinguishable from one that did not run unless you say so.
