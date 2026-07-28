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

One more, and it cuts the other way: **prose describing a red fixture as
intended** is a bug to fix, not a convention to preserve. A red fixture is
an unfinished conversion. Scope such framing to "until the agent stage
resolves it."

## Verify and report

Comment-only edits still need the crate to build, and doc comments can
break `rustfmt`:

```sh
cd translator && cargo test && cargo fmt --check && cargo clippy --all-targets
bash -n translator/build_defs/compare_runtime_output.sh   # if touched
bazel test //:buildifier_check                            # if a Bazel file changed
```

`cargo` here is the local rustup toolchain — fast, and no Bazel build
invokes it. If `bazel` fails fetching from `github.com` with a 403, that is
the egress block tracked in `TODO.md`: report it as not-run rather than
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
