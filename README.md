# bazelifier

bazelifier converts existing build scripts into Bazel `BUILD` files. It pairs a
deterministic translator with AI-agent assistance for the cases the
translator can't handle mechanically.

The project starts with **CMake** as its first supported build system, but
the architecture is meant to generalize to other build systems (Make,
Autotools, Meson, ...) over time.

## How it works

1. **Deterministic translator** — parses the source build system (e.g. a
   CMake project's `CMakeLists.txt` files) and mechanically emits Bazel
   `BUILD` files for the patterns it recognizes.
2. **Agent fallback via runbooks** — when the translator hits something it
   doesn't know how to handle (an unsupported generator expression, a custom
   command, an unusual dependency shape), it emits a **runbook**: a
   structured description of the gap. An AI coding agent (e.g. Claude Code)
   reads the runbook and provides the missing translation, which feeds back
   into the pipeline.
3. **Build + test verification** — a conversion is only considered successful
   once the generated Bazel targets build *and* the project's existing tests
   pass under Bazel. Build verification is done through Bazel itself where
   possible; see [docs/architecture/build-verification.md](docs/architecture/build-verification.md)
   for the plan to become fully hermetic (native `cmake`/`ninja` rules
   running under Bazel, eventually compatible with remote execution).

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
