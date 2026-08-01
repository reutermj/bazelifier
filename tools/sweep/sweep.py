#!/usr/bin/env python3
"""Runs the conversion pipeline over every project and records what happened.

Two measurement points, because they answer different questions and neither
substitutes for the other:

  PRE-AGENT   what the translator alone produced. Deterministic: the same
              commit gives the same numbers, so a change here is a change in
              the translator.
  POST-AGENT  what the project actually delivers, after an agent resolves the
              escalations and the module is built and compared. Necessarily
              non-deterministic (see CLAUDE.md on the pipeline being
              deliberately non-hermetic).

Pre-agent alone cannot tell whether an escalation is RESOLVABLE, and an
escalation no agent can act on is worse than one that never fired. Post-agent
alone cannot separate an improved translator from a luckier agent run.

Only the pre-agent half is implemented; --post-agent is reserved and refuses
rather than silently reporting half a sweep as a whole one.

Reports PER PROJECT. 'Green is the only passing state' is a claim about one
conversion, so an average would hide a project that regressed while another
improved.

WHAT THIS CATCHES, and what it does not. Measured by reintroducing three real
bugs from this repo's history:

  caught    a conversion that silently drops a target (xz.targets 3 -> 2, the
            recursive-primaries bug: the namesake binary vanished and the
            conversion still reported success)
  MISSED    a change WITHIN an escalation (removing a catalog probe moved xz
            from 137 to 141 unmapped macros; the item count stayed at 1, so
            nothing moved) -- bzl-ccv.8
  MISSED    a regression that changes neither counts nor kinds (header staging
            resolving against the wrong base)

So this is a coarse net, not a safety net. It sees the shape of a conversion
change; it does not see the conversion get subtly wrong. The post-agent half
is what would catch the second class, because a module that no longer builds
is not a matter of counting.
"""

import argparse
import json
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import time

REPO = pathlib.Path(__file__).resolve().parents[2]
WORKSPACE_TARGET = "//translator/tests:validation_workspace"


def bazel(*args, cwd=REPO):
    return subprocess.run(
        [str(REPO / "tools" / "bazel"), *args],
        cwd=cwd,
        capture_output=True,
        text=True,
    )


def collect(unpacked: pathlib.Path) -> list[dict]:
    """One record per project, read from the CONVERSION.json each conversion
    writes. Reading the generated BUILD.bazel instead would mean regexing
    Starlark, and the markdown would mean parsing prose."""
    records = []
    for summary in sorted(unpacked.glob("fixtures/*/CONVERSION.json")):
        project = summary.parent.name
        data = json.loads(summary.read_text())
        records.append(
            {
                "project": project,
                "module": data["module"],
                "targets": len(data["targets"]),
                "executables": sum(
                    1 for t in data["targets"] if t["kind"] == "executable"
                ),
                "tests": data["tests"],
                "config_headers": data["config_headers"],
                "escalations": sum(data["escalations_by_kind"].values()),
                "escalations_by_kind": data["escalations_by_kind"],
            }
        )
    return records


def run_pre_agent() -> dict:
    started = time.time()
    build = bazel("build", WORKSPACE_TARGET)
    if build.returncode != 0:
        # A failed conversion is a sweep with no data, not a sweep of zero
        # escalations — the distinction this whole epic exists to preserve.
        sys.stderr.write(build.stderr)
        raise SystemExit(
            f"conversion failed; {WORKSPACE_TARGET} did not build, so there is "
            "nothing to measure"
        )
    tar = REPO / "bazel-bin/translator/tests/validation_workspace.tar"
    with tempfile.TemporaryDirectory() as tmp:
        with tarfile.open(tar) as archive:
            archive.extractall(tmp)
        records = collect(pathlib.Path(tmp))
    if not records:
        raise SystemExit(
            "the workspace unpacked but contained no CONVERSION.json — the "
            "sweep is looking at nothing, which is not the same as a corpus "
            "with no escalations"
        )
    return {
        "commit": subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=REPO,
            capture_output=True,
            text=True,
        ).stdout.strip(),
        "seconds": round(time.time() - started, 1),
        "projects": records,
    }


def report(sweep: dict) -> str:
    rows = sorted(sweep["projects"], key=lambda r: (-r["escalations"], r["project"]))
    width = max(len(r["project"]) for r in rows)
    out = [
        f"{'project':{width}}  {'tgts':>4} {'exe':>4} {'test':>4} {'cfgh':>4} {'escl':>4}",
        "-" * (width + 27),
    ]
    for r in rows:
        out.append(
            f"{r['project']:{width}}  {r['targets']:>4} {r['executables']:>4} "
            f"{r['tests']:>4} {r['config_headers']:>4} {r['escalations']:>4}"
        )

    by_kind: dict[str, int] = {}
    for r in rows:
        for kind, n in r["escalations_by_kind"].items():
            by_kind[kind] = by_kind.get(kind, 0) + n

    clean = sum(1 for r in rows if r["escalations"] == 0)
    out += [
        "",
        f"{len(rows)} projects, {clean} with no open escalations",
        f"{sum(r['targets'] for r in rows)} targets, "
        f"{sum(r['tests'] for r in rows)} tests, "
        f"{sum(r['escalations'] for r in rows)} escalations",
        "",
        "escalations by kind:",
    ]
    for kind, n in sorted(by_kind.items(), key=lambda kv: (-kv[1], kv[0])):
        out.append(f"  {n:>3}  {kind}")
    out.append(f"\npre-agent sweep in {sweep['seconds']}s at {sweep['commit']}")
    return "\n".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--json",
        type=pathlib.Path,
        help="also write the raw record here, for bzl-ccv.5's trend line",
    )
    parser.add_argument(
        "--post-agent",
        action="store_true",
        help="not implemented (bzl-ccv.3); refuses rather than reporting a "
        "pre-agent sweep as a full one",
    )
    args = parser.parse_args()

    if args.post_agent:
        raise SystemExit(
            "--post-agent is not implemented. The pre-agent numbers below it "
            "would be real, which is exactly why this refuses: a half sweep "
            "labelled as a whole one is the failure this epic exists to catch."
        )

    sweep = run_pre_agent()
    print(report(sweep))
    if args.json:
        args.json.write_text(json.dumps(sweep, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
