#!/usr/bin/env bash
# Runs a test script that hard-codes the autotools build layout — libevent's
# test/test.sh drives test/regress and its sibling programs from the
# directory it sits in — under Bazel, by building that layout in a scratch
# directory: every binary and script the test names is symlinked (bin=) or
# @var@-substituted (gen=) to where automake would have put it, and the
# module's runfiles stand in for the source tree. Then the script runs with
# the arguments automake's recipe passes.
#
#   run_layout_script_test.sh <script> [bin=<layout path>=<rootpath>]...
#                             [gen=<layout path>=<rootpath of .in>]... [-- args]
# In the trailing args, `@build@` stands for the scratch layout's root and
# `@empty@` for an empty argument (automake's `-b ""`), which does not
# survive the trip through a rule's args otherwise.
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
      -e "s#@HWLOC_VERSION@#unused#g" -e "s#@prefix@#$build#g" -e "s#@exec_prefix@#$build#g" \
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
    --)    shift; for a in "$@"; do a="${a//@build@/$build}"; [ "$a" = "@empty@" ] && a=""; args+=("$a"); done; break ;;
    *)     echo "unknown argument $1"; exit 2 ;;
  esac
  shift
done
script="$build/$(basename "$script_in" .in)"
subst "$script_in" "$script"
export LANG=C LC_ALL=C HWLOC_PLUGINS_PATH=/nonexistent
# TRACE_SCRIPT=1 in the test environment traces the script: most of them run
# under `set -e` and die without saying which command failed.
cd "$build" && if [ -n "${TRACE_SCRIPT:-}" ]; then sh -x "$script" "${args[@]}"; else "$script" "${args[@]}"; fi
