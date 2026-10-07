# Resolution scripts

The agent stage's resolutions are ephemeral by design (bzl-b9b): they live in
the unpacked validation workspace and a re-conversion discards them. The
resolve-escalations skill keeps each one as a SCRIPT applied to a fresh
unpack, and until PMIx those scripts lived in a session's scratchpad.

PMIx changed that: it is the first dependent measured after the agent stage,
and its workspace needs hwloc's and libevent's resolutions applied too — by
a later session, on a workspace the sweep unpacks pre-agent (bzl-7r9.17). A
script nobody can find is not a resolution anyone can reproduce, so they are
kept here, per project, with the adapted copies of the dependencies' scripts
a dependent needs. This is a holding place, not the design: where these
belong for good is bzl-7r9.17's question.

Usage, for a workspace `sweep.py --post-agent pmix --workspace W` unpacked:

    python3 tools/resolutions/pmix/deps/resolve_hwloc.py    W/fixtures/hwloc
    python3 tools/resolutions/pmix/deps/resolve_libevent.py W/fixtures/libevent
    python3 tools/resolutions/pmix/resolve_pmix.py          W/fixtures/pmix

Each is all-or-nothing on its anchors; the dependencies' copies spell
"undefined" as `""`, which the config-header value change of 2026-09-19
requires (their originals, written before it, used `0`).

## Every real corpus project

Since 2026-10-06 every real corpus project's agent stage is kept here, one
script per project, each applied to a FRESH unpack (`W` below) and measured
with `sweep.py --post-agent <project> --workspace W`:

    python3 tools/resolutions/fmt/resolve_fmt.py                     W/fixtures/fmt
    python3 tools/resolutions/zlib/resolve_zlib.py                   W/fixtures/zlib
    python3 tools/resolutions/expat/resolve_expat.py                 W/fixtures/expat
    python3 tools/resolutions/xz/resolve_xz.py                       W/fixtures/xz
    python3 tools/resolutions/jansson/resolve_jansson.py             W/fixtures/jansson
    python3 tools/resolutions/libmicrohttpd/resolve_libmicrohttpd.py W/fixtures/libmicrohttpd
    python3 tools/resolutions/libidn2/resolve_libidn2.py             W/fixtures/libidn2
    python3 tools/resolutions/json-c/resolve_json_c.py               W/fixtures/json-c

tinyxml2 converts with no escalations and has no script. libmicrohttpd
depends on the converted zlib module, so it needs zlib in the same
workspace (it always is: the validation workspace carries every fixture).

Several of these also fix things NO item asked about — the translator
dropped them silently, and each is a filed bead rather than a resolution
anyone should expect to keep writing: automake tests run from the wrong
directory, absolute conversion-host paths in `-DSRCDIR`, shared-library
link flags (version scripts, `-export-symbols-regex`, libtool's `-DPIC`),
gnulib `sys/` headers emitted at the wrong path, and configure-generated
scripts (xz's `xzdiff`/`xzgrep`). Each script says which, where it does it.
