# Post-agent-stage modules

bazelifier's real corpus projects as converted Bazel modules AFTER the agent
stage resolved every `needs_attention/` item: what each module looks like
when it is green, with the sources it builds from. Not the fixtures.

Each was converted by the translator at `716d419` and resolved by its script
in `tools/resolutions/` on `main` (`f7b981a`), in one unpack of the
validation workspace in which all fourteen were measured green:

| Project       | Comparisons             | Module tests |
|---------------|-------------------------|--------------|
| hwloc         | 19/19                   | 180/180      |
| libevent      | 19/19 (4 omitted)       | 12/12        |
| pmix          | 40/40 (1 omitted)       | 19/19        |
| prrte         | 5/5 (1 omitted)         | 4/4          |
| openmpi       | 13/13 (3 omitted)       | 68/68        |
| xz            | 10/10                   | 21/21        |
| expat         | 5/5                     | 3/3          |
| jansson       | 19/19                   | 3/3          |
| libmicrohttpd | 30/30 (1 omitted)       | 46/46        |
| libidn2       | 6/6                     | 44/44        |
| zlib          | 2/2                     | 6/6          |
| json-c        | 30/30                   | 31/31        |
| fmt           | 1/1                     | 21/21        |
| tinyxml2      | 0/0                     | 1/1          |

Each directory is the whole converted module as resolved: `BUILD.bazel`,
`MODULE.bazel`, `TARGETS` (the manifest, including the `omitted` records),
the test runner scripts, generated headers, `project_notes/`, and the
project's own sources as the translator copied them in. Left out:
`ground_truth/`, the binaries the project's own build produced for the
runtime comparisons (machine-specific, and regenerated on every conversion).
Sources the module does not compile are still here (Open MPI's `docs/`, its
bundled tarballs). The translator copies the project tree, and this is that
tree.

To build one, put it beside its dependencies and supply `cc_config`
(`--override_module=cc_config=<bazelifier>/cc_config`). libmicrohttpd needs
zlib, PMIx needs hwloc and libevent, PRRTE needs PMIx and its dependencies,
and Open MPI needs all four. The runtime comparisons need `ground_truth/`,
which only a fresh conversion produces.

## Known gaps in what is here

Green means every check the pipeline runs passed. These are things those
checks do not see yet, found while producing this snapshot and filed on
`main`:

- **Absolute conversion-host paths.** The generated `BUILD.bazel` of hwloc
  (`XMLTESTDIR`), libmicrohttpd (`DATA_DIR`), PMIx, PRRTE and Open MPI
  (`*_BUILD_CPPFLAGS`/`LDFLAGS`, `SRCDIR`) still carry `-D` defines that name
  a path on the conversion machine. libidn2's were rewritten by its script.
- **Shared-library exports differ from the original** for hwloc, PMIx,
  expat, libevent and Open MPI's `libmpi`: the translator does not carry
  libtool's version scripts and export lists, and no check compares exported
  symbols yet. xz and jansson were fixed by their scripts and match.

This branch has no history in common with `main`, on purpose: it is an
output snapshot, regenerated rather than merged.
