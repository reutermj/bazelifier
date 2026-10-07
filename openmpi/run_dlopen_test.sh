#!/usr/bin/env bash
# Runs ompi/debuggers/dlopen_test the way make check does: from the directory
# that holds the debugger message-queue plugin. The test reads
# ./libompi_dbg_msgq.la (a libtool control file; an empty dlname means a
# static build and it skips with 77) and then opal_dl_opens
# ./libompi_dbg_msgq — so both are staged here from the module's own build of
# the plugin, with the .la stating its dlname the way libtool writes it.
set -uo pipefail
module="$(cd "$(dirname "$0")" && pwd)"
work="${TEST_TMPDIR:-$(mktemp -d)}/debuggers"
mkdir -p "${work}/.libs"
cp "${module}/libompi_dbg_msgq.so" "${work}/libompi_dbg_msgq.so"
cp "${module}/libompi_dbg_msgq.so" "${work}/.libs/libompi_dbg_msgq.so"
cat > "${work}/libompi_dbg_msgq.la" <<'LA'
# libompi_dbg_msgq.la - a libtool library file
dlname='libompi_dbg_msgq.so'
library_names='libompi_dbg_msgq.so'
old_library=''
installed=no
LA
cd "${work}" && exec "${module}/dlopen_test"
