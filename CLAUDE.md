# CLAUDE.md

Project guide for AI agents (Claude Code and others) working in this repo.
See [README.md](README.md) for the human-facing overview.

## What this project is

bazelifier converts build scripts from other build systems into Bazel
`BUILD` files. The first supported source build system is **CMake**; the
architecture should stay general enough to add others later (Make,
Autotools, Meson, ...).

## Architecture (short version)

1. A **deterministic translator**, written in Rust, discovers a CMake
   project's targets via the CMake File API and mechanically emits a
   **standalone Bazel module** for it: its own `MODULE.bazel` +
   `BUILD.bazel`, not a package inside bazelifier's own workspace. That
   distinction is the whole point — see
   [docs/architecture/build-verification.md](docs/architecture/build-verification.md).
2. When the translator can't handle something, it writes a
   **`needs_attention/<NNN>-<slug>.md`** item into that conversion's own
   output tree — a structured markdown description of the gap (what
   construct wasn't understood, what context is available, what output is
   expected). An agent reads the item and produces the missing
   translation. The agent is a **stage of the pipeline**, not a fallback
   beside it: what's under test is "translator + agent," so an unresolved
   gap is an unfinished conversion and **green is the only passing
   state**. See
   [docs/architecture/needs-attention-interface.md](docs/architecture/needs-attention-interface.md).
3. A conversion is verified two ways, both automated:
   - **Independence**: the generated module, packaged into a tarball with
     every other fixture and unpacked completely outside this repo, must
     build via `bazel build`/`bazel test` with zero reference to
     bazelifier's own `MODULE.bazel`.
   - **Functional equivalence** (not binary compatibility) to the original
     CMake build — currently a runtime output/exit-code comparison against
     ground-truth artifacts the translator also captures.

Full detail lives in [docs/architecture/](docs/architecture/). Read that
before making non-trivial changes to the translator, codegen, or the
validation pipeline.

The frontend's source of truth is the CMake File API (`codemodel-v2` +
`cache-v2`), not direct `CMakeLists.txt` parsing — see
[docs/architecture/cmake-frontend.md](docs/architecture/cmake-frontend.md)
for why. Generated `BUILD` files must build **hermetically**: C/C++ output
is built via the `llvm` toolchain the generated `MODULE.bazel` itself
depends on, not the host's system compiler — this is a hard requirement,
verified in practice by the fact that `libc++`/`libunwind` compile from
source in every fixture build.

## Where things live

- `translator/` — the Rust translator crate:
  - `src/cmake_api.rs` — CMake File API frontend; also runs the real build
    to capture ground-truth artifacts.
  - `src/codegen.rs` — renders every Bazel file the translator emits: the
    module's `MODULE.bazel` + `BUILD.bazel`, and the small ones for
    `ground_truth/` and `needs_attention/`. If you're writing Bazel syntax
    from Rust, it goes here.
  - `src/needs_attention.rs` — the translator → agent handoff: the text of
    every escalation, plus its markdown rendering.
  - `src/model.rs` — shared internal build-graph model.
  - `src/main.rs` — CLI: writes the standalone module (sources +
    generated files + `ground_truth/`) to an output directory.
  - `build_defs/convert_cmake_project.bzl` — Bazel rule wrapping the
    translator binary as an action producing a tree artifact (the
    standalone module).
  - `build_defs/validation_workspace.bzl` — packages every fixture's
    tree artifact into one tarball with a generated root `MODULE.bazel`
    (real `bazel_dep` + `local_path_override` per fixture — so fixtures
    can depend on each other too) and root `BUILD.bazel` (generated
    ground-truth comparison `sh_test`s).
  - `build_defs/compare_runtime_output.sh` — the actual equivalence check:
    diffs stdout/stderr/exit code between ground-truth and Bazel-built
    binaries.
- `translator/tests/fixtures/` — small, synthetic CMake "unit" projects
  used to TDD the translator. Each fixture's `BUILD.bazel` calls
  `convert_cmake_project` (declaring `module_name` and `expected_targets`
  explicitly, since those aren't knowable from Starlark until the
  translator action actually runs — see the doc comments in
  `convert_cmake_project.bzl`).
- `translator/tests/BUILD.bazel` — calls `validation_workspace`, listing
  every fixture. Add new fixtures here.
- `docs/architecture/` — design docs, one per major component/decision area.
- `docs/runbooks/` — recurring **repo-maintenance** procedures (e.g.
  regenerating `translator/Cargo.lock`). Nothing here is emitted by the
  translator; the translator → agent contract is
  `docs/architecture/needs-attention-interface.md`.
- `docs/lore/` — write here when you discover something non-obvious that
  cost real effort to figure out (a CMake quirk, a Bazel toolchain gotcha, a
  reason an approach was abandoned). This is tribal knowledge that isn't
  derivable by re-reading the code.

## Working conventions

- **Never validate a fixture in-place.** A fixture's own `BUILD.bazel`
  building successfully inside this repo proves nothing on its own — it
  may just be inheriting `rules_cc`/`llvm` from bazelifier's own
  `MODULE.bazel`. The real check is: `bazel build
  //translator/tests:validation_workspace`, unpack the resulting tarball
  into a directory with no relationship to this repo, and `bazel
  build`/`bazel test` from *that* root. See
  [docs/architecture/build-verification.md](docs/architecture/build-verification.md#why-unpack-it-rather-than-validate-in-tree).
- **Language:** core tool is Rust, built via Bazel (`rules_rs`), not Cargo
  directly. `translator/Cargo.toml` + `translator/Cargo.lock` exist only
  because `rules_rs`'s `crate.from_cargo` extension requires a real
  cargo-generated lockfile as its dependency-resolution input — see
  [docs/runbooks/001-regenerate-translator-cargo-lock.md](docs/runbooks/001-regenerate-translator-cargo-lock.md)
  before touching either file. Regenerating `Cargo.lock` requires a local
  `cargo` (installed via rustup on this machine already); no Bazel build
  ever invokes that local toolchain.
- **Investigate fetched Bazel repos locally, not via WebFetch.** Once
  `bazel build`/`query`/`fetch` has pulled a ruleset into
  `~/.cache/bazel/_bazel_*/*/external/<repo>+/`, read its actual `.bzl`
  source for rule signatures/attributes/behavior instead of trusting
  README summaries fetched from GitHub — READMEs can show a repo's own
  internal usage (self-referential) rather than the downstream-consumer
  API, and can be stale. This caught real bugs (e.g. `buildifier_test`'s
  `no_sandbox` requiring a `workspace` attribute, only visible in
  `factory.bzl`).
- **Test fixtures:** each fixture must actually build both with real
  CMake+Ninja (ground truth, captured automatically by
  `convert_cmake_project`) and via the unpacked validation tarball
  (hermetically, through Bazel) — don't hand-wave either check. Real-world
  open-source CMake projects will be added as a corpus later — don't
  assume they exist yet.
- **Build verification direction:** the CMake side still shells out to the
  host's `cmake` (`use_default_shell_env = True` in
  `convert_cmake_project.bzl`) — that's an accepted, current limitation,
  not the end state. The Bazel side must already be hermetic (via
  `llvm`/`rules_cc`) for anything the translator generates; don't regress
  that to "whatever the host compiler is."
- **Formatting:** generated and hand-written Bazel files go through
  `buildifier` (`bazel run //:buildifier` to fix, `bazel test
  //:buildifier_check` to verify) — see
  [docs/architecture/bazel-codegen.md](docs/architecture/bazel-codegen.md).
- **`needs_attention/` is a real interface, not a scratch file.** When the
  translator can't resolve something, it should emit a `needs_attention`
  item matching the fixed section structure in
  `translator/src/needs_attention.rs`, not an ad hoc error message. Keep
  the schema in mind even while it's markdown-first — it's expected to
  become machine-consumable later. The item's *text* is output too: when
  the translator gains a capability, grep the escalations for the
  limitation it just removed, and pin substantive guidance with a test.
  See
  [docs/architecture/needs-attention-interface.md](docs/architecture/needs-attention-interface.md).
- **Never edit input build files to make a conversion succeed.** A
  fixture's `CMakeLists.txt` (and a real project's, later) is immutable
  test input. If the translator can't handle a construct, fix the
  translator or the escalation text — adding a `FILE_SET`, restructuring
  targets,
  or otherwise "cleaning up" the source to make it translate is not a
  resolution, because it leaves the next project with the same shape just
  as broken. Resolutions always land in the **generated** output.
- **A red fixture is a bug, not a feature.** No fixture is "expected to
  fail." If `bazel test` is red in the unpacked validation workspace, the
  agent stage hasn't resolved a `needs_attention` item yet — that's work
  to do, not documented behavior. Don't add prose describing a red test as
  intended.
- **The pipeline is deliberately non-hermetic**, and that's a modelling
  choice, not a defect: converting a build system involves judgement calls,
  and the equivalence checks (not reproducibility of the process) are the
  contract. Don't redesign the agent stage out of the loop in the name of
  determinism. This is separate from the requirement that *generated
  output* build hermetically, which still holds.

## When you learn something non-obvious

If you (the agent) hit a surprising CMake behavior, a Bazel rule quirk, or
figure out why a prior approach didn't work, add an entry to
`docs/lore/`. Don't let that context evaporate at the end of the session.


<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:970c3bf2 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   bd dolt push
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->
