# CLAUDE.md

Project guide for AI agents (Claude Code and others) working in this repo.
See [README.md](README.md) for the human-facing overview.

## What this project is

bazelifier converts build scripts from other build systems into Bazel
`BUILD` files. The first supported source build system is **CMake**; the
architecture should stay general enough to add others later (Make,
Autotools, Meson, ...).

## Architecture (short version)

1. A **deterministic translator**, written in Rust, parses the source build
   system's files and mechanically emits Bazel `BUILD` files for recognized
   patterns.
2. When the translator can't handle something, it emits a **runbook** — a
   structured markdown description of the gap (what construct wasn't
   understood, what context is available, what output is expected). An
   agent reads the runbook and produces the missing translation.
3. A conversion is verified by building the generated targets and running
   the project's existing tests under Bazel.

Full detail lives in [docs/architecture/](docs/architecture/). Read that
before making non-trivial changes to the translator or runbook format.

## Where things live

- `docs/architecture/` — design docs, one per major component/decision area.
- `docs/runbooks/` — the runbook template and example runbooks. This is the
  interface contract between the deterministic translator and an agent.
- `docs/lore/` — write here when you discover something non-obvious that
  cost real effort to figure out (a CMake quirk, a Bazel toolchain gotcha, a
  reason an approach was abandoned). This is tribal knowledge that isn't
  derivable by re-reading the code.

## Working conventions

- **Language:** core tool is Rust.
- **Test fixtures:** validate the translator against small, synthetic CMake
  "unit" projects built for TDD. These live alongside the translator tests.
  Real-world open-source CMake projects will be added as a corpus later —
  don't assume they exist yet.
- **Build verification direction:** local `cmake`/`ninja` are acceptable
  today for getting things working, but prefer moving verification *into*
  Bazel (native rules/toolchains for cmake/ninja) so builds become hermetic
  and eventually support remote execution and distributed testing. Don't
  add throwaway shell-script verification paths if a Bazel-native path is
  feasible instead.
- **Runbooks are a real interface, not a scratch file.** When the
  translator can't resolve something, it should produce a runbook matching
  the template in `docs/runbooks/`, not an ad hoc error message. Keep the
  schema in mind even while it's markdown-first — it's expected to become
  machine-consumable later.

## When you learn something non-obvious

If you (the agent) hit a surprising CMake behavior, a Bazel rule quirk, or
figure out why a prior approach didn't work, add an entry to
`docs/lore/`. Don't let that context evaporate at the end of the session.
