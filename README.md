# Post-agent-stage build files

The Bazel build files of bazelifier's real corpus projects AFTER the agent
stage resolved every `needs_attention/` item — what the converted modules
look like when they are green, with the sources they build from. Not the
fixtures.

Generated from `main` at `5af0cd5` (translator at `716d419`): each project
converted by the translator, then resolved by its scripts in
`tools/resolutions/` (applied in dependency order: hwloc, libevent, PMIx,
PRRTE, Open MPI), from one fresh unpack of the validation workspace in which
all five were measured green:

| Project  | Comparisons             | Module tests |
|----------|-------------------------|--------------|
| hwloc    | 19/19                   | 180/180      |
| libevent | 19/19 (4 omitted)       | 12/12        |
| pmix     | 40/40 (1 omitted)       | 19/19        |
| prrte    | 5/5 (1 omitted)         | 4/4          |
| openmpi  | 13/13 (3 omitted)       | 68/68        |

Each directory is the whole converted module as resolved: `BUILD.bazel`,
`MODULE.bazel`, `TARGETS` (the manifest, including the `omitted` records),
the test runner scripts, generated headers, `project_notes/`, and the
project's own sources as the translator copied them in. Left out:
`ground_truth/`, the binaries the project's own build produced for the
runtime comparisons (machine-specific, and regenerated on every conversion).
Sources the module does not compile are still here (Open MPI's `docs/`, its
bundled tarballs) — the translator copies the project tree, and this is that
tree.

To build one, put it beside its dependencies and supply `cc_config`
(`--override_module=cc_config=<bazelifier>/cc_config`); the runtime
comparisons need `ground_truth/`, which only a fresh conversion produces.

Only these five: their resolutions are kept as scripts and can be
regenerated. The other corpus projects (xz, expat, jansson, libmicrohttpd,
libidn2, zlib, json-c, fmt, tinyxml2) were resolved in throwaway workspaces
and have no stored post-agent state.

This branch has no history in common with `main`, on purpose: it is an
output snapshot, regenerated rather than merged.
