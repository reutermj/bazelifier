# bazelifier

bazelifier converts existing build scripts into standalone Bazel modules —
a project's own `MODULE.bazel` and `BUILD.bazel`, ready to check into that
project's own repo with no dependency on bazelifier itself. It pairs a
deterministic translator with AI-agent assistance for the cases the
translator can't handle mechanically.

The project starts with **CMake** as its first supported build system, but
the architecture is meant to generalize to other build systems (Make,
Autotools, Meson, ...) over time.

## How it works

1. **Deterministic translator** — discovers a CMake project's targets via
   the CMake File API and mechanically emits a **standalone Bazel module**
   for it (its own `MODULE.bazel` + `BUILD.bazel`, copied sources) for the
   patterns it recognizes. It also runs the project's real build to
   capture ground-truth artifacts for verification.
2. **Agent fallback via runbooks** — when the translator hits something it
   doesn't know how to handle (an unsupported generator expression, a custom
   command, an unusual dependency shape), it emits a **runbook**: a
   structured description of the gap. An AI coding agent (e.g. Claude Code)
   reads the runbook and provides the missing translation, which feeds back
   into the pipeline.
3. **Independence + equivalence verification** — a conversion is only
   considered successful once the generated module builds with **no
   reference back to bazelifier's own workspace** (verified by packaging it
   into a tarball, unpacking it completely outside this repo, and building
   from there) *and* behaves equivalently to the original CMake build (not
   necessarily binary-identical — currently a runtime output comparison
   against the captured ground truth). See
   [docs/architecture/build-verification.md](docs/architecture/build-verification.md).

## Status

Early stage / prototype. Validation currently uses small, synthetic
("unit") CMake projects built specifically to exercise the translator
(TDD-style), with a longer-term goal of expanding to real-world open-source
CMake projects as corpus.

## Documentation

- [CLAUDE.md](CLAUDE.md) — project guide for AI agents working in this repo
  (also available as [AGENTS.md](AGENTS.md))
- [docs/architecture/](docs/architecture/) — design and component docs
- [docs/runbooks/](docs/runbooks/) — runbook format and examples for
  agent-assisted translation
- [docs/lore/](docs/lore/) — non-obvious discoveries and hard-won context
  that isn't captured elsewhere

## Scope

- **In scope (now):** CMake → Bazel conversion, starting with C/C++ projects
  built with CMake + Ninja.
- **In scope (future):** other build systems (Make, Autotools, Meson, etc.),
  broader language support, hermetic/remote-execution-friendly builds.
- **Out of scope (for now):** anything not related to translating a build
  system's build graph into Bazel.

## License

See [LICENSE](LICENSE).
