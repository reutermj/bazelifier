---
name: comment-review
description: Review and fix comment quality in the bazelifier repo — stale references to renamed or deleted code, comments that contradict what the code now does, rationale duplicated between code and docs/architecture/, non-obvious judgement calls carrying no explanation, and "what" comments that only restate the code. Use whenever the user asks about comment quality, consistency, staleness, redundancy, accuracy, or doc drift; whenever they ask whether comments still match the code or whether anything is missing a comment. Also reach for it proactively after a rename, file move, or refactor that could orphan a `see X` pointer, and before opening a PR touching translator/src/, translator/build_defs/, or docs/.
---

# Comment review

Comments in this repo carry unusual weight. The translator makes judgement
calls — which header is public, which dependency edge to drop, where the
module root sits — and the reasoning behind them lives almost entirely in
comments and `docs/architecture/`. That reasoning is not recoverable by
reading the code, so when it rots, it takes real knowledge with it.

It rots quietly. A comment is never compiled, never tested, and never fails
a build. The repo has already shipped a comment pointing at an identifier
that no longer existed, a doc describing a packaging mechanism that had been
replaced, and field docs contradicting the invariant those very fields were
required to satisfy. Each survived multiple passes of review because nothing
mechanical was looking.

That is the job: be the thing that looks.

## The convention you are enforcing

`CLAUDE.md`'s working conventions hold the rule — **comments explain why,
not what** — with three corollaries: one home per rationale, "why" goes
stale too, and three sanctioned uses of "what". Read that bullet before
starting; it is the spec, and this skill is its enforcement.

If `CLAUDE.md` and this skill ever disagree, `CLAUDE.md` wins and this file
needs updating.

## How to run a pass

Work through the four categories below. They are ordered by how objective
they are: the first two have right answers, the last two are judgement.
Fix as you go and report at the end — comment edits are cheap to review in
a diff, and batching findings for approval just adds a round trip.

Stop and ask when a comment might be documenting *intent* rather than
current behavior. "This is not yet hermetic" describing an accepted
limitation is not stale just because it is still true; deleting it loses
the fact that it was deliberate.

### 1. Stale references — things that no longer exist

Run `scripts/check_refs.py` first. It cross-checks every backticked
identifier in a comment or doc against the tree, and every relative
markdown link and anchor. It reports candidates, not verdicts — Bazel rule
names (`cc_library`), CMake commands (`target_link_libraries`), and
external symbols will surface and are fine.

What it cannot catch, and you have to read for:

- **A described mechanism that changed.** `build-verification.md` explained
  the tarball as assembled via `mtree_mutate`'s `strip_prefix`/`package_dir`
  long after that approach was replaced by on-disk staging — and
  `package_dir`'s inability to rename a basename was one of the documented
  reasons it was abandoned. The words were all still real; the mechanism
  wasn't. When a comment narrates *how* something works, open the thing and
  check.
- **A capability the translator has since gained.** `needs_attention.rs`
  text once told agents module roots were "not yet derived from the
  referenced file set," several commits after derived roots landed. When
  the translator gains a capability, grep the escalation text for the
  limitation it just removed.
- **A doc pointer that no longer fits.** The Cargo runbook cited
  `docs/lore/` for a convention that lives in `CLAUDE.md`, and
  `bazel-codegen.md` for Cargo wiring that doc has never discussed. A link
  resolving is not the same as it being about the right thing.

### 2. Contradictions — the comment and the code disagree

The highest-value category, and the one that requires actually reading.
Look for a claim in prose and go verify it against the code that claim is
about.

The pattern that has bitten this repo hardest is **a stale unit or frame of
reference**. `model.rs` documented `Target.sources` and `Target.includes` as
"relative to the CMake project root" while `is_module_relative` — the
contract those same fields are required to meet, sitting 20 lines above —
said module root. Since derived module roots landed those differ whenever a
build reaches outside the project directory, which fixture 006 exercises on
every run. `own_include_dirs` had the same error in the other direction: it
claimed to return project-relative paths, and a test three lines below
asserted they come back absolute.

So: when a comment states what a value *is* — its units, its base
directory, its nullability, when it is populated — that is a checkable
claim. Check it.

Also check claims about **what is reachable**. A test comment called its
list "the types actually reachable from a real codemodel reply," but
`cmake-frontend.md` documents `INTERFACE_LIBRARY` as never appearing in one.
Both were written honestly; nobody had put them side by side.

### 3. Duplication — rationale with more than one home

The repo's real failure mode, and it does not look like a problem: both
copies are usually correct and well written. `copy_referenced_sources`
carried three paragraphs near-verbatim from `cmake-frontend.md`'s "only
referenced files enter the module." Nothing was wrong — until one of them
needed updating, and only one would get it.

Resolve it by altitude. The comment says why *this code* is shaped this
way; `docs/architecture/` says why the *design* is. Keep what is local,
replace the rest with a pointer.

Watch for the same rationale appearing three or four times inside one file
— the "Bazel only errors on absolute paths in label attributes, `includes`
is accepted silently" argument was stated at `is_module_relative`, again at
`render_path_list`, and again in a test comment. Keep it at the definition
site and point at it from the others.

**Do not apply this to `needs_attention.rs`'s escalation text.** That text
is *output*, shipped to an agent working in an unpacked workspace with no
access to this repo. `needs-attention-interface.md` calls it self-contained
by design, so repetition there is a feature. Compressing it into "see the
docs" would break the interface. Comments *about* the escalations are fair
game; the strings inside them are not.

### 4. Gaps and "what"-only comments

Two directions, both judgement.

**Missing.** The bar is not "this function has no doc comment" — most don't
need one. It is: *does this code make a choice a reader would have to
reconstruct, or would plausibly "fix" into a bug?* Three that qualified:

- `is_inherited_via_link_libraries` chases three index hops that can each
  come back empty, and all three fall through to "the target's own." That
  default is the whole function and nothing said so.
- `compare_runtime_output.sh` sets `-uo pipefail` and deliberately not
  `-e`, because a nonzero exit from either binary is the data it compares.
  It reads as an oversight and someone will eventually "correct" it.
- The `sh_test` template's `{module_name}+` is Bazel's canonical repo name
  for a Bzlmod module. Not guessable from the template.

The signal they share: silent fallbacks, deliberate omissions, and magic
strings. Those are where a reader's reconstruction goes wrong.

**"What"-only.** A comment restating its own signature. `codegen::render`'s
doc named its return type back at it; `discover` narrated the four calls in
its own six-line body. Replace with the thing the code can't say — for
`discover` that turned out to be two orderings that are load-bearing and
invisible (queries must be written before `configure`; the build step exists
to produce ground truth).

Leave alone: module-level `//!` headers, a one-line gloss *followed by* a
why (`copy_into`), disambiguating glosses (`common_ancestor`'s "deepest"),
the serde structs in `cmake_api.rs` (they mirror the CMake File API schema,
not our choices), and the root `BUILD.bazel`'s buildifier comments (they
give an invocation, which no rule body contains).

## Things in this repo that are not comments to clean up

- **Fixture `CMakeLists.txt` files.** Their comments are load-bearing
  DO-NOT-EDIT markers protecting immutable test input — 003's "DO NOT add a
  FILE_SET here," 006's "DO NOT fix this by moving shared/ under proj/."
  Deleting one as redundant removes the guardrail. Fixture inputs are never
  edited to make a conversion succeed.
- **`Args` field docs in `main.rs`.** These are clap `--help` text, i.e.
  user-facing output.
- **Anything describing a red fixture as intended.** A red fixture is an
  unfinished conversion, not documented behavior. If you find prose framing
  red as a steady state, fix the framing — scope it to "until the agent
  stage resolves it."

## Where the comments are

- `translator/src/*.rs` — the densest reasoning. `cmake_api.rs` and
  `codegen.rs` carry the most.
- `translator/build_defs/*.bzl` + `compare_runtime_output.sh` — Bazel and
  shell subtleties, several with no other home.
- `translator/tests/fixtures/*/BUILD.bazel` and `CMakeLists.txt` — what each
  fixture exists to prove.
- `docs/architecture/*.md` — the design rationale code comments point at.
- `MODULE.bazel`, root `BUILD.bazel`, `translator/tests/BUILD.bazel`.
- `CLAUDE.md`, `README.md`, `TODO.md`, `docs/runbooks/`, `docs/lore/`.

Code and docs cross-reference heavily in both directions, so a rename in
one place strands prose in the other. The pairs worth checking together:
`model.rs`/`cmake_api.rs` ↔ `cmake-frontend.md`, `codegen.rs` ↔
`bazel-codegen.md`, `needs_attention.rs` ↔ `needs-attention-interface.md`,
`validation_workspace.bzl`/`compare_runtime_output.sh` ↔
`build-verification.md`.

## Verifying and reporting

Comment-only edits still need the crate to build, and doc comments can
break `rustfmt`:

```sh
cd translator && cargo test && cargo fmt --check && cargo clippy --all-targets
bash -n translator/build_defs/compare_runtime_output.sh   # if touched
bazel test //:buildifier_check                            # if a Bazel file changed
```

`cargo` here is the local rustup toolchain, used because it is fast and no
Bazel build invokes it. If `bazel` fails fetching from `github.com` with a
403, that is the egress-policy block tracked in `TODO.md` — report it as
not-run rather than working around it, and never disable TLS verification.

When reporting, group findings by the four categories and say what you
changed. Two things matter more than volume:

- **Report what you could not verify.** If `buildifier_check` could not run,
  say so plainly rather than implying a clean pass.
- **Do not inflate the finding count.** A pass that turns up three real
  problems is a good pass. Rewriting healthy comments to look productive is
  the main way this skill could do harm — every rewrite is a chance to
  introduce an error into prose nothing tests.

Finally, hold your own additions to the standard you are enforcing. While
writing a comment about the CMake File API's reply filenames, it is easy to
state a schema more precisely than CMake actually documents. A comment
asserting more than it knows is the next stale comment.

## Re-validating the checker

`check_refs.py` is heuristic, so it can rot the same way a comment can —
and a checker that silently finds nothing is worse than no checker. The
repo's own history is the answer key. Run it against a commit from before a
known cleanup and confirm the known-bad references still surface:

```sh
git archive 6144431 | tar -x -C /tmp/presnap        # before the first cleanup
python3 .claude/skills/comment-review/scripts/check_refs.py /tmp/presnap
```

That tree should report `_combined_mtree` (renamed to `_validation_tree`)
and `untranslatable` (never existed), and the current tree should report
neither. If a change to the script loses either, the change is wrong. Always
check the "Scanned N files" line — an N of 0 means the scan covered nothing,
not that the tree is clean.

## When a finding is worth more than a comment

If the pass turns up something that took real effort to work out — a CMake
quirk, a Bazel toolchain gotcha, why an approach was abandoned — that is
`docs/lore/`, not a longer comment. If it is a recurring procedure, that is
`docs/runbooks/`. Keeping those in their own homes is what stops comments
growing into essays that drift.
