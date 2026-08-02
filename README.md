# bazelifier

bazelifier converts existing build scripts into standalone Bazel modules —
a project's own `MODULE.bazel` and `BUILD.bazel`, ready to check into that
project's own repo with no dependency on bazelifier itself. It pairs a
deterministic translator with AI-agent assistance for the cases the
translator can't handle mechanically.

Two build systems are supported: **CMake** (via the CMake File API) and
**Autotools** (autoconf + automake + libtool, via `make`'s own resolved
command stream). Both read into one build-system-neutral model that a single
code generator renders, so adding a third (Make, Meson, ...) means adding a
frontend, not a second pipeline.

## Why two stages

A build translation is not one problem. Part of it is mechanical — a
library's sources, its include paths, which targets link which — and a
program can get that exactly right, every time, from what the build system
itself reports. The rest is judgement about a particular project: what a
`UTILITY` target is actually for, whether a macro named `JSON_C_HAVE_STDINT_H`
means what `HAVE_STDINT_H` means, whether a test script's working directory
matters. That part does not have one correct answer derivable from the
inputs, and pretending otherwise produces a translator that guesses
confidently and wrongly.

So the pipeline splits along that line, deliberately:

**Stage one is deterministic and refuses to guess.** The same commit and the
same project give the same output, byte for byte. When the translator cannot
determine something with confidence, it does not fall back to a heuristic —
it escalates, writing a structured description of the gap into the
conversion's own output. An escalation is the translator being honest about
the boundary of what it can know.

**Stage two is an agent, and is not reproducible.** It reads those items and
decides, using knowledge of the project that no amount of static analysis
would supply. Two runs may resolve the same item differently and both be
correct. That is not a defect to engineer away; it is the shape of the
problem, and a pipeline that could only handle the deterministic half would
convert almost nothing real.

**What holds it together is validation, not reproducibility.** The contract
is a concrete set of checks a conversion must pass — the module builds with
no reference back to this repo, its binaries behave identically to the
originals, and its own tests pass. Those are objective, and they are what
the project iterates against. The *process* is allowed to vary; the *result*
is not.

One consequence worth stating plainly: resolutions are **ephemeral** by
design. They are not cached and replayed on the next conversion. Replaying
them would make a re-run look green without the agent stage having engaged
with what changed — which is precisely the thing being tested.

## How it works

1. **Deterministic translator** — discovers a project's targets by asking
   its build system (the CMake File API; the build's own command output and
   `make -p` for Autotools)
   and mechanically emits a **standalone Bazel module** for it (its own
   `MODULE.bazel` + `BUILD.bazel`, copied sources) for the patterns it
   recognizes. It also runs the project's real build to capture ground-truth
   artifacts for verification.
2. **Agent stage** — when the translator hits something it doesn't know how
   to handle (an unsupported generator expression, a custom command, an
   unusual dependency shape), it writes a **`needs_attention/` item**: a
   structured description of the gap, placed in that conversion's own
   output. An AI coding agent (e.g. Claude Code) reads the item and
   provides the missing translation, which feeds back into the pipeline.
   This is a stage of the pipeline, not a fallback beside it — the thing
   being built and tested is "translator + agent," and an unresolved gap
   means the conversion isn't finished.
3. **Independence + equivalence verification** — a conversion is only
   considered successful once the generated module builds with **no
   reference back to bazelifier's own workspace** (verified by packaging it
   into a tarball, unpacking it completely outside this repo, and building
   from there) *and* behaves equivalently to the original build (not
   necessarily binary-identical — currently a runtime output comparison
   against the captured ground truth). See
   [docs/architecture/build-verification.md](docs/architecture/build-verification.md).

A conversion is always resolved by changing what bazelifier **emits**,
never by editing the project's own build files. Converting a build system
involves judgement calls at many points; the equivalence checks are the
contract, not reproducibility of the process.

## Status

Early stage / prototype. Validation uses small, synthetic ("unit") projects
built specifically to exercise the translator (TDD-style), plus a corpus of
real open-source projects — currently tinyxml2, zlib, fmt, json-c (CMake) and
xz (Autotools).

**[Pipeline metrics →](https://markreuter.dev/bazelifier/metrics/)** Open
escalations, targets and tests across the whole corpus, tracked over time.

Those numbers are *pre-agent*: what the deterministic translator produced on
its own. An open escalation is unfinished work awaiting the agent stage, not a
defect — see [how it works](#how-it-works) above. Generate the report locally
with:

```sh
python3 tools/sweep/sweep.py --append metrics/history.jsonl
python3 tools/sweep/report.py metrics/history.jsonl
```

## Documentation

- [CLAUDE.md](CLAUDE.md) — project guide for AI agents working in this repo
  (also available as [AGENTS.md](AGENTS.md))
- [docs/architecture/](docs/architecture/) — design and component docs,
  including the
  [translator → agent handoff format](docs/architecture/needs-attention-interface.md)
- [docs/runbooks/](docs/runbooks/) — maintenance procedures for this repo
- [docs/lore/](docs/lore/) — non-obvious discoveries and hard-won context
  that isn't captured elsewhere
- [metrics/](metrics/) — the sweep's history and how to read it

## Scope

- **In scope (now):** CMake → Bazel and Autotools → Bazel conversion, for
  C/C++ projects.
- **In scope (future):** further build systems (Make, Meson, etc.), broader
  language support, hermetic/remote-execution-friendly builds.
- **Out of scope (for now):** anything not related to translating a build
  system's build graph into Bazel.

## License

See [LICENSE](LICENSE).
