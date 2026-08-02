---
name: comment-review
description: Mechanics for a comment-accuracy pass over the bazelifier repo — a reference-checking script for stale identifiers and dead links, plus the repo-specific things that look like comment problems but must not be touched. Use whenever the user asks about comment quality, staleness, redundancy, accuracy, or doc drift; whenever they ask whether comments still match the code or whether something is missing a comment. Also reach for it before a redundancy or cleanup pass over comments, since the traps below are not obvious from reading the files, and after a rename or file move that could orphan a `see X` pointer.
---

# Comment review

**The standard lives in `CLAUDE.md`'s working conventions** — the
comments-explain-why bullet, with its corollaries on one home per
rationale, "why" going stale, the three sanctioned uses of "what", the bar
for a *missing* comment, and checkable claims about units and base
directories. Read that first. It is the spec; this file is only the
mechanics that don't belong in a convention list.

Fix as you go and report at the end. Comment edits are cheap to review in a
diff, and batching findings for approval just adds a round trip. Stop and
ask when a comment might document *intent* rather than current behavior —
"not yet hermetic" describing an accepted limitation is not stale merely
because it is still true, and deleting it loses the fact that it was
deliberate.

## Dispatching this as a subagent

`/review` runs this pass in a subagent, which changes two things: the agent
cannot ask a follow-up question, and its report is the only artifact. So the
dispatch is **report-only** — it does not fix as you go — and the prompt has
to carry what an in-session agent would otherwise absorb from context.

**Lane.** Staleness and accuracy: does a comment still match the code. NOT
duplicated rationale — that is `duplication-review`'s lane, and running both
against the same files without the split is how the first pass here missed
duplication entirely.

**Seed it with what changed.** A cold agent reviewing 12k lines finds
whatever it happens to open. Name the recent capability changes and say
which older claims they could have invalidated — that is what turns a
sampling pass into a targeted one.

**Severity, for this review type:**

- **P1** — a reader following the comment makes a wrong change. A stated
  frame of reference that is now false, or a doc comment attached to the
  wrong item (Rust silently welds a doc block to whatever follows it, so
  this compiles).

  **Run clippy first for the orphan class**, before reading anything:

  ```sh
  cd translator && cargo clippy --all-targets 2>&1 | grep -A3 'duplicated attribute'
  ```

  When a test is inserted above an existing one the `#[test]` gets duplicated
  along with the comment, and clippy names every site. Five sat unread in
  this tree, one of them stacking three unrelated rationales above a single
  test. Read those warnings as findings, not lint noise — and note the
  trigger is *any commit adding a test to an existing `mod tests`*, not only
  a rename or file move: three of those five came from plain insertions.
- **P2** — stale or contradicted, but a reader would notice before acting.
- **P3** — a dangling `see X` pointer, a miscounted list.

**Require coverage, not just findings.** "List what you read and found
correct" — without it a short report is unreadable, because clean and
unchecked look identical. And require an explicit "what I could not verify",
which every pass needs and none volunteers.

**Seed hypotheses as things to test, not accept.** An open bead's stated
diagnosis can be overtaken; `bzl-dc9` described a divergence that had
already been half-fixed, and only a reviewer told to *verify* the bead
would have found the other half.

**Critique this skill when you are done.** Say plainly whether anything here
was wrong, missing, or misleading; whether it led you to what you found or
you found it anyway; and whether its do-not-report list suppressed something
it should not have. That feedback is the mechanism by which this file stops
being wrong — a stale motivating example teaches the next agent to look
backwards, and an over-broad exemption silently costs findings. Report it
alongside the findings, not instead of them.


## Run the reference checker

```sh
python3 .claude/skills/comment-review/scripts/check_refs.py
```

It cross-checks every backticked identifier in a comment or doc against the
tree, and every relative markdown link and heading anchor. It reports
candidates, not verdicts: prose legitimately names Bazel rules, CMake
commands, and `rules_rs` internals that live elsewhere, so a handful of
explicable hits is the normal steady state.

It catches renames and deletions. It cannot catch a *described mechanism*
that changed while its vocabulary stayed real — `build-verification.md`
explained the tarball via `mtree_mutate`'s `strip_prefix`/`package_dir`
long after on-disk staging replaced that, and every word was still a word.
When a comment narrates how something works, open the thing.

### Re-validating the checker

A checker that silently finds nothing is worse than no checker. The repo's
history is the answer key: run it against a commit from before a known
cleanup and confirm the known-bad references still surface.

```sh
git archive <pre-cleanup-commit> | tar -x -C /tmp/presnap
python3 .claude/skills/comment-review/scripts/check_refs.py /tmp/presnap
```

Commit `6144431` is the reference point — it should report `_combined_mtree`
(since renamed to `_validation_tree`) and `untranslatable` (never existed),
and the current tree should report neither. If an edit to the script loses
either, the edit is wrong. If that hash ever goes missing, any commit
before the first comment-cleanup on this branch works; find it with
`git log --oneline --all | grep -i "stale\|comment"`. Always read the
"Scanned N files" line — an N of 0 means the scan covered nothing, not that
the tree is clean.

**A doc that enumerates code items goes stale by omission, and no checker
sees it.** `CLAUDE.md`'s "Where things live" lists every translator module
with the reason it exists separately; `docs/architecture/`'s frontend docs
list each frontend's inputs. Nothing breaks when a module is added and the
list is not — but since every other entry states *why this is a real seam*,
an absent entry reads as "not a seam," which is the opposite of what the
extraction argued. Diff the inventory against `ls translator/src/*.rs`; the
reference checker cannot, because a missing name is not a broken one.

## What looks like a finding but isn't

This is the part worth having written down; the rest of a comment pass is
judgement a careful reader already has.

- **`needs_attention.rs`'s escalation strings.** The single most repetitive
  text in the repo — `do NOT edit the project's CMakeLists.txt` appears
  four times — and it must stay that way. That text is *output*, rendered
  into separate `.md` files read in isolation by an agent with no access to
  this repo. Deduplicating it breaks the interface. Comments *about* the
  escalations are fair game; the string literals are not.
- **Fixture `CMakeLists.txt` comments.** Load-bearing DO-NOT-EDIT
  guardrails on immutable test input — 003's "DO NOT add a `FILE_SET`
  here," 006's "DO NOT fix this by moving `shared/` under `proj/`." They
  sit in the file someone would be tempted to edit, which is the whole
  point. Deleting one as redundant removes the guardrail.
- **`Args` field docs in `main.rs`.** These render as clap `--help` text.
  They overlap the `convert_cmake_project.bzl` attr docs by necessity —
  different surface, different audience.
- **Repeated `see docs/architecture/...` pointers.** A pointer repeated in
  five places is the convention working, not five copies of a rationale.
  Per-fixture "Full rationale:" footers exist so each fixture reads
  standalone.
- **Long single-site rationale.** `is_module_relative`, `render_path_list`,
  and `validation_workspace.bzl`'s docstring on the three bsdtar behaviors
  are long but repeat nothing. If the actual request is fewer comment lines
  rather than less duplication, that is a different task — confirm it
  first, because that pass deletes real information.

One more, and the distinction is subtle: prose framing a red fixture as a
**permanent, acceptable** end state ("expected to fail, leave it") is a bug
to fix — but prose describing it as the **expected start of the
agent-in-the-loop cycle** (starts red with an open `needs_attention` item,
the agent resolves it in the generated output, then it goes green) is
correct and load-bearing, not stale. Escalation-firing fixtures (003, 005,
015) are *designed* to start red; that is the coverage. Preserve that
framing; only rewrite prose that presents red as a finished, do-not-touch
state. See `CLAUDE.md`, "A red fixture is unfinished work, not a terminal
state."

## Verify and report

Comment-only edits still need the crate to build, and doc comments can
break `rustfmt`:

```sh
# Tests ALWAYS run through Bazel — never `cargo test` (see CLAUDE.md).
bazel test //translator:bazelifier_test
cd translator && cargo fmt --check && cargo clippy --all-targets  # fmt/lint only, not tests
bash -n translator/build_defs/compare_runtime_output.sh   # if touched
bazel test //:buildifier_check                            # if a Bazel file changed
```

`cargo fmt`/`cargo clippy` here are the local rustup toolchain used only for
formatting and linting — not for running tests, which always go through
Bazel. If `bazel` fails fetching from `github.com` with a 403, that is an
egress limit in a restricted session: report it as not-run rather than
working around it, and never disable TLS verification.

Two things matter more than the size of the finding list. **Say what you
could not verify** — if `buildifier_check` did not run, say so rather than
implying a clean pass. And **do not inflate the count**: a pass that turns
up three real problems is a good pass. Rewriting healthy comments to look
productive is the main way this skill could do harm, since every rewrite is
a chance to put an error into prose that nothing tests. Hold your own
additions to the same standard — it is easy, writing about the CMake File
API, to state a schema more precisely than CMake actually documents.

## When a finding is worth more than a comment

Something that took real effort to work out — a CMake quirk, a Bazel
toolchain gotcha, why an approach was abandoned — belongs in `docs/lore/`.
A recurring procedure belongs in `docs/runbooks/`. Keeping those in their
own homes is what stops comments growing into essays that drift.
