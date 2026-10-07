#!/usr/bin/env bash
# Runs one of hwloc's configure-generated test scripts (utils/hwloc/*.sh.in,
# utils/lstopo/*.sh.in, utils/hwloc/test-hwloc-dump-hwdata/*.sh.in) under
# Bazel. Those scripts hard-code the autotools layout — the binary at
# $HWLOC_top_builddir/utils/hwloc/hwloc-calc, data at $HWLOC_top_srcdir/
# tests/hwloc/xml — so this builds that layout in a scratch directory: the
# source tree is the module's own runfiles (the test's working directory),
# and every binary the script names is symlinked to where automake would
# have put it. Then it does what configure does to the .in — substitutes
# the @vars@ — and runs the result with the arguments automake would pass.
#
#   run_hwloc_script_test.sh <script.sh.in> [bin=<layout path>=<rootpath>]...
#                            [gen=<layout path>=<rootpath of .in>]... [-- args]
# In the trailing args, `@build@` stands for the scratch layout's root, so a
# LOG_COMPILER wrapper can be handed the binary at its automake path.
# `gen=` is for a script the build itself generates from a .in (hwloc's
# hwloc-compress-dir, a bin_SCRIPT): it is substituted the same way and
# placed in the layout.
set -u
script_in="$1"; shift
# The module's runfiles root: this runner ships at the module root, so its
# own directory is where every $(rootpath) argument resolves from. The test's
# working directory is NOT it for a module consumed as a dependency.
src_root="$(cd "$(dirname "$0")" && pwd)"
build="$(mktemp -d)"
trap 'rm -rf "$build"' EXIT
mkdir -p "$build/hwloc/.libs"
subst() {
  sed -e "s#@HWLOC_top_srcdir@#$src_root#g" -e "s#@HWLOC_top_builddir@#$build#g" \
      -e "s#@DIFF@#diff#g" -e "s#@HWLOC_DIFF_U@#-u#g" -e "s#@HWLOC_DIFF_W@#-w#g" \
      -e "s#@HWLOC_XML_LOCALIZED@#1#g" -e "s#@XMLLINT@##g" -e "s#@EXEEXT@##g" \
      -e "s#@HWLOC_VERSION@#2.7.1#g" -e "s#@prefix@#$build#g" -e "s#@exec_prefix@#$build#g" \
      -e "s#@bindir@#$build/utils/hwloc#g" "$1" > "$2"
  chmod +x "$2"
}
args=()
while [ $# -gt 0 ]; do
  case "$1" in
    bin=*) spec="${1#bin=}"; rel="${spec%%=*}"; path="${spec#*=}"
           mkdir -p "$build/$(dirname "$rel")"
           ln -s "$src_root/$path" "$build/$rel" ;;
    gen=*) spec="${1#gen=}"; rel="${spec%%=*}"; path="${spec#*=}"
           mkdir -p "$build/$(dirname "$rel")"
           subst "$src_root/$path" "$build/$rel" ;;
    --)    shift; for a in "$@"; do args+=("${a//@build@/$build}"); done; break ;;
    *)     echo "unknown argument $1"; exit 2 ;;
  esac
  shift
done
script="$build/$(basename "$script_in" .in)"
subst "$script_in" "$script"
export LANG=C LC_ALL=C HWLOC_PLUGINS_PATH=/nonexistent
# HWLOC_TRACE=1 in the test environment traces the script: most of them run
# under `set -e` and die without saying which command failed.
cd "$build" && if [ -n "${HWLOC_TRACE:-}" ]; then sh -x "$script" "${args[@]}"; else "$script" "${args[@]}"; fi
