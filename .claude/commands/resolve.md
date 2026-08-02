---
description: Run the pipeline's agent stage against one converted project — resolve its needs_attention/ items in the generated output until the module builds and its comparisons pass.
---

Resolve the escalations for `$ARGUMENTS` (a project name, e.g. `xz`,
`json-c`, `zlib`).

**Follow `.claude/skills/resolve-escalations/SKILL.md`.** It is the
mechanics: where a resolution goes, what the item and recipes are for, the
two constraints that are easy to violate and hard to notice, and what done
means. Do not paraphrase it here.

## Shape of the run

1. `python3 tools/sweep/sweep.py --post-agent <project> --workspace /tmp/resolve-<project>`
   to unpack outside the repo and see what is open.
2. Resolve each item in the **generated** output, deleting its `.md` as you
   genuinely close it.
3. Re-run the same command. It exits 0 only when no items remain, at least
   one comparison ran, and none failed.

## Report

Say what you changed and **why that is the right answer for this project**,
not just that the tests pass — a resolution that works for the wrong reason
passes the same comparison.

Call out separately:

- anything you could not resolve, and what is blocking it
- any translator or escalation-text gap the work exposed (file it; the
  escalation did its job by making it visible)
- whether you had to reach outside the module, which should only ever be the
  `cc_config` catalog branch

If nothing is open, say so rather than reporting success — a project with no
escalations was never a task.
