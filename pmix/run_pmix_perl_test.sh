#!/usr/bin/env bash
# Runs one of PMIx's registered perl test drivers (test/run_testsNN.pl.in).
#
# Upstream, configure substitutes @PMIX_BUILT_TEST_PREFIX@ (the build tree)
# and @PMIX_COMPONENT_LIBRARY_PATHS@ (the .libs dirs of DSO components) into
# each driver, and `make check` runs it from test/. The driver chdirs into
# <prefix>/test, derives its test number from its own file name and runs
# `./pmix_test <args>`; pmix_test then execs the client it finds through its
# OWN argv[0]: with argv[0] = "./pmix_test" that is "./../pmix_client", one
# directory UP from test/ (test_common.c, written for libtool's .libs/
# layout). So the layout is rebuilt in a scratch tree: the three binaries
# reachable as <prefix>/test/pmix_* and pmix_client also as
# <prefix>/pmix_client, the driver rendered with the prefix, and no
# component path at all — every MCA component in this module is static.
#
# Not `set -e`: the driver's exit status is the result.
set -uo pipefail

driver="$1"  # run_testsNN
module_runfiles="$(cd "$(dirname "$0")" && pwd)"
prefix="${TEST_TMPDIR:-/tmp}/pmix-$driver"
rm -rf "$prefix"
mkdir -p "$prefix/test"
for bin in pmix_test pmix_client pmix_regex; do
    ln -s "$module_runfiles/$bin" "$prefix/test/$bin"
done
ln -s "$module_runfiles/pmix_client" "$prefix/pmix_client"
sed -e "s|@PMIX_BUILT_TEST_PREFIX@|$prefix|g" \
    -e "s|@PMIX_COMPONENT_LIBRARY_PATHS@||g" \
    "$module_runfiles/test/$driver.pl.in" > "$prefix/test/$driver.pl"
cd "$prefix/test" || exit 1
exec perl "./$driver.pl"
