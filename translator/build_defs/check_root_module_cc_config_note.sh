#!/usr/bin/env bash
# Pins the cc_config decision recorded in validation_workspace.bzl: the
# generated root MODULE.bazel must NOT resolve cc_config itself, and must say
# so in a comment that ships in the unpacked workspace.
#
# Both halves matter, so both are asserted. Without the negative, someone
# "fixes" the unpacked module-resolution error by writing an override here and
# silently bakes a checkout path into the deliverable — the exact edit this
# repo already attempted once (bzl-fxa.15). Without the positive, the note
# rots away and the next reader hits a bare Bazel error with no pointer.
set -uo pipefail

module_bazel="$1"

if [[ ! -s "${module_bazel}" ]]; then
  echo "FAIL: root MODULE.bazel is missing or empty: ${module_bazel}" >&2
  exit 1
fi

status=0

# The negative: an override for cc_config would make the tarball non-portable.
# Match a real declaration, not the word inside the explanatory comment.
if grep -Eq '^[^#]*(bazel_dep|local_path_override|module_name|name)[^#]*cc_config' "${module_bazel}"; then
  echo "FAIL: root MODULE.bazel declares cc_config, but it must be supplied" >&2
  echo "      via --override_module on the validation invocation so the" >&2
  echo "      tarball stays path-free. See validation_workspace.bzl." >&2
  echo "--- offending lines ---" >&2
  grep -En '^[^#]*(bazel_dep|local_path_override|module_name|name)[^#]*cc_config' "${module_bazel}" >&2
  status=1
fi

# The positive: the pointer has to be readable from the unpacked root.
if ! grep -q -- '--override_module=cc_config=' "${module_bazel}"; then
  echo "FAIL: root MODULE.bazel has no cc_config note. It must carry the" >&2
  echo "      --override_module=cc_config= invocation, because that error" >&2
  echo "      surfaces where this repo's docs are not in view." >&2
  echo "--- actual MODULE.bazel header ---" >&2
  head -30 "${module_bazel}" >&2
  status=1
fi

exit "${status}"
