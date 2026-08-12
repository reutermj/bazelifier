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

**Do not reason about whether a pure predicate diverged — execute it.**
Compile both copies standalone and feed them the input the difference would
show up on. That is thirty seconds and it converts a suspicion into a fact
the reader cannot argue with. `Config.H` through the two config-header
extension predicates is the worked case: one says true, the other false, and
no amount of reading the two `matches!` arms side by side would have settled
it as fast.

## Bias toward LEAVE

**Scoped to abstractions that do not exist yet.** When a shared helper
already exists and a call site has inlined its body, the presumption flips
to EXTRACT — that is the duplication, and it needs no parameters at all.
Applying the bias there is how the fifth copy of an extension list survived
a review.

For a *proposed new* helper: if it would need three call-site-specific
parameters, the abstraction is wrong. Say so explicitly rather than
proposing it.

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

**If you find three code homes and no doc home, the finding is the missing
doc, not the three comments.** This pass assumes a doc argues and code
restates compactly. When nothing argues, the argument fragments across
whichever files needed it and every copy is equally authoritative, so
"demote to a pointer" has nowhere to point. Measured instance: the gnulib
replacement-header capability spans `autotools.rs`, `model.rs`, `codegen.rs`,
`config_header.bzl` and `expand_config_header.py`, and a grep for
`gnulib|replacement header|shadow` across all of `docs/architecture/`
returns zero. Two of the 2026-08-03 findings were downstream of that
absence rather than independent of it.

## What looks like duplication and isn't

Do not report these:

- **Any text that ships to an agent with no access to this repo** —
  `needs_attention.rs` escalation strings, `project_notes.rs` notes, *and*
  the comments codegen emits into a generated `BUILD.bazel`. CLAUDE.md:
  repetition across items is a *feature*, and a dedup pass must leave them
  alone. **The exemption is about audience, not about two filenames** — read
  literally it would wave through the emitted `genrule` comment, which is the
  same case for the same reason.

  It does **not** cover the code that *builds, selects, or parameterises*
  those strings. That code is the highest-value finding this lane produces,
  and it hides behind the exemption if you read it as "escalations are off
  limits." Two variants seen so far, and the phrasing has to be general
  enough to catch the next one:

  - two frontends constructing one shared constructor's arguments
    differently, so the shared call looks deduplicated and the inputs have
    drifted;
  - one call site passing a **wrong value** for a shared dialect parameter —
    `autotools.rs` passes `needs_attention::ConfigDialect::Autoconf` for an
    `AC_CONFIG_FILES` header a few lines below its own comment saying that
    template is the other dialect. The enum was built to prevent exactly
    this and the second caller defeated it;
  - **the TYPE the strings are selected by, duplicated.** That last case has
    a cause the first two don't: `model::ConfigDialect` has three variants
    and `needs_attention::ConfigDialect` has two, so the `Substitution` case
    has no correct value to pass and the call site had to choose a wrong
    one. Two enums modelling one concept is plain code duplication whose
    only symptom is shipped text — report it.

  And a fourth, which is about the string itself rather than the code
  around it, so the exemption looks like it covers it and does not:

  - **a correction applied across N homes that missed one, where the miss
    is inside a protected string.** The string is exempt from
    *deduplication*, never from being wrong. `039ac2a` corrected
    `static_deps`→`deps` in the item's `context`, the recipe and the lore,
    and left the same item's `expected_output` still naming `static_deps`
    as the success criterion eighteen lines below. One item contradicting
    itself is not repetition, and the 2026-08-03 reviewer nearly suppressed
    it because it pattern-matched the exemption.

  The observable symptom in all four is *text*, which is what makes the
  exemption tempting. The defect is the code that chose it, or a correction
  that did not finish.

  **A named example here that still reproduces is a finding, not
  calibration.** The `ConfigDialect` case above has been carried as an
  illustration across more than one review pass while remaining an open
  bug. When you confirm a worked example is live, say so in the findings —
  the alternative is a skill that quietly documents a defect instead of
  getting it fixed.
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

**Say in the dispatch that the named pairs are the cold-start fix, not the
search space, and that an all-LEAVE verdict on them is a normal result.**
On 2026-08-03 five of six named suspects were LEAVE and the pass's only P1
was on none of them — found instead by the one open-ended prompt in the
list ("what did the second frontend copy rather than share?"). Without that
framing an agent reads the list as the assignment and spends its budget
confirming dismissals. Keep one open-ended question in every dispatch.

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
