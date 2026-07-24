# Architecture docs

This directory holds design documentation for bazelifier's major components
and decisions. Each file covers one area; keep them focused rather than
growing a single monolithic design doc.

## Index

- [overview.md](overview.md) — system overview and pipeline
- [cmake-frontend.md](cmake-frontend.md) — parsing/understanding CMake input
- [bazel-codegen.md](bazel-codegen.md) — emitting Bazel `BUILD` files
- [runbook-interface.md](runbook-interface.md) — the translator ↔ agent
  handoff contract (see also [docs/runbooks/](../runbooks/) for the actual
  templates/examples)
- [build-verification.md](build-verification.md) — how conversions get
  verified (build + test), and the path toward hermetic, remote-execution-
  friendly verification

## Conventions

- These are living design docs, not a decision log. If a design changes,
  update the doc in place rather than appending "UPDATE:" notes.
- If you abandon an approach and it's worth remembering *why*, put that in
  [docs/lore/](../lore/) rather than leaving it as commented-out prose here.
- Mark open questions explicitly as `**Open question:**` so they're easy to
  grep for.
