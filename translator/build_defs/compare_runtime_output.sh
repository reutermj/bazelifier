#!/usr/bin/env bash
# Gates on unresolved needs_attention/ items, then compares the
# ground-truth (real cmake+ninja) binary's runtime behavior against the
# Bazel-built binary for the same CMake target: stdout, stderr, and exit
# code must match. See docs/architecture/build-verification.md — this is
# NOT a binary-identical check, only a behavioral-equivalence check.
#
# needs_attention/ means the translator could not confidently resolve
# something for THIS conversion (see
# docs/architecture/needs-attention-interface.md) — the pipeline's agent
# stage triages and resolves it, by editing the generated BUILD.bazel here
# in the unpacked workspace, before this comparison is meaningful (a
# conversion can happen to still build/run despite an open needs_attention
# item, which would otherwise mask the real gap). So this script fails
# loud, printing the item(s) to fix, rather than silently running the
# comparison anyway.
#
# A failure here is an UNFINISHED conversion to be resolved and re-run, not
# an expected steady state. Resolutions go in the generated output; the
# source CMakeLists.txt is immutable input and is never edited to make a
# conversion pass. See docs/architecture/build-verification.md.
#
# NOTE the deliberate absence of `-e`: a nonzero exit from either binary is
# data this script compares, not a reason to stop, and `diff -q` reporting a
# difference is likewise. Under `set -e` the first such case would kill the
# script before it could report anything, and a ground truth that exits
# nonzero would never be comparable at all. Failures are surfaced by
# accumulating `status` instead.
set -uo pipefail

bazel_bin="$1"
ground_truth_bin="$2"
# The needs_attention/ dir's runfiles-relative path (e.g.
# "with_library+/needs_attention"), NOT a $(locations) expansion — Bazel's
# $(locations) can't expand to zero files, which needs_attention/ often
# legitimately does. @module//needs_attention:all is still a `data` dep
# (see validation_workspace.bzl) so its files land in runfiles; found here
# via the test's own runfiles root instead.
needs_attention_relative_dir="$3"

runfiles_root="${TEST_SRCDIR:-$0.runfiles}"
needs_attention_dir="${runfiles_root}/${needs_attention_relative_dir}"
needs_attention_files=()
if [[ -d "${needs_attention_dir}" ]]; then
  for f in "${needs_attention_dir}"/*.md; do
    [[ -e "${f}" ]] && needs_attention_files+=("${f}")
  done
fi

if [[ "${#needs_attention_files[@]}" -gt 0 ]]; then
  echo "FAIL: unresolved needs_attention item(s) — triage these before validating:"
  echo
  for f in "${needs_attention_files[@]}"; do
    echo "===== ${f} ====="
    cat "${f}"
    echo
  done
  exit 1
fi

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
