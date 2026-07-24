# Runbook interface

The runbook is the contract between the deterministic translator and an AI
agent (e.g. Claude Code). When the translator encounters something it can't
confidently translate, it stops and produces a runbook instead of guessing.

See [docs/runbooks/](../runbooks/) for the actual template and examples.
This doc covers the design intent.

## Why runbooks (vs. just calling an agent API directly)

We're starting with markdown runbooks, consumed by a human handing them to
an agent (or an agent picking them up directly in an IDE/CLI context),
rather than wiring the translator directly to a specific LLM provider's API.
This keeps the translator provider-agnostic and lets any agent capable of
reading a markdown file and editing a repo participate — no SDK lock-in
while the format is still stabilizing.

## Design intent

- **Markdown first, structured underneath.** Runbooks should read naturally
  to a human/agent, but their sections should be consistent and eventually
  mechanically extractable, so this can grow into a machine-readable
  (YAML/JSON) task spec later without a redesign. Don't add prose-only
  sections that couldn't later become structured fields.
- **Self-contained.** A runbook should contain enough context (what file,
  what construct, what the translator already inferred, what's expected
  back) that an agent doesn't need to re-derive the surrounding project
  state from scratch.
- **One gap per runbook.** Don't bundle multiple unrelated unhandled
  constructs into a single runbook — makes them harder to resolve
  independently and harder to test.
- **Resumable.** After an agent resolves a runbook, the translator should be
  able to pick the pipeline back up (e.g. by incorporating a hand-authored
  snippet of Bazel, or a new mapping rule) without re-running the entire
  conversion from scratch.

## Open questions

- Exact schema for the "structured underneath" part — revisit once we have
  a handful of real runbooks and can see what fields actually recur.
- How resolved runbooks feed back into the translator (a directory of
  accepted overrides? inline patches to generated BUILD files? new mapping
  rules the translator learns for next time?). Needs a decision once the
  translator exists.
