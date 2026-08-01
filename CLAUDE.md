# CLAUDE.md

Project guide for AI agents (Claude Code and others) working in this repo.
See [README.md](README.md) for the human-facing overview.

## What this project is

bazelifier converts build scripts from other build systems into Bazel
`BUILD` files. Two frontends exist — **CMake** (first and more developed) and
**Autotools** — sharing one model and one codegen. Others (Make, Meson, ...)
remain possible later; the boundary that would let them in is now tested
rather than assumed, since the Autotools frontend needed no codegen change and
no new model field.

## Architecture (short version)

1. A **deterministic translator**, written in Rust, discovers a project's
   targets through its own build system (the CMake File API, or the build's
   own command output and `make -p` for Autotools) and mechanically emits a
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
     build — currently a runtime output/exit-code comparison against
     ground-truth artifacts the translator also captures.

Full detail lives in [docs/architecture/](docs/architecture/). Read that
before making non-trivial changes to the translator, codegen, or the
validation pipeline.

Every frontend's source of truth is the build system's own RESOLVED output,
never its input files: the CMake File API (`codemodel-v2` + `cache-v2`) rather
than `CMakeLists.txt`, and the build's own output and `make -p` rather than `Makefile.am`. Both
input forms carry unexpanded variables and conditionals, so the declared graph
is not the built one — see
[docs/architecture/cmake-frontend.md](docs/architecture/cmake-frontend.md) and
[docs/architecture/autotools-frontend.md](docs/architecture/autotools-frontend.md)
for why in each case. Generated `BUILD` files must build **hermetically**: C/C++ output
is built via the `llvm` toolchain the generated `MODULE.bazel` itself
depends on, not the host's system compiler — this is a hard requirement,
verified in practice by the fact that `libc++`/`libunwind` compile from
source in every fixture build.

## Where things live

- `translator/` — the Rust translator crate:
  - `src/autotools.rs` — Autotools frontend: recovers the graph from the
    build's own stdout (the resolved command stream, this frontend's File API
    analogue) joined with `make -p` (make's variable database, which carries the target NAMES
    the command stream lacks). See
    [docs/architecture/autotools-frontend.md](docs/architecture/autotools-frontend.md).
  - `src/cmake_api.rs` — CMake frontend: reads the File API
    (`codemodel-v2` + `cache-v2`) and runs the real build to capture
    ground-truth artifacts. Derives the module root and rebases reported
    paths onto it (building on `src/paths.rs`).
  - `src/configure_file.rs` — `configure_file` handling, separate because
    it's the one part of the frontend that parses TEXT: these calls appear
    in no File API reply, so they're recovered from `cmake --trace-expand`
    and the templates read off disk. Owns the `@cc_config` catalog mapping
    (`CATALOG_DEFINES`, kept in step with the Starlark catalog by
    `//:catalog_sync_check`).
  - `src/ctest.rs` — the CTest frontend, separate because it reads a
    different source: the File API has no test model, so registered tests
    come from `ctest --show-only=json-v1`. Owns reading the reply, rebasing
    test paths, and deciding which tests the translator can express at all
    (one whose command isn't a binary this module builds escalates).
  - `src/headers.rs` — header classification: which headers a target must
    carry (everything at or below its own include dirs) and which the
    project declared PUBLIC (an `install()` to an include destination, or a
    `FILE_SET`). The two questions have different evidence and different
    failure directions — see the module doc on why its two header predicates
    deliberately disagree.
  - `src/error.rs` — the frontend's error type. Its own module so
    `cmake_api`, `ctest` and `configure_file` can share it without any of
    them importing another: `cmake_api` drives and the other two are
    dependency-free modules it calls into, so homing the error in the driver
    made the callees import their own caller.
  - `src/paths.rs` — pure path geometry (normalize, absolutize, common
    ancestor, resolve-against-source-dir) the frontend's rebasing is built
    on; no CMake or build-graph knowledge, so it generalizes to other
    frontends.
  - `src/codegen.rs` — renders every Bazel file the translator emits: the
    module's `MODULE.bazel` + `BUILD.bazel`, and the small ones for
    `ground_truth/` and `needs_attention/`. If you're writing Bazel syntax
    from Rust, it goes here.
  - `src/needs_attention.rs` — the translator → agent handoff: the text of
    every escalation, plus its markdown rendering.
  - `src/resolutions.rs` — the recipes shipped into each module's
    `resolutions/`: how a *shape* of gap is usually closed, as opposed to
    what went wrong in this project (which is `needs_attention/`'s job).
    Sketches to adapt, never patches to apply, and deliberately duplicated
    per module because a module is meant to be lifted out on its own.
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
- `translator/tests/fixtures/` — small, synthetic "unit" projects used to TDD
  the translator; the numbered ones are CMake and `autotools/` holds the
  Autotools ones. Each fixture's `BUILD.bazel` just calls
  `convert_cmake_project` (or `convert_autotools_project`) with its sources — the module name and
  executable target names are read back out of the translator's own
  generated `MODULE.bazel`/`BUILD.bazel` at execution time (see
  `validation_workspace.bzl`), not hand-declared, so they can't drift from
  what the translator actually emitted.
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
- **Tests come in three tiers, and each one proves something the others
  can't.** Fix a bug at the lowest tier that can fail on it, and don't
  mistake a tier's pass for a claim it never made.
  - *Rust unit tests* (`bazel test //translator:bazelifier_test` — always
    run them through Bazel, never `cargo test`: cargo uses a different
    toolchain and dependency-resolution path, so it can pass while the
    Bazel build is red, which makes its green meaningless here) run over
    inputs we wrote. They pin decisions — classification, path rebasing,
    rendering, escalation text — and they are the only tier where asserting
    a *negative* is cheap. What they cannot do is contradict us: every
    `#[serde(rename)]`/`#[serde(default)]` in `cmake_api.rs` is a claim
    about the CMake File API, and a wrong one deserializes to a default in
    silence.
  - *Fixture conversion* runs real CMake+Ninja over a real `CMakeLists.txt`,
    so it is the only tier that can contradict us — and it captures the
    ground truth automatically. A translator capability isn't finished
    until a fixture exercises it — unit tests agreeing with each other only
    proves we are self-consistent. Real-world open-source CMake projects
    will be added as a corpus later; don't assume they exist yet.
  - *The unpacked validation workspace* is the only tier that proves
    independence and functional equivalence, and the only one that runs the
    generated output as a build rather than as a string. Never validate a
    fixture in place (above); don't hand-wave either half.
- **Green has to be earned.** For each test, ask what edit would make it
  fail — and when the answer isn't obvious, make that edit and watch it go
  red. The failure this repo keeps producing is a check that passes because
  it is looking at nothing: `compare_runtime_output.sh`'s `needs_attention/`
  gate cannot tell an empty directory from a path that doesn't resolve, and
  `translator/tests/BUILD.bazel`'s fixture list silently omits any fixture
  nobody added to it. Four corollaries:
  - **Both directions or neither.** An escalation needs the case that fires
    it *and* the case that must not — fixture `006-sibling-sources` and
    `cmake_api.rs`'s `to_target_no_needs_attention_*` tests exist for that,
    not for coverage. A gate only ever observed failing is
    indistinguishable from one wired to nothing.
  - **Assert the claim, print the evidence.** A `contains` over generated
    text must put that text in the failure message. `assertion failed:
    item.context.contains(..)` sends the reader to the source to find out
    what it actually said, which is the one thing the test already knew.
  - **A comment stating a checkable claim is a test that hasn't been
    written yet** — the "why goes stale too" corollary below, applied to
    behavior. `codegen::render`'s ordering rationale ("nothing yet on disk")
    and `TargetSource::is_generated`'s "reported as an ABSOLUTE path" are
    assertions about what the code does, not commentary about it.
  - **When a fixture contradicts us, what travels down a tier is the
    evidence, not the corrected belief.** Rewriting the claim in a unit
    test just restates the new belief in the same voice as the old one —
    the input is still ours. Capturing the real File API reply that proved
    us wrong, and deserializing *that*, gives the unit tier something it
    can finally be wrong against. Two limits, and the second is the one
    that gets forgotten: captured evidence is frozen, so it catches our
    regression (a dropped `#[serde(rename)]`) and never CMake changing —
    the thing it was captured to check. It therefore supplements the
    fixture and never replaces it. **Deleting a fixture because a unit
    test now covers it is always wrong**, however much the two look alike
    in a diff.
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
- **Comments explain why, not what.** A "what" comment is a second copy of
  the code: it can only be redundant or wrong, and it turns into wrong the
  moment the line above it changes. Spend the comment on what the code
  *can't* say — the alternative that was rejected, a constraint from
  outside this file, the direction a wrong guess fails in.
  `render_path_list` doesn't say that it asserts on each path; it says why
  the assert is real and not a `debug_assert!`. Three corollaries, each of
  which has already cost this repo a stale comment:
  - **One home per rationale.** The comment says why *this code* is shaped
    this way; `docs/architecture/` says why the *design* is. Where they
    overlap, point at the doc instead of restating it. Duplicated
    rationale is how `copy_referenced_sources` came to carry three
    paragraphs of `cmake-frontend.md` that only one of the two would ever
    get updated.
  - **"Why" goes stale too.** When it's assertable, pin it with a test —
    see `needs_attention.rs`'s
    `sources_outside_deliverable_escalation_points_at_the_deliverable_root`.
    Prose naming an identifier (`see X in Y`) is a reference nothing
    checks, so re-grep those after a rename.
  - **"What" earns its place in three spots:** a module-level `//!` or
    docstring orienting someone who lands in the file cold; a one-line
    signature gloss on a non-obvious helper *followed by* the why; and a
    mirror of an external schema (`cmake_api.rs`'s serde structs describe
    the CMake File API, not our choices). What-then-why is the pattern;
    what alone isn't.

  The same bar decides when a comment is *missing* — not "this function has
  no doc comment," since most don't need one, but "would a reader have to
  reconstruct this choice, or would they plausibly *fix* it into a bug?"
  Silent fallbacks, deliberate omissions and magic strings are where that
  reconstruction goes wrong: `is_inherited_via_link_libraries`'s three hops
  all defaulting to "the target's own," `compare_runtime_output.sh` omitting
  `set -e` on purpose, the `+` in `{module_name}+`.

  And when a comment states what a value *is* — its units, its base
  directory, when it is populated — that is a checkable claim, so check it.
  A stale frame of reference is the contradiction this repo keeps
  producing: `Target.sources` documented against the CMake project root
  after derived module roots had moved it, with `is_module_relative` twenty
  lines above already saying otherwise.
- **`needs_attention/` is a real interface, not a scratch file.** When the
  translator can't resolve something, it should emit a `needs_attention`
  item matching the fixed section structure in
  `translator/src/needs_attention.rs`, not an ad hoc error message. Keep
  the schema in mind even while it's markdown-first — it's expected to
  become machine-consumable later. The item's *text* is output too: when
  the translator gains a capability, grep the escalations for the
  limitation it just removed, and pin substantive guidance with a test.
  Because that text ships to an agent working in an unpacked workspace with
  no access to this repo, it is deliberately self-contained — repetition
  across items is a feature, and a pass aimed at reducing duplication must
  leave the escalation strings alone.
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
- **A red fixture is unfinished work, not a terminal state — but starting
  red is expected.** Escalation-firing fixtures (003, 005, 015) are the
  test of the *whole* pipeline including the agent: they start red because
  the translator emitted a `needs_attention` item, the agent stage resolves
  it (always in the **generated** output), and only then do they turn green.
  That start-red → agent-resolves → green cycle is the coverage; a fixture
  that never escalates can't exercise it. So two things are true at once:
  red means there is work to do (an open `needs_attention` item an agent
  must resolve), and a fixture *designed* to escalate is legitimate,
  valuable test input, not a defect. What is a bug: making a red fixture
  green by editing its immutable input or narrowing what it tests, and
  framing a red fixture as a *permanent, acceptable* end state ("expected to
  fail, leave it") rather than as an escalation awaiting resolution. Green
  is still the only *passing* state — the agent has to actually close the
  item — but a fixture being red today, with its item open, is the pipeline
  working as designed.
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
