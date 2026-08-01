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
headers.rs                    header classification, shared by both frontends
paths.rs                      pure path geometry, knows nothing about either
model.rs                      build-system-neutral
codegen.rs                    imports only model; must not learn which frontend ran
error.rs, needs_attention.rs  shared
```

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
3. **Where could the two frontends silently disagree?** The highest-value
   question, because the answer is invisible by construction. Found:
   `deliverable_root` accepted and ignored by one frontend, so equivalent
   projects converted differently and nothing said so.
4. **What does a shared field mean to each caller?** A model field two
   frontends populate with different conventions is a leak the type system
   does not catch — `ConfigHeader::values`, C-quoted by one and raw by the
   other, documented as neither.
5. **What is accepted-and-ignored?** A parameter taken and never read is a
   contract silently unhonoured. Grep for `_`-prefixed parameters.

## What looks like a leak and isn't

- **`use_default_shell_env` and the host `cmake` dependency.** An accepted,
  documented current limitation, not an oversight. CLAUDE.md says so.
- **The pipeline being non-hermetic.** A modelling choice. Do not propose
  designing the agent stage out in the name of determinism.
- **A frontend having more code than the other.** CMake reads four sources;
  Autotools reads three. Asymmetry is not imbalance.
- **`ctest.rs` being CMake-only.** There is no Autotools test frontend yet;
  `graph.tests` is empty for it. That is absence, not a boundary violation.

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
