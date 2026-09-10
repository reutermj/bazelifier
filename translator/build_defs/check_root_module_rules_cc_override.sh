#!/usr/bin/env bash
# Pins the other half of the rules_cc override recorded in codegen.rs
# (RULES_CC_OVERRIDE_COMMIT): the generated root MODULE.bazel must repeat the
# git_override every fixture carries, with the SAME commit, because Bazel
# honours overrides only from the root module. A fixture's own override is
# inert once the fixture is consumed as a dependency, so without this copy
# every fixture fails at module resolution against a registry that lacks
# the version — an error Bazel reports far from the cause.
#
# Both directions: if the fixture has no override (the registry caught up and
# codegen stopped emitting one), the root must not carry a stale one either.
# Delete this test with the override — see bzl-ti9.
set -uo pipefail

root_module_bazel="$1"
fixture_module_bazel="$2"

for f in "${root_module_bazel}" "${fixture_module_bazel}"; do
  if [[ ! -s "${f}" ]]; then
    echo "FAIL: MODULE.bazel is missing or empty: ${f}" >&2
    exit 1
  fi
done

commit_of() {
  sed -n '/^git_override($/,/^)$/p' "$1" | sed -n 's/^    commit = "\(.*\)",$/\1/p'
}

fixture_commit="$(commit_of "${fixture_module_bazel}")"
root_commit="$(commit_of "${root_module_bazel}")"

if [[ "${fixture_commit}" != "${root_commit}" ]]; then
  echo "FAIL: rules_cc git_override commit differs between the fixture and the root." >&2
  echo "      fixture (${fixture_module_bazel}): '${fixture_commit}'" >&2
  echo "      root    (${root_module_bazel}): '${root_commit}'" >&2
  echo "      Overrides apply only from the root module, so the root must repeat" >&2
  echo "      exactly what codegen emitted. See validation_workspace.bzl." >&2
  exit 1
fi

# The root must NOT also bazel_dep on rules_cc. Bazel orders registered
# toolchains by module depth, and rules_cc's own MODULE.bazel registers the
# auto-configured HOST toolchain; as a direct dependency of the root that
# registration outranks the fixtures' `@llvm//toolchain:all`, and every
# fixture silently compiles with /usr/bin/gcc. Measured 2026-09-10: the same
# tree selected gcc with the line and clang without it. An override needs no
# bazel_dep — it applies to the module wherever it sits in the graph.
if grep -Eq '^bazel_dep\(name = "rules_cc", ' "${root_module_bazel}"; then
  echo "FAIL: root MODULE.bazel has a direct bazel_dep on rules_cc. That puts" >&2
  echo "      rules_cc's host-toolchain registration ahead of the fixtures'" >&2
  echo "      llvm one and the whole workspace compiles with the host gcc." >&2
  echo "      The git_override alone is enough. See validation_workspace.bzl." >&2
  exit 1
fi

echo "rules_cc override commit: '${fixture_commit:-<none>}' in both fixture and root"
