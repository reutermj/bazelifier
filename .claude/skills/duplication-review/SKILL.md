---
name: duplication-review
description: Mechanics for a duplication pass over the bazelifier repo — the two lanes it splits into (rationale, and code), what each lane's P1 actually means, and the things that look duplicated and are deliberate. Use whenever the user asks about duplication, deduplication, shared utilities, or whether something should be extracted; whenever a second frontend or backend has just landed and you want to know what it copied; and before any cleanup pass aimed at reducing repetition, since several of the most repetitive things in this repo are repetitive on purpose.
---

# Duplication review

**The standard lives in `CLAUDE.md`** — "one home per rationale" for prose,
and the architecture layering for code. Read it first; this file is only the
mechanics.

This pass splits into **two lanes that must not be run together**. They find
different things, and an agent given both does one of them properly:

- **rationale** — the same *decision* explained twice: comment vs doc, doc vs
  doc, comment vs comment.
- **code** — the same *logic* written twice, especially across the two
  frontends.

## The test that makes findings real

For rationale: **if this changed, would both places be found?** That framing
is the whole pass. Without it a reviewer reports verbosity, which is not the
problem — CLAUDE.md is explicit that the harm is a fact with two homes being
updated in one. `copy_referenced_sources` is the named historical case.

For code: **has it already diverged?** One copy fixed and the other not is
the strongest possible evidence for extraction and the highest-value finding
available. Rank those P1 even when the divergence is currently latent.

## Bias toward LEAVE

If a shared helper would need three call-site-specific parameters, the
abstraction is wrong. Say so explicitly rather than proposing it.

The worked example is the two path rebasers. They look identical — both
resolve, both cap by `deliverable_root`, both drop what escapes — and they
should stay separate: the Autotools closure takes a **per-command** base
because recursive make resolves each path against the directory its own
command ran in, tries **two roots** because an out-of-tree build reports
against both, and classifies escapes into two buckets where CMake uses
three. Base, root list, and escape classifier as parameters is the tell.

But note what that same review *did* overturn: the **survey** halves feeding
those rebasers are identical and had silently diverged. "These solve
different problems" can be true of one half of a function and false of the
other, so re-derive it rather than inheriting the conclusion.

**The rationale lane needs a different bias, and the paragraph above is not
it.** Everything here is calibrated for code, where LEAVE is usually right.
For rationale the answer is almost never "delete one copy" — it is *demote
one to a pointer and decide which home is authoritative*, which is a real
recommendation and should not be talked out of itself. The in-repo model is
`cmake_api.rs`: state the rule compactly, then `See
docs/architecture/cmake-frontend.md`. A clean worked case of choosing a home
is `overview.md` vs `build-verification.md` on the success criteria — the
verification doc is where anyone editing verification behaviour goes, so it
holds the argument and `overview.md` keeps the two-line claim plus its
existing pointer. Two EXTRACTs in a rationale pass is a normal result, not a
sign of over-reach.

## What looks like duplication and isn't

Do not report these:

- **Any text that ships to an agent with no access to this repo** —
  `needs_attention.rs` escalation strings, `resolutions/` recipes, *and* the
  comments codegen emits into a generated `BUILD.bazel`. CLAUDE.md:
  repetition across items is a *feature*, and a dedup pass must leave them
  alone. **The exemption is about audience, not about two filenames** — read
  literally it would wave through the emitted `genrule` comment, which is the
  same case for the same reason.

  It does **not** cover the code that *builds* those strings. Two frontends
  constructing one shared constructor's arguments differently is the highest-
  value finding this lane produces, and it hides behind the exemption if you
  read it as "escalations are off limits."
- **A comment that points at a doc** (`see docs/architecture/X`) *instead of*
  making the argument. A pointer **alongside** a full restatement is a
  finding, not the pattern working — `autotools.rs` has both, thirty lines
  apart. Seeing the pointer is not a reason to stop reading.
- **CLAUDE.md restating `docs/architecture/`** *in summary form with a
  pointer*. It is deliberately the agent-facing summary. But when CLAUDE.md
  carries a full independent argument it becomes a third vote nobody
  recounts: it is one of the five homes of the ephemerality rationale, two of
  which now disagree.
- **The `cc_config`-not-staged note appearing in three places.** The lore
  entry *designs* that placement: the rationale sits at each point the
  mistake gets made, because the reader is inside an unpacked workspace.
  Pinned by `root_module_cc_config_note_test`.
- **Unrelated claims sharing a phrase.** Three uses of "byte-identical" about
  three different things is not duplication.
- **Test scaffolding duplicated across module test modules.** `headers.rs`
  carries a comment defending its local `library_target` copy — sharing it
  would let one module's tests reshape another's fixtures. That reasoning
  holds; assess it, do not overturn it by reflex.
- **Sorting for determinism**, appearing everywhere. Idiomatic, not
  duplicated.

## Dispatching this as a subagent

`/review` runs both lanes as **two separate subagents** with the lanes stated
explicitly, in parallel. They have been run that way once and neither
collided nor duplicated the other's findings.

**Report-only.** Extraction changes behaviour, and every recommendation here
is a judgement call the user should make.

**Work doc-by-doc for the rationale lane**, not file-by-file: list the
decisions a doc *argues*, then find the code implementing each, then read the
comments there. Grepping for shared phrasing only finds the copies that share
words, which is the minority — a previous grep-based pass over the same tree
found one finding where the doc-by-doc pass found six.

**Name the suspected pairs for the code lane** rather than letting it hunt
cold, and include the relevant beads as hypotheses to *test*. A bead's stated
diagnosis can be overtaken: `bzl-dc9` described a case-sensitivity split that
had already been fixed in one of the three modules, and the real finding was
a fourth copy that grew afterwards.

**Severity:**

- **P1** — already diverged; both copies substantive enough that they
  realistically will; **or one copy is missing entirely while a shared
  contract claims it exists.** That last one is not duplication by this
  file's own definition, which is why nothing here used to point at it: it
  is one implementation, two callers, one of which silently skips it, with a
  doc comment asserting otherwise. `Target::soname`'s "already sanitised for
  Bazel label legality" is true only for the frontend that runs
  `target_label`. In a repo this worried about gates looking at nothing, an
  absent second copy outranks a duplicated one.
- **P2** — real duplication, low drift risk.
- **P3** — borderline, judgement call.

**Require** a list of what was examined and found genuinely distinct, and an
explicit "what I could not verify".

**Critique this skill when you are done.** Say plainly whether anything here
was wrong, missing, or misleading; whether it led you to what you found or
you found it anyway; and whether its do-not-report list suppressed something
it should not have. That feedback is the mechanism by which this file stops
being wrong — a stale motivating example teaches the next agent to look
backwards, and an over-broad exemption silently costs findings. Report it
alongside the findings, not instead of them.
 For the code lane, require a
recommendation per finding — EXTRACT, PARAMETERISE (name the parameter), or
LEAVE (name what distinguishes them) — plus a rough size.
