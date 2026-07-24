# Runbooks

A runbook is how the deterministic translator hands off a translation gap
to an AI agent. When the translator encounters a construct in the source
build system (e.g. CMake) that it can't confidently convert to Bazel on its
own, it produces a runbook describing the gap instead of guessing or
silently emitting something wrong.

See [docs/architecture/runbook-interface.md](../architecture/runbook-interface.md)
for the design rationale. This directory holds the concrete format.

## Format

Runbooks are markdown files following [TEMPLATE.md](TEMPLATE.md). They're
written to be human/agent-readable now, with section structure consistent
enough to become machine-parseable later without a redesign — don't add
freeform sections that break that.

## Naming

`docs/runbooks/<project>/<NNN>-<short-slug>.md`, e.g.
`docs/runbooks/examples/001-generator-expression-in-custom-command.md`.
Numbering is per-project, just to keep ordering stable; it's not a global
sequence.

## Directory layout

- [TEMPLATE.md](TEMPLATE.md) — the runbook template. Copy this to start a
  new runbook.
- `examples/` — worked examples showing the format filled in, for reference
  (not necessarily tied to a real in-progress conversion).
- `maintenance/` — runbooks for recurring repo/tooling maintenance tasks
  that aren't CMake translation gaps (e.g. regenerating a lockfile) but are
  still the kind of non-obvious, easy-to-forget procedure worth capturing
  in a consistent, agent-readable format. These don't fit
  [TEMPLATE.md](TEMPLATE.md)'s translation-specific fields (source
  project/location, translator stage) — adapt the section headers to fit
  (trigger, gap, what was tried, resolution) rather than forcing the
  CMake-specific frame.

## Lifecycle

1. Translator hits an unhandled construct, fills in the template as best it
   can (context, what it tried, why it stopped) → this is now an "open"
   runbook.
2. An agent picks up the runbook, resolves the gap, and records the
   resolution in the runbook's `## Resolution` section.
3. The resolution feeds back into the pipeline (exact mechanism TBD — see
   the open question in
   [runbook-interface.md](../architecture/runbook-interface.md)).

If you resolve a runbook and learn something non-obvious in the process
that will help future conversions, consider whether it belongs in
[docs/lore/](../lore/) as well.
