# Retrospective: onboarding libevent and hwloc (2026-09-18 to 2026-09-19)

The first two nodes of the Open MPI chain (bzl-7r9). Both reached green
from the unpacked workspace. libevent took one working day including the
cross-module machinery it needed; hwloc took most of another, and five
full agent-stage cycles. This records what surfaced, what the process got
right, and what it got wrong, with the changes made in response.

## What surfaced

Sixteen translator gaps between the two projects. Twelve fixed, each with a
unit test on real-shaped input; four filed.

| # | Gap | Found by | Tier that caught it |
|---|---|---|---|
| 1 | Primaries declared through variables (`lib_LTLIBRARIES = $(LIBEVENT_LIBS_LA)`) converted to zero targets | libevent | conversion failed loudly ("NO build targets") |
| 2 | A make-time `sed`-generated header was silently absent | libevent | agent stage: compile error far from the cause |
| 3 | Installed public headers not exported at their installed path (three iterations: flat `include_HEADERS`, then `include_<x>dir`, then `includedir` expanded before its prefix was checked) | libevent, then hwloc | cross-module consumer; hwloc's `hdrs` inspected by eye |
| 4 | A program under two primaries rendered twice | libmicrohttpd, via the sweep | unpacked root failed to load |
| 5 | A source under a probe-decided automake conditional dropped when the host satisfied the probe (`strlcpy.c`) | libevent | agent stage: link error |
| 6 | A valued macro gated on a boolean flag frozen as `1` (`SIZEOF_PTHREAD_T`) | libevent | reading the generated header by hand; every gate was blind |
| 7 | Per-target variables read from the flattened database (`hwloc_bind_SOURCES` from two directories) | hwloc | conversion failed: copy of a file that did not exist |
| 8 | `make -p` scopes split on `Entering` only, so a parent's database went to its last child | hwloc | 67 spurious test entries, read by hand |
| 9 | Primaries expanded against the flattened map, and the first fix (refuse anything defined two ways) dropped exactly the targets that needed it (`shmem`) | hwloc | `make check` in scratch built binaries the module lacked |
| 10 | An undefined `$(am__EXEEXT_N)` escalated as unresolved instead of expanding to nothing | hwloc | test item read by hand |
| 11 | `local_defines` not shell-quoted; rules_cc ate the quotes (`RUNSTATEDIR`) | hwloc | agent stage: compile error; unused macros had hidden it since the first autotools project |
| 12 | Assertions forbade the `#undef` line of a value of `0` | hwloc | agent stage: the module's own assertion failed |
| 13 | `LOG_COMPILER` ignored: check programs run bare (bzl-dsu) | hwloc | agent stage: two tests failed |
| 14 | A flag paired with the wrong macro (`--enable-plugins` under `with_x`, bzl-iwo second instance) | hwloc | reading the generated `select()` by hand |
| 15 | Catalog facts the frontend had FROZEN never reached the unmapped list, so a harvest driven by that list missed them | hwloc | second catalog pass needed |
| 16 | The `unmapped_config_macros` item lists both frontends' names but the catalog harvest is manual | both | 167 catalog entries added by scripts written per project |

Plus one reversed decision: `--disable-pci` on hwloc also disables sysfs PCI
discovery, and upstream's own suite fails seven tests under it.

The instances that hurt most were not individually hard. **Gaps 7, 8, 9 and
10 — and bzl-oek before them — are one bug**: `parse_variables` flattens a
recursive project's per-directory databases into one map, and every consumer
of that map was wrong in a different way. It was fixed five times, once per
consumer, over three weeks. The model was the bug.

## What worked

- **Measuring upstream before trusting a failure.** Seven hwloc topology
  tests failed in the module; running hwloc's own `make check` under the same
  configure flags showed upstream fails the same seven. That turned "my runner
  is wrong" into "the flag is wrong" in one command, and it is the single
  most valuable move of the two days.
- **Both directions, at the lowest tier.** Every fix landed with a unit test
  that holds the negative too (the ambiguity guard's replacement, the
  `am__` rule, the assertion change). None regressed later.
- **The sweep after every change.** The per-directory work recovered
  libmicrohttpd's entire test suite (104 to 174 tests) and a fixture's
  conditional target as side effects; the sweep is how that was seen rather
  than discovered by the next onboarding.
- **Project notes for n=1 findings.** `strlcpy.c`, the regress binary,
  `--disable-pci`, `wrapper.sh`: each is a paragraph that ships with the
  module, and each would otherwise have been rediscovered by the next agent.
- **Committing per milestone.** libevent's pin, libevent green, hwloc green:
  three commits, each verified from the unpacked workspace against the
  previous run's pass/fail set.
- **Designing cross-module dependencies on a fixture pair first.** The
  sysroot mechanism was proven on eleven files before any corpus project
  used it, and neither leaf needed a change to it.
- **Bounding comparison runs.** Four of libevent's samples are servers;
  without the timeout they would have read as a hung harness.

## What went wrong

- **Fixing a bug class one instance at a time.** See above. The tell was
  visible early: `accumulates_across_directories`'s doc comment already
  argued with its own code. Rule now recorded in
  `autotools-frontend.md`: the third instance of a shape means the model,
  not the instance.
- **Reasoning from a flag's name.** `--disable-pci` was chosen because it
  sounded like `--disable-libxml2`. Reading the twenty lines of m4 it gates
  would have taken a minute; the wrong flag cost a full re-pin and re-cycle.
- **Runners written before reading every driver end to end.** The topology
  runner was drafted from three of the five drivers; the other two (`allowed`
  exports two environment variables; `x86` uses `-i --if cpuid`) each cost a
  cycle. The script runner resolved the module root from `$PWD`, which is
  wrong for a module consumed as a dependency; the generated
  `run_registered_test.sh` beside it already had the right answer.
- **Sidecar data and substitutions found one failure at a time.** `@EXEEXT@`,
  the annotate script's XML input, the `HWLOC_HAVE_PLUGINS` value dropped with
  its `select`: each a one-line fix, each a five-minute rebuild.
- **A resolution script that could not be re-applied.** Applying it twice
  double-edited the BUILD, so every retry meant restoring the module from the
  validation tree by hand. Resolutions are a script over a FRESH unpack, or
  they are idempotent; nothing in between.
- **Harvesting the catalog from the escalation rather than from configure.**
  The frontend had frozen ten declarations as values, so they were not in
  the item and not in the first harvest.
- **Edit scripts aborting on stale anchors** after `cargo fmt` reformatted
  the text they matched. All-or-nothing writes meant no file was ever left
  half-edited, which is the right failure mode, but it happened six times.

## Changes made

- `.claude/skills/onboard-project`: pin only after reading what each
  configure flag gates in the project's m4, and run upstream's own
  `make check` under the pinned flags before the agent stage; harvest
  catalog facts from `configure` itself.
- `.claude/skills/resolve-escalations`: read every test driver end to end
  before writing a runner; resolve the module root from the runner's own
  path; keep the resolution as a script applied to a fresh unpack; when a
  reproduced test fails, ask first whether upstream passes it.
- `docs/architecture/autotools-frontend.md`: the flattened-database bug
  class named, with its five instances, so the sixth is recognised.
- bzl-7r9.10: replace the flattened map with per-directory scopes as the
  frontend's one model.
- bzl-7r9.11: a `configure`-driven catalog harvester, so the next project's
  facts are one command rather than a script written per project.
