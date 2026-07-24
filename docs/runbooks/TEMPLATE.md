<!--
Runbook template. Copy this file to start a new runbook — see
docs/runbooks/README.md for naming and lifecycle.

Keep section headings as-is even if a section is brief; consistent
structure is what will let this format become machine-parseable later.
-->

# Runbook: <short title>

- **Status:** open | resolved
- **Source project:** <path or name of the CMake project being converted>
- **Source location:** <file path(s) and line(s) in the source build
  system that triggered this runbook>
- **Translator stage:** <which stage of the translator produced this —
  e.g. frontend parsing, dependency resolution, codegen>

## Gap

What construct or pattern the translator encountered and could not
confidently translate. Be specific — quote the relevant CMake (or other
source build system) snippet.

## Context

Whatever surrounding information the translator already has that's relevant
to resolving this — e.g. the target this belongs to, its known
dependencies, relevant variable values, platform/config assumptions. The
goal is that an agent shouldn't have to re-derive this from the raw project.

## What was tried

What the translator attempted (if anything) before giving up, and why it
wasn't confident in the result. If it produced a tentative/partial Bazel
snippet, include it here clearly marked as tentative.

## Expected output

What kind of resolution is needed — e.g. "a `cc_library` rule fragment
covering these sources," "a determination of whether this custom command
needs a genrule or can be dropped," "a mapping rule the translator can
reuse for similar cases." Be as concrete as possible about the shape of a
good answer.

## Resolution

<!-- Filled in by the agent once resolved. -->

What was actually done to resolve the gap, and the resulting Bazel
snippet/mapping rule/decision. Include enough reasoning that a future
reader (human or agent) understands *why*, not just what.
