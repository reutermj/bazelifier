# Architecture docs

This directory holds design documentation for bazelifier's major components
and decisions. Each file covers one area; keep them focused rather than
growing a single monolithic design doc.

## Index

- [overview.md](overview.md) — system overview and pipeline
- [cmake-frontend.md](cmake-frontend.md) — parsing/understanding CMake input
- [bazel-codegen.md](bazel-codegen.md) — emitting Bazel `BUILD` files
- [needs-attention-interface.md](needs-attention-interface.md) — the
  translator → agent handoff contract: what the translator emits when it
  can't convert something, and what a resolution has to look like
- [build-verification.md](build-verification.md) — how conversions get
  verified (build + test), and the path toward hermetic, remote-execution-
  friendly verification
- [configure-file-and-toolchain-probes.md](configure-file-and-toolchain-probes.md)
  — planned design for `configure_file`-generated config headers via a
  shared Bazel-native probing module (not yet implemented)

## Conventions

- These are living design docs, not a decision log. If a design changes,
  update the doc in place rather than appending "UPDATE:" notes.
- If you abandon an approach and it's worth remembering *why*, put that in
  [docs/lore/](../lore/) rather than leaving it as commented-out prose here.
- Mark open questions explicitly as `**Open question:**` so they're easy to
  grep for.
