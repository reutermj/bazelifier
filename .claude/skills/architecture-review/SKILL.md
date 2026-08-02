---
name: architecture-review
description: Mechanics for an architecture pass over the bazelifier repo — the standing questions about the frontend/codegen boundary, what a leak actually looks like here, and the layering the pass is measuring against. Use whenever the user asks about architecture, layering, seams, module boundaries, or whether something is in the right place; whenever a new frontend, backend or shared module has just landed; and before extracting a module, since the question of whether a seam is real is the thing this pass exists to answer.
---

# Architecture review

**The standard lives in `CLAUDE.md`** — the architecture section and "Where
things live". Read it first; this file is only the mechanics.

The layering being measured against:

```
cmake_api.rs / autotools.rs   frontends: read a build system, produce a model::BuildGraph
ctest.rs, configure_file.rs   CMake-side, called BY cmake_api; they import nothing from it
ninja_deps.rs                 CMake-side, reads `ninja -t deps`
headers.rs                    header classification, shared by both frontends
paths.rs                      pure path geometry, knows nothing about either
model.rs                      build-system-neutral
codegen.rs                    imports only model; must not learn which frontend ran
error.rs, needs_attention.rs  shared
resolutions.rs                static recipe text shipped into each module
main.rs                       driver: consumes a Discovery WITHOUT knowing which
                              frontend produced it
```

`main.rs`'s contract is the one most easily forgotten, and it is where
frontend knowledge escapes to.

## The standing questions

Asking these beats "review the architecture", which returns essays. Each has
found something real:

1. **Did the newest frontend leak into shared code?** Not "is there shared
   code" — is there shared code whose shape only makes sense for one caller.
   The found instance: `codegen` hardcoding `#cmakedefine` in an assertion
   applied to every config header, in the one module whose contract is
   "imports only `model`".
2. **Is there a seam worth extracting?** A *different input read a different
   way* is a seam; line count is not. `configure_file.rs` earned its module
   by reading the configure trace rather than the File API.

   To find candidates rather than just judge them: **list every distinct
   input each frontend reads, and check each is read from exactly one
   module.** Doing that surfaces `main.rs` parsing libtool `.la` files and
   wrapper scripts — a third Autotools input, read from the driver rather
   than the frontend.
3. **Where could the two frontends silently disagree?** The highest-value
   question, because the answer is invisible by construction — nothing fails,
   two equivalent projects just convert differently.

   Look for a field one frontend processes and the other does not, and for a
   **fix applied to one frontend and never swept to the other** — currently
   this repo's dominant defect shape. Live instance: `6b7de3d` anchored every
   path an escalation names, touched `cmake_api.rs` only, and both frontends
   call the same constructor, so Autotools still passes a raw sandbox path
   into shipped text. Its sibling shape is a *shared* constructor whose
   arguments are built by two drifted paths — perfect deduplication at the
   call it makes, divergence in what it is handed.

   (Three earlier instances are FIXED — do not go hunting: `deliverable_root`
   accepted and ignored; `public_headers` not rebased; the module-root survey
   reading a narrower path set. All three are worked solutions, and a cold
   agent chasing them spends its budget re-confirming fixes.)
4. **What does a shared field mean to each caller?** A model field two
   frontends populate with different conventions is a leak the type system
   does not catch. Live instance: `Target::soname`'s doc asserts the name
   "has already been sanitised for Bazel label legality" — only Autotools
   runs `target_label`, so the contract is documented, half-implemented, and
   unenforced. (`ConfigHeader::values`, C-quoted by one and raw by the other,
   was the example here and is FIXED — `model.rs` now documents both
   dialects.)
5. **What is accepted-and-ignored?** A parameter taken and never read is a
   contract silently unhonoured. Grep for `_`-prefixed parameters.
6. **Does a stated invariant still hold — and was it ever the right
   invariant?** Distinct from comment staleness: this is an architectural
   guarantee asserted as settled in `CLAUDE.md` or `docs/architecture/`,
   which the code has since violated. Check the claim's date against the
   commits that would falsify it; `git log --reverse` catches the case where
   a doc was written *already* stale rather than drifting. Live instance:
   "the Autotools frontend needed no codegen change and no new model field"
   was committed 27 minutes after the second commit that added both. The fix
   is usually not to delete the claim — the underlying property was real —
   but to state the invariant that actually holds ("codegen never learns
   which frontend ran") instead of the proxy that does not (a field count).
   A guardrail known to be stale stops being read.

## What looks like a leak and isn't

- **`use_default_shell_env` and the host `cmake` dependency.** An accepted,
  documented current limitation, not an oversight. CLAUDE.md says so.
- **The pipeline being non-hermetic.** A modelling choice. Do not propose
  designing the agent stage out in the name of determinism.
- **A frontend having more code than the other.** CMake reads four sources;
  Autotools reads three, so *volume* asymmetry is not imbalance.

  But a **behavioural** gap is exactly the finding. If one frontend rebases a
  field and the other does not, that is Q3, not asymmetry — and it will look
  like this entry at first glance, because every behavioural gap presents as
  one side having code the other lacks. When in doubt, Q3 wins: this entry is
  about size, not behaviour.
- ~~**`ctest.rs` being CMake-only.**~~ **This exemption is REVOKED and is
  now a live lead.** It said `graph.tests` is empty for Autotools so the
  gap was absence rather than a boundary violation. That premise died with
  `e1c5932`: the Autotools frontend populates `graph.tests` and
  `unexpressed_tests`, and calls a CTest-named escalation constructor. The
  CTest vocabulary has since leaked into `model::Test`, into
  `render_sh_test`, and into a *shipped filename* — three of four Autotools
  modules carry `run_cmake_test.sh`. An agent reading the old exemption at
  face value drops that finding; one did, and only a per-run brief saved it.

  Kept visible rather than deleted, because the lesson generalises: an
  exemption is a claim about the world with a shelf life. **Before honouring
  one, check its stated premise still holds** — this one outlived its truth
  by four days and cost a P1's worth of near-miss.

## Dispatching this as a subagent

**Report-only.** Every finding here is a design decision.

**Lane.** Layering and boundaries. Not comment accuracy, not code
duplication — though this pass and the code-duplication lane will surface
adjacent findings, and that is fine: one asks "should these be one thing",
the other asks "is this in the right place".

**Give it the standing questions above.** A cold agent told to review the
architecture will summarise it back. Told to answer five specific questions,
it audits.

**Severity:**

- **P1** — a boundary violated in a way that is currently producing wrong
  output, or a contract accepted and silently unhonoured.
- **P2** — a violation that is latent, or a seam that should exist.
- **P3** — naming or placement that will mislead later.

**Require** what was checked and found sound, and what could not be verified.

**Critique this skill when you are done.** Say plainly whether anything here
was wrong, missing, or misleading; whether it led you to what you found or
you found it anyway; and whether its do-not-report list suppressed something
it should not have. That feedback is the mechanism by which this file stops
being wrong — a stale motivating example teaches the next agent to look
backwards, and an over-broad exemption silently costs findings. Report it
alongside the findings, not instead of them.


**Check fixture enrollment.** A fixture on disk but absent from
`translator/tests/BUILD.bazel` is never validated, and nothing reports it —
`translator/tests/BUILD.bazel`'s own comment says so. That is an architecture
finding, not a test one: it means a boundary the fixture was written to prove
has never been exercised.

```sh
comm -23 \
  <(find translator/tests/fixtures -name BUILD.bazel -printf '%h\n' \
      | xargs -n1 basename | sort -u) \
  <(grep -oE 'fixtures/(autotools/)?[^:]+' translator/tests/BUILD.bazel \
      | xargs -n1 basename | sort -u)
```

A hit is not automatically a defect — a fixture may be parked deliberately —
but an unenrolled fixture with no comment and no bead saying why is.
