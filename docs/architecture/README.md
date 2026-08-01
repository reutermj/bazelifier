# Architecture docs

This directory holds design documentation for bazelifier's major components
and decisions. Each file covers one area; keep them focused rather than
growing a single monolithic design doc.

## Index

- [overview.md](overview.md) — system overview and pipeline
- [cmake-frontend.md](cmake-frontend.md) — reading CMake input, via the
  CMake File API
- [autotools-frontend.md](autotools-frontend.md) — reading Autotools input,
  via `make`'s resolved command stream and variable database. Written to be
  read alongside `cmake-frontend.md`: the two frontends make the same
  decisions for the same reasons and differ mainly in available evidence
- [bazel-codegen.md](bazel-codegen.md) — emitting Bazel `BUILD` files
- [needs-attention-interface.md](needs-attention-interface.md) — the
  translator → agent handoff contract: what the translator emits when it
  can't convert something, and what a resolution has to look like
- [build-verification.md](build-verification.md) — how ONE conversion gets
  verified (build + test), and the path toward hermetic, remote-execution-
  friendly verification
- [pipeline-metrics.md](pipeline-metrics.md) — how the whole corpus is
  measured over time, so a change to the translator that moves some OTHER
  project is visible. The question no per-conversion check asks
- [configure-file-and-toolchain-probes.md](configure-file-and-toolchain-probes.md)
  — `configure_file`-generated config headers via a shared Bazel-native
  probing module. Used by both frontends: the catalog resolves CMake's
  `#cmakedefine` and autoconf's `#undef` identically

## Conventions

- These are living design docs, not a decision log. If a design changes,
  update the doc in place rather than appending "UPDATE:" notes.
- If you abandon an approach and it's worth remembering *why*, put that in
  [docs/lore/](../lore/) rather than leaving it as commented-out prose here.
- Mark open questions explicitly as `**Open question:**` so they're easy to
  grep for.
