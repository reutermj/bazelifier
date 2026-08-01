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

The pre-agent half runs the whole corpus for free. The post-agent half is
opt-in and takes ONE project (`--post-agent xz`), because the agent stage
costs real tokens per run — a full-corpus post-agent pass is a loop you launch
deliberately, not the default.

The post-agent half does not resolve anything itself. The agent stage is a
stage of the pipeline, not a subroutine of this script: it needs judgement
about project semantics, which is the whole reason the translator escalates
instead of guessing. This sets the run up outside the repo, says what is open
and where the recipes are, and measures the result once a resolution is in
place.

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
import re
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


def unpack_workspace(dest: pathlib.Path) -> pathlib.Path:
    """Builds the validation workspace and unpacks it into `dest`.

    Unpacked OUTSIDE the repo on purpose: a fixture building in place proves
    nothing, because it may just be inheriting bazelifier's own toolchains.
    See docs/architecture/build-verification.md.
    """
    build = bazel("build", WORKSPACE_TARGET)
    if build.returncode != 0:
        sys.stderr.write(build.stderr)
        raise SystemExit(
            f"conversion failed; {WORKSPACE_TARGET} did not build, so there is "
            "nothing to measure"
        )
    with tarfile.open(REPO / "bazel-bin/translator/tests/validation_workspace.tar") as a:
        a.extractall(dest)
    return dest


def open_items(project_dir: pathlib.Path) -> list[pathlib.Path]:
    return sorted(project_dir.glob("needs_attention/*.md"))


def run_post_agent(project: str, work: pathlib.Path) -> dict:
    """Measures what the project actually delivers, after the agent stage.

    Deliberately does NOT resolve anything itself. The agent stage is a stage
    of the pipeline, not a subroutine of this script — it needs judgement
    about project semantics, which is the whole reason the translator
    escalates instead of guessing. This sets the run up, reports what is open,
    and measures the result once a resolution is in place.
    """
    # Unpacked only ONCE per workspace. Re-unpacking would overwrite the
    # resolution the agent stage just wrote, silently re-measuring the
    # unresolved conversion and reporting it as the resolved one.
    if not (work / "BUILD.bazel").is_file():
        unpack_workspace(work)
    project_dir = work / "fixtures" / project
    if not project_dir.is_dir():
        raise SystemExit(
            f"no project '{project}' in the workspace; it must be enrolled in "
            "translator/tests/BUILD.bazel to be swept"
        )

    items = open_items(project_dir)

    # The comparison targets are named "<project>_<binary>_matches_ground_truth"
    # and read out of the generated root BUILD.bazel. NOT --test_filter, which
    # selects cases WITHIN a test binary and silently matches nothing for an
    # sh_test — it ran the whole corpus and reported 65 passes for one project.
    names = re.findall(
        rf'name = "({re.escape(project)}_[A-Za-z0-9_.-]+_matches_ground_truth)"',
        (work / "BUILD.bazel").read_text(),
    )
    if not names:
        raise SystemExit(
            f"'{project}' has no comparison targets in the generated workspace. "
            "A project with no runnable binary produces none, so there is "
            "nothing for the post-agent half to measure."
        )
    tests = bazel(
        "test",
        *[f"//:{n}" for n in names],
        "--override_module=cc_config=" + str(REPO / "cc_config"),
        "--keep_going",
        cwd=work,
    )
    # Counted per NAMED target, so a summary line mentioning "PASSED" cannot
    # inflate the number.
    combined = tests.stdout + tests.stderr
    passed = sum(1 for n in names if re.search(rf"//:{re.escape(n)}\s+.*PASSED", combined))
    failed = len(names) - passed

    return {
        "project": project,
        "workspace": str(work),
        "open_items": [
            {
                "file": f.name,
                "kind": _header_field(f, "kind"),
                "subject": _header_field(f, "subject"),
            }
            for f in items
        ],
        "resolved": not items,
        "comparisons_passed": passed,
        "comparisons_failed": failed,
    }


def _header_field(path: pathlib.Path, field: str) -> str:
    """Reads one field from an item's machine-readable header.

    Only the header — the prose below it is for the agent, and keying on it
    is what the header exists to avoid.
    """
    for line in path.read_text().splitlines()[1:5]:
        if line.startswith(f"{field}: "):
            return line[len(field) + 2 :].strip('"')
    return ""


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


def append_history(path: pathlib.Path, sweep: dict) -> None:
    """Appends one sweep to a JSONL history.

    JSONL rather than a single JSON document so appending is a write, not a
    read-modify-write — two sweeps racing would otherwise lose one, and a
    truncated file would lose the whole series rather than one line.

    A repeated commit is REPLACED rather than appended. Re-running a sweep on
    the same commit is normal (it is how you check a change did nothing), and
    every run would otherwise add a point the graph reads as elapsed time.
    """
    dated = dict(sweep)
    dated["date"] = time.strftime("%Y-%m-%d")
    dated["timestamp"] = int(time.time())

    rows = []
    if path.exists():
        rows = [
            json.loads(line)
            for line in path.read_text().splitlines()
            if line.strip()
        ]
    rows = [r for r in rows if r.get("commit") != dated["commit"]]
    rows.append(dated)
    rows.sort(key=lambda r: r.get("timestamp", 0))

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in rows))
    print(f"\nappended to {path} ({len(rows)} sweeps)")


def report_post_agent(r: dict) -> str:
    out = [f"project:   {r['project']}", f"workspace: {r['workspace']}", ""]
    if r["open_items"]:
        out.append(f"{len(r['open_items'])} OPEN escalation(s) — resolve in the")
        out.append("GENERATED output under the workspace above, then re-run:")
        for i in r["open_items"]:
            out.append(f"  {i['kind']:<28} {i['subject']}")
        out.append("")
        out.append("Recipes for each shape are shipped alongside them, in")
        out.append(f"  {r['workspace']}/fixtures/{r['project']}/resolutions/")
    else:
        out.append("no open escalations")
    out.append("")
    out.append(
        f"comparisons: {r['comparisons_passed']} passed, "
        f"{r['comparisons_failed']} failed"
    )
    return "\n".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--json",
        type=pathlib.Path,
        help="also write this sweep's raw record here",
    )
    parser.add_argument(
        "--append",
        type=pathlib.Path,
        metavar="HISTORY.jsonl",
        help="append this sweep to a history file, one JSON object per line, "
        "for the trend report",
    )
    parser.add_argument(
        "--post-agent",
        metavar="PROJECT",
        help="measure ONE project's post-agent state: unpack the workspace "
        "outside the repo, report its open escalations, and run its "
        "comparisons. Opt-in and single-project because the agent stage "
        "costs real tokens per run",
    )
    parser.add_argument(
        "--workspace",
        type=pathlib.Path,
        help="where to unpack (--post-agent only). Kept rather than deleted, "
        "so a resolution can be written into it and re-measured",
    )
    args = parser.parse_args()

    if args.post_agent:
        work = args.workspace or pathlib.Path(
            tempfile.mkdtemp(prefix=f"sweep_{args.post_agent}_")
        )
        work.mkdir(parents=True, exist_ok=True)
        result = run_post_agent(args.post_agent, work)
        print(report_post_agent(result))
        if args.json:
            args.json.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        # Open items mean an unfinished conversion, so the exit code says so —
        # green is the only passing state.
        green = (
            result["resolved"]
            and result["comparisons_failed"] == 0
            and result["comparisons_passed"] > 0
        )
        # comparisons_passed > 0 matters: a project whose comparisons all
        # failed to BUILD reports 0 passed and 0 failed, which would otherwise
        # read as success — a gate looking at nothing.
        return 0 if green else 1

    sweep = run_pre_agent()
    print(report(sweep))
    if args.json:
        args.json.write_text(json.dumps(sweep, indent=2, sort_keys=True) + "\n")
    if args.append:
        append_history(args.append, sweep)
    return 0


if __name__ == "__main__":
    sys.exit(main())
