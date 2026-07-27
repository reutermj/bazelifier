# The `needs_attention/` interface

`needs_attention/` is the contract between the deterministic translator and
an AI agent (e.g. Claude Code). When the translator encounters something it
can't confidently translate, it escalates instead of guessing: it writes a
`needs_attention/<NNN>-<slug>.md` file into *that conversion's own output
tree*, describing the gap and what a good resolution looks like.

The agent is a **stage of the pipeline**, not a fallback beside it (see
[overview.md](overview.md)), so an open item is an unfinished conversion.
The validation tests gate on the directory being empty before they compare
anything — see
[build-verification.md](build-verification.md#equivalence-checks).

Escalating is **per gap, not process-wide**: the translator does not stop.
It converts everything it does understand and records each thing it
doesn't, so an unrecognized construct costs the project that construct
rather than the whole conversion. This follows directly from "one gap per
item" below — a gap that aborted the run couldn't be resolved
independently of every other gap in the project.

## Format

The renderer is `translator/src/needs_attention.rs`, which also holds the
text of every escalation the translator can emit. Four sections, fixed:

```markdown
# <title>

## Gap

What the translator encountered and could not confidently translate.

## Context

What the translator already knows that bears on resolving it — the target,
its dependencies, which other targets were affected, what the construct
normally maps to in Bazel.

## Expected output

What a resolution actually looks like, concretely enough to act on.
```

Files are numbered per conversion (`001-`, `002-`, ...) purely to keep
ordering stable; the slug is derived from the title.

## Design intent

- **Markdown first, structured underneath.** Items should read naturally to
  a human or agent, but the sections are fixed so this can grow into a
  machine-readable (YAML/JSON) task spec later without a redesign. Don't
  add prose-only sections that couldn't become structured fields.
- **Provider-agnostic.** Escalations are markdown on disk, not calls into a
  specific LLM provider's API. Any agent that can read a file and edit a
  repo can participate — no SDK lock-in while the format is stabilizing.
- **Self-contained.** An item should carry enough context that an agent
  doesn't have to re-derive the surrounding project state from scratch.
- **One gap per item.** Don't bundle unrelated unhandled constructs into a
  single file — it makes them harder to resolve independently and harder
  to test.
- **Say what was lost.** When the translator drops something to keep the
  rest of the conversion working (a dependency edge on a skipped target, a
  generated source it can't produce), the item names it explicitly. A
  deliberate omission and an overlooked one are indistinguishable in the
  output otherwise.
- **Type-specific guidance beats a generic error.** "This isn't supported"
  tells an agent nothing it couldn't read off the title. What's useful is
  the shape of the Bazel answer for that particular construct — see
  `unsupported_type_guidance`.
- **Resolvable in the generated output only.** An item must always be
  answerable by changing what bazelifier emits, or by re-running it with
  different inputs (e.g. a wider `--deliverable-root`). "Edit the source
  `CMakeLists.txt` so it translates cleanly" is never a valid resolution —
  the source build files are the input being translated. If a gap looks
  resolvable only by changing the input, that's a signal the translator or
  the escalation text needs to get smarter, not that the project is
  malformed.

## Keeping the text honest

The escalation text is output, and it goes stale like any other output. The
"sources the module cannot reach" item once told agents that module roots
were "not yet derived from the referenced file set" for several commits
after derived module roots had landed — nothing caught it, because no test
read the text.

So: escalations that give substantive guidance carry a test asserting on
that guidance, not just on the title. When the translator gains a capability,
grep the escalation text for the limitation it just removed.

## Open questions

- Exact schema for the "structured underneath" part — revisit once enough
  distinct escalations exist to see which fields actually recur.
- **Open question:** how resolved items feed back into the translator. Today
  a resolution is ephemeral: the agent edits the generated `BUILD.bazel` in
  the unpacked validation workspace and nothing is persisted. A directory of
  accepted overrides? New mapping rules the translator learns for next time?
  Needs a decision — see the fixture-loop item in [TODO.md](../../TODO.md).
- **Open question:** how to tell a genuine resolution from a deleted markdown
  file, for gaps where the build goes green either way (header visibility is
  the live example). See
  [build-verification.md](build-verification.md#header-visibility-is-not-enforced-by-default).

## Not to be confused with `docs/runbooks/`

[docs/runbooks/](../runbooks/) holds **repo maintenance** procedures for
people working on bazelifier itself (e.g. regenerating `Cargo.lock`).
Nothing there is emitted by the translator, and nothing there is part of
this interface.
