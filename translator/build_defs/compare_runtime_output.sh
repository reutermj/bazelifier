#!/usr/bin/env bash
# Gates on unresolved needs_attention/ items, then compares the
# ground-truth (real cmake+ninja) binary's runtime behavior against the
# Bazel-built binary for the same CMake target: stdout, stderr, and exit
# code must match. See docs/architecture/build-verification.md — this is
# NOT a binary-identical check, only a behavioral-equivalence check.
#
# Each binary is run twice so a line that varies between a binary's OWN two
# runs (a nondeterministic diagnostic like json_parse's "maxrss: <N> KB") can
# be excluded from the ground-truth-vs-Bazel comparison without weakening it
# for a deterministic binary — see compare_stream below and bzl-fxa.12.
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

# MANIFEST is always written by the translator (see
# main::write_needs_attention), even with zero items, unlike the *.md glob
# it sits alongside. A missing MANIFEST can therefore only mean the
# runfiles path above doesn't actually resolve to the fixture's real
# needs_attention/ directory (stale module_name, changed canonical repo
# naming, etc.) — never a clean conversion. Gating existence on the
# directory itself was tried first and doesn't work: Bazel drops an empty
# `data` filegroup from runfiles entirely rather than leaving an empty
# directory, so "wiring is broken" and "zero items" were indistinguishable.
if [[ ! -f "${needs_attention_dir}/MANIFEST" ]]; then
  echo "FAIL: no needs_attention MANIFEST at ${needs_attention_dir} — wiring is broken (stale module_name, changed canonical repo naming, ...), not a clean conversion."
  exit 1
fi

needs_attention_files=()
for f in "${needs_attention_dir}"/*.md; do
  [[ -e "${f}" ]] && needs_attention_files+=("${f}")
done

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

# A dynamically linked ground-truth binary (e.g. json-c's json_parse against
# libjson-c.so.5) loads its shared library by SONAME at run time, and the
# absolute RUNPATH CMake baked in is dead by now. copy_ground_truth_artifacts
# stages the .so chain at the ground_truth/ ROOT, but a binary from a CMake
# subdirectory lands in a subdir of it (json-c: libs at ground_truth/, binary
# at ground_truth/apps/json_parse), so the binary's own directory is NOT
# necessarily where the loader must look — assuming it was, which held while
# every fixture was flat, made the ground-truth run die at exit 127 before
# main. Search both: the binary's dir and every ancestor up to the
# ground_truth/ root. Scoped to the ground-truth run — the Bazel binary links
# the library statically and is self-contained. See bzl-fxa.11, bzl-0x7 and
# docs/architecture/build-verification.md.
ground_truth_dir="$(cd "$(dirname "${ground_truth_bin}")" && pwd)"
ground_truth_lib_path="${ground_truth_dir}"
probe_dir="${ground_truth_dir}"
while [[ "${probe_dir}" == */* && "$(basename "${probe_dir}")" != "ground_truth" ]]; do
  probe_dir="$(dirname "${probe_dir}")"
  ground_truth_lib_path="${ground_truth_lib_path}:${probe_dir}"
  [[ "${probe_dir}" == "/" ]] && break
done

# Each binary is run TWICE. Comparing a binary's two runs to each other tells us
# which output lines are nondeterministic (json-c's json_parse prints
# "maxrss: <ru_maxrss> KB" to stderr, and ru_maxrss varies between runs and
# between the two builds), so those lines can be excluded from the ground-truth
# vs Bazel comparison WITHOUT weakening it for a deterministic binary — one
# whose two runs are identical has nothing excluded, so the comparison stays
# byte-exact. This is self-calibrating per binary: no per-fixture config, no
# global relaxation. See bzl-fxa.12 and docs/architecture/build-verification.md.
#
# Each binary's OWN PATH is rewritten to a sentinel on the way out. A program
# that reports errors as `<argv[0]>: message` (xz's lzmainfo, json-c's
# test_util_file) necessarily prints a different string from each build, since
# the two binaries are at different paths by construction — that is a fact
# about where Bazel put them, not a behavioural difference, and it is the one
# difference this comparison must not report. Deliberately NOT a general path
# filter: only the invoked binary's own path is masked, so a genuine path in
# the output still has to match. See bzl-fxa.19.
mask_self() {
  # Both the full path and the basename, because a program may print either
  # (xz uses argv[0] verbatim; some use basename(argv[0])).
  sed -e "s|$1|<SELF>|g" -e "s|\b$(basename "$1")\b|<SELF>|g"
}
run_gt() {
  local rc
  LD_LIBRARY_PATH="${ground_truth_lib_path}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
    "${ground_truth_bin}" 2>"$2.raw" >"$1.raw"
  # Captured immediately: the caller reads $? from this function, and the
  # masking below would otherwise overwrite it with sed's exit code — turning
  # every nonzero program exit into 0 and silently disabling the exit-code
  # comparison, the one check here that needs no diffing to be right.
  rc=$?
  mask_self "${ground_truth_bin}" <"$1.raw" >"$1"
  mask_self "${ground_truth_bin}" <"$2.raw" >"$2"
  return "${rc}"
}
run_bazel() {
  local rc
  "${bazel_bin}" 2>"$2.raw" >"$1.raw"
  rc=$?
  mask_self "${bazel_bin}" <"$1.raw" >"$1"
  mask_self "${bazel_bin}" <"$2.raw" >"$2"
  return "${rc}"
}

run_gt "${tmpdir}/gt_out_a" "${tmpdir}/gt_err_a"; ground_truth_exit=$?
run_gt "${tmpdir}/gt_out_b" "${tmpdir}/gt_err_b"; ground_truth_exit_b=$?
run_bazel "${tmpdir}/bz_out_a" "${tmpdir}/bz_err_a"; bazel_exit=$?
run_bazel "${tmpdir}/bz_out_b" "${tmpdir}/bz_err_b"; bazel_exit_b=$?

status=0

# A binary whose two runs disagree on exit code is nondeterministic in a way
# this methodology can't reconcile — surface it rather than compare noise.
if [[ "${ground_truth_exit}" -ne "${ground_truth_exit_b}" ]]; then
  echo "FAIL: ground-truth exit code is nondeterministic (${ground_truth_exit} vs ${ground_truth_exit_b})"
  status=1
fi
if [[ "${bazel_exit}" -ne "${bazel_exit_b}" ]]; then
  echo "FAIL: Bazel exit code is nondeterministic (${bazel_exit} vs ${bazel_exit_b})"
  status=1
fi

if [[ "${ground_truth_exit}" -ne "${bazel_exit}" ]]; then
  echo "FAIL: exit code mismatch: ground_truth=${ground_truth_exit} bazel=${bazel_exit}"
  status=1
fi

# Masks the lines that vary between a binary's own two runs (a_file vs b_file)
# to a sentinel, in BOTH the ground-truth and Bazel copies of the same stream,
# then diffs. The masks are unioned: a line nondeterministic in EITHER binary is
# excluded from both, so a value that happens to be stable across the ground
# truth's two runs but varies across Bazel's is still not required to match. A
# line-count difference between a binary's own two runs means the nondeterminism
# is structural (lines added/removed, not just a value), which the line-indexed
# mask can't model — that is failed loudly rather than masked.
compare_stream() {
  local label="$1" gt_a="$2" gt_b="$3" bz_a="$4" bz_b="$5"

  local n_gt_a n_gt_b n_bz_a n_bz_b
  n_gt_a=$(wc -l <"${gt_a}"); n_gt_b=$(wc -l <"${gt_b}")
  n_bz_a=$(wc -l <"${bz_a}"); n_bz_b=$(wc -l <"${bz_b}")
  if [[ "${n_gt_a}" -ne "${n_gt_b}" ]]; then
    echo "FAIL: ground-truth ${label} line count is nondeterministic (${n_gt_a} vs ${n_gt_b})"
    return 1
  fi
  if [[ "${n_bz_a}" -ne "${n_bz_b}" ]]; then
    echo "FAIL: Bazel ${label} line count is nondeterministic (${n_bz_a} vs ${n_bz_b})"
    return 1
  fi

  # Line numbers (1-based) that differ between the two runs of either binary.
  local nondeterministic
  nondeterministic="$(
    {
      diff --unchanged-line-format= --old-line-format='%dn
' --new-line-format= "${gt_a}" "${gt_b}"
      diff --unchanged-line-format= --old-line-format='%dn
' --new-line-format= "${bz_a}" "${bz_b}"
    } | sort -un
  )"

  # Replace those line numbers with a sentinel in both streams, then diff.
  local masked_gt masked_bz
  masked_gt="${tmpdir}/masked_gt_${label}"
  masked_bz="${tmpdir}/masked_bz_${label}"
  awk -v drop="${nondeterministic}" '
    BEGIN { n = split(drop, a, "\n"); for (i = 1; i <= n; i++) if (a[i] != "") d[a[i]] = 1 }
    { print (FNR in d) ? "<nondeterministic>" : $0 }
  ' "${gt_a}" >"${masked_gt}"
  awk -v drop="${nondeterministic}" '
    BEGIN { n = split(drop, a, "\n"); for (i = 1; i <= n; i++) if (a[i] != "") d[a[i]] = 1 }
    { print (FNR in d) ? "<nondeterministic>" : $0 }
  ' "${bz_a}" >"${masked_bz}"

  if ! diff -q "${masked_gt}" "${masked_bz}" >/dev/null; then
    echo "FAIL: ${label} mismatch (nondeterministic lines already excluded):"
    diff "${masked_gt}" "${masked_bz}" || true
    return 1
  fi
  return 0
}

compare_stream stdout "${tmpdir}/gt_out_a" "${tmpdir}/gt_out_b" "${tmpdir}/bz_out_a" "${tmpdir}/bz_out_b" || status=1
compare_stream stderr "${tmpdir}/gt_err_a" "${tmpdir}/gt_err_b" "${tmpdir}/bz_err_a" "${tmpdir}/bz_err_b" || status=1

if [[ "${status}" -eq 0 ]]; then
  echo "PASS: ${ground_truth_bin} and ${bazel_bin} behave equivalently"
fi

exit "${status}"
