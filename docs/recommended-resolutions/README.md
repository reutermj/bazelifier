# Recommended resolutions

Bugs this project found **in the projects it converts**, written for their
maintainers rather than for us.

Converting a build system reads it more literally than its authors ever do,
so the pipeline turns up real defects in upstream projects as a side effect.
Those findings are worth keeping and worth reporting, and they need somewhere
to live that is not `needs_attention/` — an escalation is a gap in *our*
translation, addressed to an agent resolving *this* conversion. A defect in
the project's own build is a different artifact with a different audience.

## What belongs here

A finding that is **actionable by the upstream maintainer** and **verified**:
you have run the thing, read the generated output, and can show the defect
rather than infer it from reading their build files. The bar is the same one
this repo applies to itself — read the resolved output, not the input's
apparent intent.

## What does NOT belong here

- **A translator gap.** If our conversion is wrong, that is a bead.
- **A style disagreement.** "This CMake could be tidier" is not a defect.
- **Anything that would have us edit their build files.** CLAUDE.md forbids
  editing a project's input build files to make a conversion succeed, and
  that rule is not suspended because we think we found a bug. These are
  reports, not patches we apply.

## The conversion still reproduces the bug

This is the part that is easy to get backwards. A conversion reproduces what
the project's build system **does**, not what it meant to do. Where a
recommended resolution says "this should be `#define`", the converted module
should still emit `#undef` if that is what the project's own build produces —
otherwise the module and the project disagree, and the module is the one
that is wrong.

Fix it upstream, re-pin, and the conversion follows.

## Index

- [json-c: `set()` with no value defeats an in-tree feature assertion](001-json-c-apps-set-with-no-value.md)
