# Bazel codegen

Covers how bazelifier turns the internal build-graph model (see
[cmake-frontend.md](cmake-frontend.md)) into Bazel `BUILD` files.

## Goals

- Emit idiomatic, human-readable `BUILD` files — output is meant to be
  reviewed and maintained by people, not treated as a black box.
- Prefer native Bazel rules (`cc_library`, `cc_binary`, `cc_test`, etc.) over
  custom genrule-wrapping of the original build system wherever the
  translator can confidently produce them.
- Where the translator can't confidently produce a native rule, fall back to
  the runbook process (see [runbook-interface.md](runbook-interface.md))
  rather than silently emitting something wrong or overly conservative.

## Open questions

- **Rule set / ruleset dependencies:** which Bazel rulesets do we standardize
  on for C/C++ (`rules_cc`, bare native rules, etc.)? Needs a decision once
  we're generating real output.
- **WORKSPACE vs `MODULE.bazel`:** assume Bzlmod (`MODULE.bazel`) as the
  default target given it's the current Bazel direction, but confirm before
  committing.
- **Formatting:** should output be passed through `buildifier` as a
  post-processing step? Likely yes — capture the decision here once wired
  up.

This doc is intentionally thin until there's real codegen to describe —
expand it as the translator grows rather than speculating ahead of the code.
