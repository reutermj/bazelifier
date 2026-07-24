#!/usr/bin/env bash
# Compares the ground-truth (real cmake+ninja) binary's runtime behavior
# against the Bazel-built binary for the same CMake target: stdout,
# stderr, and exit code must match. See
# docs/architecture/build-verification.md — this is NOT a binary-identical
# check, only a behavioral-equivalence check.
set -uo pipefail

ground_truth_bin="$1"
bazel_bin="$2"

tmpdir="${TEST_TMPDIR:-.}"
ground_truth_stderr="${tmpdir}/ground_truth_stderr"
bazel_stderr="${tmpdir}/bazel_stderr"

ground_truth_stdout="$("${ground_truth_bin}" 2>"${ground_truth_stderr}")"
ground_truth_exit=$?
bazel_stdout="$("${bazel_bin}" 2>"${bazel_stderr}")"
bazel_exit=$?

status=0

if [[ "${ground_truth_exit}" -ne "${bazel_exit}" ]]; then
  echo "FAIL: exit code mismatch: ground_truth=${ground_truth_exit} bazel=${bazel_exit}"
  status=1
fi

if [[ "${ground_truth_stdout}" != "${bazel_stdout}" ]]; then
  echo "FAIL: stdout mismatch:"
  echo "  ground_truth: ${ground_truth_stdout}"
  echo "  bazel:        ${bazel_stdout}"
  status=1
fi

if ! diff -q "${ground_truth_stderr}" "${bazel_stderr}" > /dev/null; then
  echo "FAIL: stderr mismatch:"
  diff "${ground_truth_stderr}" "${bazel_stderr}" || true
  status=1
fi

if [[ "${status}" -eq 0 ]]; then
  echo "PASS: ${ground_truth_bin} and ${bazel_bin} behave equivalently"
fi

exit "${status}"
