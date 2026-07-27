# Maintenance runbooks

Procedures for maintaining **this repo** that are non-obvious, easy to
forget, and recur — the kind of thing that otherwise gets re-derived
painfully every time. Written to be followed by a human or an agent.

These are about bazelifier's own tooling and build wiring. They have
nothing to do with converting a CMake project.

> **Not the translator → agent interface.** When the translator can't
> convert something, it does *not* write here. It writes a
> `needs_attention/<NNN>-<slug>.md` file into that conversion's own output
> tree — see
> [docs/architecture/needs-attention-interface.md](../architecture/needs-attention-interface.md).

## Format

One file per procedure, `<NNN>-<short-slug>.md`. Numbering just keeps
ordering stable; it isn't meaningful. Sections that have proven useful:

- **Status / Trigger** — when to re-run this.
- **Gap** — what doesn't work on its own, and why (cite the actual source
  you read, not a README summary — see
  [CLAUDE.md](../../CLAUDE.md)'s convention on investigating fetched Bazel
  repos locally).
- **What was tried** — the approaches that didn't pan out, so nobody
  retries them.
- **Resolution** — the commands that actually work, and how to verify.

Adapt as the procedure needs; consistency matters more than the exact
headings.

## When to write one instead of lore

- A **runbook** is a procedure you re-run: it has a trigger and steps.
- [docs/lore/](../lore/) is for a *discovery* — a surprising behavior or an
  abandoned approach, where the value is understanding, not a checklist.

If you resolve a runbook and learn something non-obvious on the way, the
discovery half belongs in lore even though the procedure stays here.
