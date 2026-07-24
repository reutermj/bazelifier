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

The frontend's source of truth is the CMake File API (`codemodel-v2`), not
direct `CMakeLists.txt` parsing — see
[docs/architecture/cmake-frontend.md](docs/architecture/cmake-frontend.md)
for why. Generated `BUILD` files should build **hermetically**: C/C++
output is built via the `llvm` toolchain already registered in
`MODULE.bazel`, not the host's system compiler — this is a hard
requirement, not an aspiration, and is verified in practice (see the
`translator/tests/fixtures/001-hello-world` round trip).

## Where things live

- `translator/` — the Rust translator crate (`translator/src/`:
  `cmake_api.rs` = File API frontend, `codegen.rs` = Bazel output, `model.rs`
  = shared internal build-graph model, `main.rs` = CLI).
- `translator/tests/fixtures/` — small, synthetic CMake "unit" projects used
  to TDD the translator. Each fixture is a real, buildable CMake project;
  don't assume a fixture's expected Bazel output without actually running
  `bazel build`/`bazel test` against it.
- `docs/architecture/` — design docs, one per major component/decision area.
- `docs/runbooks/` — the runbook template and example runbooks. This is the
  interface contract between the deterministic translator and an agent.
  `docs/runbooks/maintenance/` holds recurring repo-maintenance runbooks
  (non-CMake-translation gaps, e.g. regenerating `translator/Cargo.lock`).
- `docs/lore/` — write here when you discover something non-obvious that
  cost real effort to figure out (a CMake quirk, a Bazel toolchain gotcha, a
  reason an approach was abandoned). This is tribal knowledge that isn't
  derivable by re-reading the code.

## Working conventions

- **Language:** core tool is Rust, built via Bazel (`rules_rs`), not Cargo
  directly. `translator/Cargo.toml` + `translator/Cargo.lock` exist only
  because `rules_rs`'s `crate.from_cargo` extension requires a real
  cargo-generated lockfile as its dependency-resolution input — see
  [docs/runbooks/maintenance/001-regenerate-translator-cargo-lock.md](docs/runbooks/maintenance/001-regenerate-translator-cargo-lock.md)
  before touching either file. Regenerating `Cargo.lock` requires a local
  `cargo` (installed via rustup on this machine already); no Bazel build
  ever invokes that local toolchain.
- **Investigate fetched Bazel repos locally, not via WebFetch.** Once
  `bazel build`/`query`/`fetch` has pulled a ruleset into
  `~/.cache/bazel/_bazel_*/*/external/<repo>+/`, read its actual `.bzl`
  source for rule signatures/attributes/behavior instead of trusting
  README summaries fetched from GitHub — READMEs can show a repo's own
  internal usage (self-referential) rather than the downstream-consumer
  API, and can be stale.
- **Test fixtures:** validate the translator against small, synthetic CMake
  "unit" projects built for TDD (see `translator/tests/fixtures/`). Each
  fixture must actually build both with real CMake+Ninja (ground truth) and
  via the translator's generated `BUILD.bazel` (hermetically, through
  Bazel) — don't hand-wave either check. Real-world open-source CMake
  projects will be added as a corpus later — don't assume they exist yet.
- **Build verification direction:** the CMake side still shells out to a
  local `cmake` (for the File API query) — that's an accepted, current
  limitation, not the end state. The Bazel side must already be hermetic
  (via `llvm`/`rules_cc`) for anything the translator generates; don't
  regress that to "whatever the host compiler is."
- **Formatting:** generated and hand-written Bazel files go through
  `buildifier` (`bazel run //:buildifier` to fix, `bazel test
  //:buildifier_check` to verify) — see
  [docs/architecture/bazel-codegen.md](docs/architecture/bazel-codegen.md).
- **Runbooks are a real interface, not a scratch file.** When the
  translator can't resolve something, it should produce a runbook matching
  the template in `docs/runbooks/`, not an ad hoc error message. Keep the
  schema in mind even while it's markdown-first — it's expected to become
  machine-consumable later.

## When you learn something non-obvious

If you (the agent) hit a surprising CMake behavior, a Bazel rule quirk, or
figure out why a prior approach didn't work, add an entry to
`docs/lore/`. Don't let that context evaporate at the end of the session.
