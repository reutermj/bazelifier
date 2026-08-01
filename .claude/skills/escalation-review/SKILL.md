---
name: escalation-review
description: Mechanics for reviewing the escalations the pipeline actually ships — reading the generated needs_attention/ items as the agent receives them, and asking whether each is true, actionable, and followable without access to this repo. Use whenever the user asks whether escalations make sense, whether an agent could act on one, or after the translator gains a capability that an escalation may still describe as impossible. Not a prose-quality pass: repetition across items is deliberate and this skill must not reduce it.
---

# Escalation review

**The contract lives in
`docs/architecture/needs-attention-interface.md`**, and the conventions in
`CLAUDE.md`'s `needs_attention/` bullet. Read both first.

## Review the OUTPUT, not the source strings

This is the whole method, and it is what makes this pass different from
reading `needs_attention.rs`. Generate the pipeline's real output and read
the `.md` files an agent would actually receive:

```sh
bazel build //translator/tests:validation_workspace
mkdir -p /tmp/esc && tar xf bazel-bin/translator/tests/validation_workspace.tar -C /tmp/esc
ls /tmp/esc/fixtures/*/needs_attention/*.md
```

Reading the constructors instead hides the two things most likely to be
wrong. A string that is correct in `needs_attention.rs` can be **false for
the project it lands on** — the same escalation is emitted by both frontends
and only one of them has a `CMakeLists.txt`. And a string that reads fine in
isolation can name a file, flag or target that **does not exist in the
unpacked module**, which is the only place the agent can look.

Group by `kind` from each item's header, then read at least one of every kind
against the project that produced it. Seven kinds ship today.

## The five questions

1. **Is it TRUE for this project?** The live failure: xz's only escalation
   tells the agent about `#cmakedefine` and not to edit `CMakeLists.txt`. xz
   is an Autotools project and has neither. Any escalation both frontends can
   emit is suspect — check `needs_attention.rs` for which constructors are
   called from `autotools.rs`.
2. **Can the agent actually DO this?** The resolution must be reachable from
   inside the unpacked module. "Re-run the conversion with a wider
   `--deliverable-root`" is actionable; anything requiring a file, script or
   bead that only exists in this repo is not.
3. **Does every name in it exist THERE?** Every path, target, flag and file
   the item mentions, checked against the unpacked module — not against this
   checkout. A `resolutions/` recipe it points at must be one of the files
   actually shipped beside it.
4. **Has a capability landed that this text still calls impossible?**
   `CLAUDE.md` requires this after every translator capability and nothing
   enforces it. Diff the escalation's claims against what the frontend now
   does.
5. **Would the resolution pass the equivalence check?** An escalation that
   suggests something the comparison would reject is worse than silence.

## What this pass must NOT do

- **Do not dedup, shorten, or merge.** Items are deliberately
  self-contained and repetitive because each ships to an agent with no
  access to this repo and no other items in view. `CLAUDE.md` is explicit;
  a pass aimed at reducing duplication must leave these strings alone.
- **Do not smooth the tone** or trim what reads as over-explanation. Length
  is not the defect being looked for.
- **Do not treat a red fixture as a finding.** 003, 005, 015 and others are
  *supposed* to escalate; that is the agent-loop test, not a bug.
- **Do not edit `resolutions/` recipes to match a specific project.** They
  are shapes to adapt, deliberately generic.

The defect classes are **inaccuracy** and **unactionability**. Nothing else.

## The strongest verification available

Pick one escalation and **try to follow it**, in an unpacked workspace, as
the agent would. That found `bzl-lvm`. If the instructions cannot be carried
out, or can be carried out and the module still does not build, that is a
finding with proof attached rather than an opinion about wording.

## Dispatching this as a subagent

**Report-only.** Escalation text is a shipped interface; changing it is the
user's call.

**Lane.** Truth and actionability of generated `needs_attention/` items. Not
comment accuracy, not duplication — and explicitly not the prose quality of
`needs_attention.rs` as source.

**Give it the unpacked workspace**, or the commands to produce one. An agent
that reads only the Rust source will report source-level observations and
miss both live failure modes.

**Severity:**

- **P1** — the item states something FALSE for the project it shipped to, or
  its resolution cannot be carried out from inside the module.
- **P2** — accurate but names something the agent cannot reach, or describes
  a limitation the translator no longer has.
- **P3** — ambiguous, or missing a detail the agent would have to guess.

**Require** the list of kinds read and which projects they came from — with
seven kinds shipping, a report that does not say which it covered is
unreadable. And require "what I could not verify".

**Critique this skill when you are done.** Say plainly whether anything here
was wrong, missing, or misleading; whether the five questions led you to what
you found; and whether the must-not-do list suppressed something it should
not have. A stale example teaches the next agent to look backwards.
