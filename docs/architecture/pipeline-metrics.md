# Pipeline metrics

How the whole corpus is measured over time, and what those numbers do and do
not mean.

Distinct from [build-verification.md](build-verification.md), which is about
proving ONE conversion correct. This is about noticing that a change to the
translator moved some *other* project — the question no per-conversion check
asks, because each one only ever looks at itself.

## The problem

The translator changes constantly. Before this existed, a change that made
zlib emit two new escalations was invisible until someone converted zlib by
hand. The only signals were a count in `needs_attention/MANIFEST` and
pass/fail of the runtime comparison, and neither was compared against
anything.

## Two measurement points

They answer different questions and neither substitutes for the other.

**Pre-agent** — what the translator alone produced: escalations by kind,
targets emitted, whether the conversion succeeded. Deterministic: the same
commit gives the same numbers, so a change here is a change in the translator.

**Post-agent** — what the project actually delivers, after an agent resolves
the escalations and the module is built and compared. Non-deterministic by
design (see [overview.md](overview.md) on the pipeline being deliberately
non-hermetic).

Pre-agent alone cannot tell whether an escalation is *resolvable*, and an
escalation no agent can act on is worse than one that never fired. Post-agent
alone cannot separate an improved translator from a luckier agent run.

## Why `kind` is the key, and nothing else is

Escalations are grouped by the `kind` field in each item's machine-readable
header (see
[needs-attention-interface.md](needs-attention-interface.md#the-header)).
That choice was measured, not assumed: `needs_attention.rs` changed in 18 of
the last 30 commits that touched it, and 7 of those changed a **title** — and
therefore the `<NNN>-<slug>` filename derived from it.

So a metric keyed on the title or the filename would silently re-partition
whenever someone improved the wording, producing movement in the graph that
never happened in the pipeline. Constructors, by contrast, have only ever
been added: 4 → 5 → 6 → 7 → 8, none ever renamed.

## What it catches, and what it does not

Measured by reintroducing three real bugs from this repo's history rather
than by reasoning about coverage:

| regression | result |
|---|---|
| a conversion silently drops a target | **caught** — `xz.targets: 3 → 2` |
| a change *within* an escalation | **missed** — removing a catalog probe moved xz from 137 to 141 unmapped macros while the item count stayed at 1 |
| a regression that changes no counts or kinds | **missed** — header staging resolving against the wrong base |

This is a coarse net, not a safety net. It sees the *shape* of a conversion
change; it does not see a conversion get subtly wrong. The post-agent half is
what catches the second class, because a module that no longer builds is not
a matter of counting. The magnitude gap is tracked as bzl-ccv.8.

## Reading the graph

An open escalation is **unfinished work awaiting the agent stage**, not a
defect — so a project going red is the pipeline working, and the escalation
count going *up* can mean the translator learned to detect something it
previously ignored. A rising line is not automatically bad, which is why the
report shows movement per project rather than one aggregate number: an
average hides a project that regressed while another improved.

## Where it lives

- `tools/sweep/sweep.py` — runs the sweep; `--post-agent <project>` for the
  second measurement point, opt-in per project because the agent stage costs
  real tokens per run
- `tools/sweep/report.py` — renders the history as one self-contained HTML
  file (inline SVG, no dependencies, no network)
- `metrics/history.jsonl` — one row per commit, appended
- `docs/metrics/index.html` — the committed copy GitHub Pages serves

Each conversion writes its own `CONVERSION.json`, which is what the sweep
reads. That file deliberately does **not** supersede `TARGETS` or
`needs_attention/MANIFEST`: both have consumers with semantics it must not
take over, in particular `MANIFEST`'s *presence* being how
`compare_runtime_output.sh` distinguishes "zero escalations" from "the
runfiles path does not resolve".

## History starts where the record does

There is no backfill. Commits predating `CONVERSION.json` would have to be
measured by counting files instead of reading the record the conversion
wrote, and a series that changes measurement method partway is worse than a
short one.
