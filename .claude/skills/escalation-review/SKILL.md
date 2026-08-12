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

**Take the frontend census from the checkout, never from the module.**
Converted modules ship no input build files at all, so "does this module
contain a `CMakeLists.txt`?" answers *no* for all 37 and will label every
CMake project Autotools:

```sh
grep -rl convert_autotools_project translator/tests/   # the Autotools ones
```

This matters because the highest-value defect in this pass is an escalation
whose prose is right for one frontend and false for the other, and you
cannot see it without knowing which project came from which. Cross-check
against the constructors that frontend actually calls — `autotools.rs` calls
only two.

**`CONVERSION.json` and `TARGETS` ship too, and are in scope.** A silent
drop shows up there as a confident zero rather than as an absence, and no
item exists to be read. An escalation review that only opens
`needs_attention/` cannot see the gaps where the escalation is the thing
that is missing.

Derive the candidates rather than trusting a list — xz was the standing
example here until it started reporting 12 tests, and a reviewer chased the
stale claim for several minutes:

```sh
cd <workspace>/fixtures
for d in */; do
  t=$(python3 -c "import json;print(json.load(open('$d/CONVERSION.json')).get('tests',0))" 2>/dev/null)
  b=$(grep -c '^cc_binary(' "$d/BUILD.bazel" 2>/dev/null)
  [ "$t" = 0 ] && [ "${b:-0}" -gt 0 ] && echo "$d tests=0 binaries=$b"
done
```

A hit is a candidate, not a verdict: the zero may be correctly escalated
elsewhere. Check for an item before calling it silent.

## What to ask

Three groups, and they are different questions rather than a checklist of
seven. **Is it right** (1-5), **should it exist** (6), **is it specific
enough** (7).

### Is it right?

1. **Is it TRUE for this project?** The live failure shape: text written for
   CMake shipped into an Autotools module. xz's escalation tells the agent
   about `#cmakedefine` and not to edit `CMakeLists.txt`; xz has neither.

   Do not scope this to one item. Text that ships is copied into *every*
   converted module, so one wrong sentence is wrong N times — ask it of
   every piece of text the conversion ships, and check which constructors
   `autotools.rs` actually calls, since anything both frontends emit is
   suspect by default.

   The `resolutions/` recipes were the standing instance here — CMake-flavoured
   bodies shipped into every Autotools module — and they are GONE as of
   2026-08-12, folded into the escalations themselves. Do not go hunting for
   that directory. What the fold means for this pass: the per-dialect wording
   now lives in `TestDialect`/`ConfigDialect` methods, so a leak shows up as a
   MISSING dialect branch rather than as a stale file. Check that a
   dialect-aware string really has both arms (`tests_the_build_itself` is the
   newest) and that neither arm reads like the other's build system. What was
   CMake-flavoured throughout (
   entirely `configure_file()` and `#cmakedefine`) and ship unchanged into
   every Autotools module.
2. **Can the agent actually DO this?** The resolution must be reachable from
   inside the unpacked module. "Re-run the conversion with a wider
   `--deliverable-root`" is actionable; anything requiring a file, script or
   bead that only exists in this repo is not.
3. **Does every name in it exist THERE?** Every path, target, flag and file
   the item mentions, checked against the unpacked WORKSPACE — the module,
   its `project_notes/` if it has one, and the workspace root, not this
   checkout. The root `MODULE.bazel` is where several answers live.

   **Read `project_notes/` as part of this pass, not as context.** It ships
   only for projects that have one, and it is where a fact the item cannot
   derive is recorded — json-c's `set(VAR)`-with-no-value trap, where the
   obvious resolution is wrong and passes every gate. A note that contradicts
   its item, or names something the module does not contain, is the same
   class of defect as a false item.
4. **Has a capability landed that this text still calls impossible?**
   `CLAUDE.md` requires this after every translator capability and nothing
   enforces it. Diff the escalation's claims against what the frontend now
   does.

   **And after a CORRECTION, verify it landed in every copy** — the item's
   `context`, its `expected_output`, both arms of any dialect method it
   uses, and every branch of the item's own prose. This is the opposite direction from a
   capability landing, and the question above does not point at it. Live
   instance: `039ac2a` corrected `static_deps`→`deps` in the context and
   the recipe, and left `expected_output` naming `static_deps` as the
   success criterion eighteen lines below — one item contradicting itself,
   found only because a per-run brief named it.
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

**A comment claiming an escalation exists is escalation-lane evidence** —
grep the drop sites for prose asserting the opposite of what they do. Neither
a pure escalation pass (reads the shipped items, so an absence is invisible)
nor a pure comment pass (reads source, but cannot know what ships) finds
this; it falls between the two and needs claiming explicitly. Live instance:
`autotools.rs` says of `external_links`/`unbuilt` that they are "collected
rather than dropped: it is an input the generated module cannot satisfy,
which is an escalation, not a silent omission", roughly two hundred lines
above the `let _ =` that discards both.

**A clean conversion is a claim.** Zero escalations asserts everything was
understood. Spot-check a non-trivial one against what the project declares.

A findings say "stop escalating this, here is the rule". B findings say
"start escalating this, here is what is being lost" — name the input that
would trigger it and what the user sees today instead.

### Is it specific to THIS project?

An item can be true, actionable and correctly emitted, and still be nearly
useless — because it describes the general shape of a problem and leaves the
agent to rediscover the particulars.

The worked case is fmt's `header_visibility` items. The item says the target
"has header-like files among its sources" and asks the agent to determine
which are "actually part of its public interface" — naming none of them. The
word "header" appears nine times; no filename appears once. Meanwhile the
generated `BUILD.bazel` in the same module shows `test-main`'s `srcs`
carrying `test/gtest-extra.h` plus fourteen of fmt's own public headers that
are *already* in another target's `hdrs` — so much of the classification is
derivable from a file the agent is holding, and the item does not say so.

(zlib's `ctest_command_not_a_target` item used to be the example here, on
the grounds that it never printed the commands. It prints them now —
`ctest.rs:178`. Its live defect is different and larger: the prose describes
a project shell script driving a built binary, and all thirteen commands are
`/usr/bin/cmake`, `/usr/bin/ctest` and `/bin/gcov`. Generic prose is not the
only failure; prose that is *specifically wrong about this project* reads as
more helpful and costs more.)

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

  **Restructuring a long uniform list IS in scope**, and is neither
  shortening nor deduplication — say so plainly, because this prohibition
  and the volume guidance sit eighty lines apart and read as being in
  tension. libmicrohttpd ships 209 macro names in one flat bullet list and
  libidn2 164, of which ~120 are a single `GNULIB_*` family with one
  answer. The finding there is "substitute structure for the flat list", not
  "make it shorter".
- **Do not treat a red fixture as a finding.** 003, 005, 015 and others are
  *supposed* to escalate; that is the agent-loop test, not a bug. But a
  fixture being red by design says nothing about whether its item's TEXT is
  right — 015's item is a designed escalation and still carried a P1.
- **Do not make an escalation's shared guidance project-specific.** It is
  emitted for every project that hits that shape. A fact true of one project
  belongs in its `project_notes/`, which is what that directory is for.

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
  over silently so the user learns nothing; its central instruction cannot
  be followed without a fact the translator had and withheld; or **the item
  and the `project_notes/` entry beside it give opposite instructions.** That
  last one is incoherence in the shipment rather than an error in either
  document, so it survives reading either alone. Nothing declares a winner
  between them, which is deliberate — they answer different questions — so
  an agent hitting a real contradiction has no rule to fall back on.
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
