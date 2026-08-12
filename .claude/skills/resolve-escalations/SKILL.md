---
name: resolve-escalations
description: Mechanics for running the pipeline's agent stage — resolving a converted module's needs_attention/ items in the GENERATED output until the module builds and its comparisons pass. Use whenever the user asks to resolve escalations, close out a conversion, make a corpus project green, or take a project from converted to validated. This is a pipeline stage, not a cleanup task; green is the only passing state.
---

# Resolve escalations

**The contract lives in
`docs/architecture/needs-attention-interface.md`**, and the pipeline's shape
in `overview.md`. Read both first.

The agent stage is **inside the loop**, not a fallback beside it. What the
pipeline validates is that a deterministic translator *plus an agent* can
convert a project, so an unresolved item is an unfinished run and **green is
the only passing state**.

## Set the run up with the sweep

`tools/sweep/sweep.py` owns the setup and the measurement; this skill owns
the middle. It deliberately does not resolve anything.

```sh
python3 tools/sweep/sweep.py --post-agent <project> --workspace /tmp/resolve-<project>
```

That unpacks the validation workspace **outside this repo**, lists the open
items with their kinds, and points at the recipes. `--workspace` keeps the
tree so you can edit it and re-measure; without it the tree is temporary.

Re-run the same command after resolving. It unpacks only once, so your work
survives — and it exits 0 only when the items are gone AND at least one
comparison ran AND none failed.

## Where a resolution goes

**In the GENERATED output**, always. Edit the unpacked module's own
`BUILD.bazel` and its own copies of files.

Never edit the project's own build files — `CMakeLists.txt`, `Makefile.am`,
`configure.ac`. They are the input being translated, and "fixing" one leaves
the next project with the same shape just as broken. This is a hard rule in
`CLAUDE.md`, not a preference.

Never edit the translator to special-case this project either. If the fix
belongs in the translator, that is a *different* piece of work — file it and
resolve the module by hand meanwhile.

## What the item and the notes are for

Each `needs_attention/<NNN>-<slug>.md` describes one gap in **this** project
and carries its own guidance — how that shape of gap is usually closed is
written into the item, not kept in a separate file.

`project_notes/` is different, and is the first thing to read when an item
looks obvious. A note records an oddity of **this project** where the
obvious answer is wrong — json-c's `apps/CMakeLists.txt` writes `set(VAR)`
with no value under a comment claiming the feature is present, so the macro
must stay undefined. A module ships the directory only when it has notes.

A note supplies a fact; it does not make the decision. Where a note and an
item seem to disagree, they are usually answering different questions —
re-read both before choosing.

## Two constraints that are easy to violate and hard to notice

- **Do not vendor build-machine results.** Anything the conversion host
  computed — probe answers, a generated config header, an absolute path — is
  a fact about *that machine*, not about the project. Baking it in makes the
  module build correctly only where it was converted, and it will pass the
  comparison, which is what makes this dangerous.
- **Keep the module portable.** No absolute paths, no reference back to this
  checkout. The module has to work when someone drops it into their own repo.

One resolution legitimately needs this checkout: extending the `cc_config`
catalog, since `cc_config` is supplied by `--override_module` and is not in
the tarball. Every other branch must be reachable from inside the module.

## Delete the item when you close it

The `.md` file is the open-work marker. `compare_runtime_output.sh` gates on
`needs_attention/*.md` being empty before it compares anything, so an item
left behind blocks validation even when the underlying gap is fixed.

Deleting it is a claim that the gap is genuinely closed. Do not delete one
you worked around.

## Done means everything passes, not just the build

"The module builds" is necessary and nowhere near sufficient. A conversion is
finished when all four hold:

1. `needs_attention/` holds no `.md` files,
2. the module builds,
3. every **ground-truth comparison** passes — stdout, stderr and exit code
   match what the project's own build system produced, and
4. every test the **module itself** ships passes: the config-header
   assertions, and any test the project registered — a CTest test the
   translator could express becomes an `sh_test` in the module, and it
   counts.

At least one test of *some* kind must have run. Both shapes are legitimate
on their own: a library-only project has no runnable binary to compare and
may still ship an `sh_test`; a fixture with no config header and no
registered test has only its comparison. What must never pass is a project
where everything failed to build, which reports zero passed and zero failed
on both counts.

Re-running `sweep.py --post-agent` checks all four and exits 0 only when they
hold. It reports the last two separately, because they fail for different
reasons and a resolution can easily satisfy one and break the other.

**Point 4 is the one that gets skipped, and for a config-header resolution it
is the only check that bites at all.** A wrong `values` entry or a probe wired
to the wrong fact usually still compiles and still produces byte-identical
runtime output — the comparison cannot see it. The generated
`assert_config_header_test` can. Do not treat a passing comparison as
evidence that a config header is right.

## When you cannot resolve it

Say so, and say why. A resolution that papers over a gap is worse than an
open item, because the item is visible and the workaround is not.

Three things that are **not** resolutions, and are called out in `CLAUDE.md`:
editing the project's input build files, narrowing what a fixture tests, and
deleting the item without closing the gap.

If the honest answer is that the translator should handle this, file it —
`bd create` — and say which project exposed it. That is the pipeline working:
the escalation did its job by making a translator gap visible.

## When a resolution reveals a translator bug

Common, and worth watching for. If several projects need the same edit, or
the recipe's advice is wrong for this project, or the item names a file that
does not exist — that is a finding about the translator or the escalation
text, not about this conversion.

File it separately and keep going. Resolving the module by hand and filing
the gap are both correct; only one of them is this stage's job.
