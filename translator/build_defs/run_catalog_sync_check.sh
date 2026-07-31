#!/usr/bin/env bash
# Fails if the translator's CATALOG_DEFINES drifts from the cc_config catalog.
#
# The catalog's set of probed defines is declared twice, deliberately: once in
# Starlark (cc_config/catalog/BUILD.bazel), which the Rust translator cannot
# read at codegen time, and once as CATALOG_DEFINES in
# translator/src/configure_file.rs. Two hand-maintained copies are acceptable only
# if drift is caught — and until bzl-2eu the checker existed but nothing ran
# it, which is weaker than a check that looks at nothing: it wasn't looking at
# all.
#
# Bash rather than reusing cc_config/check_catalog_sync.py: the sandbox has no
# python3 on PATH, and depending on the host interpreter would make this test's
# green depend on the machine. The logic is a regex over two files, which shell
# does natively. The Python version stays for running by hand from the repo
# root, and the two must agree — this one is the gate.
#
# Both inputs arrive as arguments because the sandbox path is not the
# source-tree path.
set -uo pipefail

catalog_build="$1"
rust_source="$2"

for f in "${catalog_build}" "${rust_source}"; do
  # A missing input would otherwise read as "no defines found", which looks
  # like drift rather than broken wiring.
  if [[ ! -s "${f}" ]]; then
    echo "FAIL: input missing or empty: ${f}" >&2
    exit 1
  fi
done

# Every catalog define is an uppercase HAVE_*/SIZEOF_* string literal, and the
# catalog has no other such literals, so matching those directly is reliable
# without parsing Starlark.
catalog_defines="$(grep -oE '"(HAVE_|SIZEOF_)[A-Z0-9_]+"' "${catalog_build}" |
  tr -d '"' | sort -u)"

# Scoped to the CATALOG_DEFINES array so an unrelated HAVE_* elsewhere in the
# frontend (an escalation string, a test fixture) doesn't count.
rust_defines="$(sed -n '/const CATALOG_DEFINES/,/\];/p' "${rust_source}" |
  grep -oE '"(HAVE_|SIZEOF_)[A-Z0-9_]+"' | tr -d '"' | sort -u)"

# An empty side means the extraction matched nothing — a checker looking at
# nothing, which would pass the moment BOTH sides broke. Neither list is ever
# legitimately empty.
if [[ -z "${catalog_defines}" ]]; then
  echo "FAIL: no defines found in ${catalog_build} — the extraction broke, not the catalog" >&2
  exit 1
fi
if [[ -z "${rust_defines}" ]]; then
  echo "FAIL: no defines found in CATALOG_DEFINES in ${rust_source} — the extraction broke" >&2
  exit 1
fi

only_catalog="$(comm -23 <(echo "${catalog_defines}") <(echo "${rust_defines}"))"
only_rust="$(comm -13 <(echo "${catalog_defines}") <(echo "${rust_defines}"))"

if [[ -z "${only_catalog}" && -z "${only_rust}" ]]; then
  echo "PASS: CATALOG_DEFINES and the catalog agree ($(echo "${catalog_defines}" | wc -l) defines)."
  exit 0
fi

echo "FAIL: the translator's CATALOG_DEFINES has drifted from the catalog." >&2
if [[ -n "${only_catalog}" ]]; then
  echo "  in the catalog but NOT CATALOG_DEFINES — the translator will escalate a" >&2
  echo "  macro that already has a working probe, burning an agent cycle:" >&2
  echo "${only_catalog}" | sed 's/^/    /' >&2
fi
if [[ -n "${only_rust}" ]]; then
  echo "  in CATALOG_DEFINES but NOT the catalog — the translator will emit" >&2
  echo "  @cc_config//catalog:<name> and the generated module will fail at ANALYSIS" >&2
  echo "  time in the unpacked workspace with 'no such target':" >&2
  echo "${only_rust}" | sed 's/^/    /' >&2
fi
exit 1
