# Bazel overrides apply only from the root module

*(The rules_cc override this describes was removed on 2026-09-18, once
0.2.25 reached the registry. The rule about overrides stands; the mechanism
below is how it was learned.)*

`git_override`, `archive_override`, `local_path_override` and
`single_version_override` are read from the ROOT module's `MODULE.bazel`
only. The same declaration in a module that is being consumed as a
dependency is ignored — silently, apart from a warning that is easy to miss
in a long resolution log.

Where it bit (2026-09-06, bzl-ti9): the translator started pinning rules_cc
to a commit with a `git_override` in every generated `MODULE.bazel`, because
the release it needs (0.2.23, the first to accept `includes = ["."]`) is not
on the Bazel Central Registry. Built from its own directory, a generated
module IS the root and the override applies. Built from the validation
workspace, the same module is a `local_path_override` dependency of a
generated root, so its override is inert and its `bazel_dep` on rules_cc
0.2.23 resolves against a registry that has no such version:

    ERROR: ... module rules_cc@0.2.23 not found in registries

which reads like a broken tarball or blocked egress, and is neither.

The fix is structural, not a flag: `validation_workspace.bzl` copies the
override block out of the first fixture's `MODULE.bazel` into the root it
generates, and `root_module_rules_cc_override_test` asserts the two carry
the same commit. Copied rather than pinned a second time so the commit has
one home in `codegen.rs`; when codegen stops emitting the override the copy
stops with it.

Two things follow for anyone shipping a generated module elsewhere:

- A consumer that depends on a generated module has to repeat the override
  in ITS root. The module's own `MODULE.bazel` says so in a comment, since
  the consumer has no access to this repo.
- This is the same shape as cc_config being supplied by
  `--override_module` on the validation invocation (see
  `cc-config-is-supplied-by-flag-not-shipped-in-the-tarball.md`), and the
  two were resolved differently on purpose: a public git remote is portable
  and can ship in the deliverable; an absolute checkout path is not.
