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
against the project that produced it. Take the census from the data rather
than a number written here, which would rot:

```sh
# which kinds ship, and how many of each
for f in /tmp/esc/fixtures/*/needs_attention/*.md; do
  sed -n '2s/^kind: //p' "$f"
done | sort | uniq -c | sort -rn

# and which project each came from, to pick one of every kind to read
for f in /tmp/esc/fixtures/*/needs_attention/*.md; do
  printf '%-28s %s\n' "$(sed -n '2s/^kind: //p' "$f")" \
                       "$(basename "$(dirname "$(dirname "$f")")")"
done | sort
```

That reads the machine-readable header every item carries, so it stays right
as kinds are added.

## What to ask

Three groups, and they are different questions rather than a checklist of
seven. **Is it right** (1-5), **should it exist** (6), **is it specific
enough** (7).

### Is it right?

1. **Is it TRUE for this project?** The live failure shape: text written for
   CMake shipped into an Autotools module. xz's escalation tells the agent
   about `#cmakedefine` and not to edit `CMakeLists.txt`; xz has neither.

   Do not scope this to one item. The same defect was in
   `resolutions/README.md` on **all 35 projects**. Ask it of every piece of
   text the conversion ships, and check which constructors `autotools.rs`
   actually calls — anything both frontends emit is suspect by default.
2. **Can the agent actually DO this?** The resolution must be reachable from
   inside the unpacked module. "Re-run the conversion with a wider
   `--deliverable-root`" is actionable; anything requiring a file, script or
   bead that only exists in this repo is not.
3. **Does every name in it exist THERE?** Every path, target, flag and file
   the item mentions, checked against the unpacked WORKSPACE — the module,
   its `resolutions/`, and the workspace root, not this checkout. The root
   `MODULE.bazel` is where several answers live.

   **Read `resolutions/` as part of this pass, not as context.** It ships
   beside every item, it is byte-identical across all modules, and it is
   where the widest-blast-radius defect was found: its README told all 35
   projects never to edit `CMakeLists.txt`, including the Autotools ones that
   have none — while the recipe one file over named `configure.ac` and
   `Makefile.am` correctly.
4. **Has a capability landed that this text still calls impossible?**
   `CLAUDE.md` requires this after every translator capability and nothing
   enforces it. Diff the escalation's claims against what the frontend now
   does.
5. **Would the resolution pass the equivalence check?** An escalation that
   suggests something the comparison would reject is worse than silence.

### Should it exist? Both directions

The questions above ask whether an item is *right*. This asks whether it
should *exist* — and its inverse, which is the harder half.

**A: should this escalate at all?** Escalations are for judgement about
project semantics. Anything the translator could work out mechanically and
punts instead looks like the pipeline working while costing an agent round
trip on every conversion, forever. For each item, ask what the translator
would need to know, and whether it already has it — `main.rs` reads a
libtool `.la`'s `dlname=` and discards it, so codegen cannot emit the right
SONAME.

Volume is a signal here. An item naming 137 macros and one naming 2 are the
same *kind* and a different problem; where a list is long and uniform, ask
whether a stated, testable rule would collapse most of it.

Be conservative. A rule that guesses wrong silently is worse than an
escalation — the whole design says so — so "correctly escalated, leave it"
is a real conclusion and usually the right one. Say *why*.

**B: what should escalate and does not?** Something the translator cannot
handle and passes over in silence. Worse than A, and harder to see, because
the evidence is an absence: you cannot find it by reading the items, only by
asking what a conversion dropped without saying so. This is the repo's named
recurring failure, and it has bitten at least three times — a source that
escaped the module, an unrecognised extension classified as a header,
`unbuilt` targets collected and discarded behind a `let _ =`.

Where to hunt: any `let _ =`, any `filter`/`filter_map`/`partition` that
discards, any `unwrap_or_default`, any `continue` in a loop over declared
things, any `if let Some(..)` whose else-branch does nothing. For each: if
this drops something real, does the user learn?

**Compare the frontends against each other.** One escalating a case the other
silently drops is strong evidence, and they have already diverged that way
twice.

**A clean conversion is a claim.** Zero escalations asserts everything was
understood. Spot-check a non-trivial one against what the project declares.

A findings say "stop escalating this, here is the rule". B findings say
"start escalating this, here is what is being lost" — name the input that
would trigger it and what the user sees today instead.

### Is it specific to THIS project?

An item can be true, actionable and correctly emitted, and still be nearly
useless — because it describes the general shape of a problem and leaves the
agent to rediscover the particulars.

The worked case is zlib's `ctest_command_not_a_target` item: 46 lines, well
argued, and almost entirely generic. The script "usually invokes a binary
this module DOES build". The working directory "often points into the CMake
BUILD tree". Files it reads "have to be listed in the test's `data`". Each
of those is a placeholder where a fact could be — and
`ctest_command_not_a_target_needs_attention` takes `commands: &[String]`,
the actual command lines, which the item never prints. The agent goes
looking for what the escalation already knew.

Two directions per item:

- **What project detail is present?** Does it name the actual targets, files,
  paths, macros, commands — or describe a category and hand over a search?
- **What could the translator have added, from evidence already in hand?**
  Read the constructor's parameters and what the frontend has in scope. A
  known working directory, script path, or consuming target that the item
  does not mention is the finding. Name the fact, say where it already
  exists, and say what the item says instead.

**This is not "make items longer."** Adding generic prose is the opposite of
the fix. The test is whether a sentence would read differently for a
different project; if not it is boilerplate whatever its length, and a long
item may want facts *substituted* for prose rather than added to it.

**Not the same as the deliberate self-containment.** Items repeat context
across each other on purpose, because each ships alone. That repetition
stays. A placeholder where a known fact belongs does not.

## What this pass must NOT do

- **Do not dedup, shorten, or merge.** Items are deliberately
  self-contained and repetitive because each ships to an agent with no
  access to this repo and no other items in view. `CLAUDE.md` is explicit;
  a pass aimed at reducing duplication must leave these strings alone.
- **Do not smooth the tone** or trim what reads as over-explanation. Length
  is not the defect being looked for.
- **Do not treat a red fixture as a finding.** 003, 005, 015 and others are
  *supposed* to escalate; that is the agent-loop test, not a bug. But a
  fixture being red by design says nothing about whether its item's TEXT is
  right — 015's item is a designed escalation and still carried a P1.
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

**"What should escalate and does not" needs the SOURCE too**, and that is
not a contradiction: the
shipped items answer "is this right", but "what is missing" can only be found
by reading the frontends for what they discard. Give it both, and say which
question each is for — an agent handed only the workspace will conclude the
absences do not exist.

**Severity:**

- **P1** — the item states something FALSE for the project it shipped to; its
  resolution cannot be carried out from inside the module; a real gap is passed
  over silently so the user learns nothing; or its central instruction cannot
  be followed without a fact the translator had and withheld.
- **P2** — accurate but names something the agent cannot reach, describes a
  limitation the translator no longer has, or escalates something the
  translator has the evidence to resolve itself.
- **P3** — ambiguous, or missing a detail the agent would have to guess.

A specificity finding is **P3** when the missing detail would merely have
saved a search — it only reaches P1 when the instruction cannot be followed
without it.

**Require** the list of kinds read and which projects they came from — with
seven kinds shipping, a report that does not say which it covered is
unreadable. And require "what I could not verify".

**Critique this skill when you are done.** Say plainly whether anything here
was wrong, missing, or misleading; whether the five questions led you to what
you found; and whether the must-not-do list suppressed something it should
not have. A stale example teaches the next agent to look backwards.
