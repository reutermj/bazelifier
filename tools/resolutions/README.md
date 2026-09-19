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
