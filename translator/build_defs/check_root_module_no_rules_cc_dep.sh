#!/usr/bin/env bash
# Pins a toolchain-selection hazard recorded in validation_workspace.bzl: the
# generated root MODULE.bazel must not depend on rules_cc directly, and must
# not override it either.
#
# Bazel orders registered toolchains by module depth, and rules_cc's own
# MODULE.bazel registers the auto-configured HOST toolchain. As a direct
# dependency of the root, that registration outranks the fixtures'
# `@llvm//toolchain:all`, and every fixture silently compiles with
# /usr/bin/gcc. Measured 2026-09-10: the same unpacked tree selected gcc with
# the line and clang without it. Nothing fails, so nothing but this reports it.
#
# An override is refused for a different reason: it would make the tarball
# resolve rules_cc from somewhere other than the registry, which the
# independence claim rules out, and the generated modules no longer carry one
# to repeat (bzl-ti9).
set -uo pipefail

root_module_bazel="$1"

if [[ ! -s "${root_module_bazel}" ]]; then
  echo "FAIL: root MODULE.bazel is missing or empty: ${root_module_bazel}" >&2
  exit 1
fi

status=0

if grep -Eq '^bazel_dep\(name = "rules_cc", ' "${root_module_bazel}"; then
  echo "FAIL: root MODULE.bazel has a direct bazel_dep on rules_cc. That puts" >&2
  echo "      rules_cc's host-toolchain registration ahead of the fixtures'" >&2
  echo "      llvm one and the whole workspace compiles with the host gcc." >&2
  echo "      See validation_workspace.bzl." >&2
  status=1
fi

# Only an override that names rules_cc: the root legitimately carries one
# local_path_override per fixture, which is how the fixtures get in at all.
if sed -n '/^[a-z_]*_override($/,/^)$/p' "${root_module_bazel}" | grep -q 'module_name = "rules_cc"'; then
  echo "FAIL: root MODULE.bazel overrides rules_cc. It must resolve from the" >&2
  echo "      registry like every module the tarball needs, except cc_config," >&2
  echo "      which is supplied by --override_module on the invocation (see" >&2
  echo "      the note in the file itself)." >&2
  status=1
fi

exit "${status}"
