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
  caught    a change WITHIN an escalation, since 2026-08-11: each item
            carries a digest of its RENDERED text, and a project whose
            digest moved while its count did not is reported separately.
            Verified by reintroducing the am__ merge bug -- xz's fabricated
            test names went 8 -> 18 with the item count unchanged, and the
            sweep named five affected projects (bzl-ccv.8).
  MISSED    a regression that changes neither counts nor kinds NOR any
            escalation text (header staging resolving against the wrong
            base changed only which files were copied)

So this is a coarse net, not a safety net. It sees the shape of a conversion
change; it does not see the conversion get subtly wrong. The post-agent half
is what would catch the second class, because a module that no longer builds
is not a matter of counting.
"""

import argparse
import hashlib
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
                # What the escalations SAY, not just how many there are.
                # Every count above is blind to an item whose content moved:
                # xz reports `{unmapped_config_macros: 1}` whether that item
                # names 70 macros or 77, so two real regressions shipped with
                # every field here byte-identical. See bzl-ccv.8.
                #
                # One value per project rather than per item: the sweep's
                # question is "did this project's escalations change", and a
                # list would grow the row without answering it any better.
                "escalation_digest": escalation_digest(data.get("escalations", [])),
            }
        )
    return records


def escalation_digest(escalations: list[dict]) -> str:
    """Combines a project's per-item digests into one comparable value.

    Sorted, so two runs that emit the same items in a different order agree
    — the sweep would otherwise report drift on every run and the signal
    would be ignored, which is worse than not having it.

    Empty for a project with no escalations, which reads better in a diff
    than a hash of nothing and is exactly as comparable.
    """
    digests = sorted(e.get("digest", "") for e in escalations)
    if not digests:
        return ""
    combined = hashlib.sha256("\n".join(digests).encode()).hexdigest()
    return combined[:16]


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


EXPECTED_COMPARISONS = "expected_comparisons.txt"


def freeze_expected_comparisons(work: pathlib.Path) -> None:
    """Records the comparison targets the CONVERSION produced, before the
    agent stage can touch them.

    The gate used to derive its expected set by regexing the root
    `BUILD.bazel` at measurement time — which is *after* the agent has
    edited the workspace. A resolution that deleted a failing test therefore
    removed it from the expectation too, and the project went green having
    fixed nothing. Mutation-verified on jansson: 19 failing comparisons
    became 0 passed / 0 failed / exit 0.

    Written at unpack, which is the only moment the workspace is known to be
    the translator's own output. `run_post_agent` refuses to re-unpack for
    the same reason this exists, so the snapshot survives the agent stage.
    """
    names = comparison_names((work / "BUILD.bazel").read_text())
    (work / EXPECTED_COMPARISONS).write_text("".join(f"{n}\n" for n in sorted(names)))


def comparison_names(build_text: str, project: str | None = None) -> list[str]:
    """Every `*_matches_ground_truth` target in a root `BUILD.bazel`, or just
    one project's.

    One parser rather than two: the freeze and the per-project measurement
    have to agree about what counts as a comparison target, and a regex
    written twice is how they would stop agreeing.
    """
    prefix = rf"{re.escape(project)}_" if project else ""
    return re.findall(rf'name = "({prefix}[A-Za-z0-9_.-]+_matches_ground_truth)"', build_text)


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
        freeze_expected_comparisons(work)
    project_dir = work / "fixtures" / project
    if not project_dir.is_dir():
        raise SystemExit(
            f"no project '{project}' in the workspace; it must be enrolled in "
            "translator/tests/BUILD.bazel to be swept"
        )

    items = open_items(project_dir)
    # The module's Bazel repo name, taken from the root MODULE.bazel's
    # local_path_override rather than assumed equal to the directory name.
    m = re.search(
        rf'module_name\s*=\s*"([^"]+)",\s*\n\s*path\s*=\s*"fixtures/{re.escape(project)}"',
        (work / "MODULE.bazel").read_text(),
    )
    module = m.group(1) if m else project

    # The comparison targets are named "<project>_<binary>_matches_ground_truth"
    # and read out of the generated root BUILD.bazel. NOT --test_filter, which
    # selects cases WITHIN a test binary and silently matches nothing for an
    # sh_test — it ran the whole corpus and reported 65 passes for one project.
    # From the FROZEN snapshot, not the current BUILD.bazel. Reading the live
    # file would let a resolution that deleted a failing comparison delete the
    # expectation with it — the project then reports 0 failed and goes green
    # having fixed nothing. See freeze_expected_comparisons.
    frozen = work / EXPECTED_COMPARISONS
    if frozen.is_file():
        names = [
            n
            for n in frozen.read_text().split()
            if n.startswith(f"{project}_") and n.endswith("_matches_ground_truth")
        ]
    else:
        # A workspace unpacked before this existed. Fall back rather than
        # refuse, but say so: this run cannot detect a deleted comparison.
        print(
            f"warning: {frozen.name} missing — expectations taken from the "
            "current BUILD.bazel, so a DELETED comparison will not be caught. "
            "Remove the workspace and re-run for a trustworthy result.",
            file=sys.stderr,
        )
        names = comparison_names((work / "BUILD.bazel").read_text(), project)

    # A target the conversion produced and the workspace no longer defines was
    # deleted by the agent stage. Counted as failed rather than skipped: the
    # whole point of freezing is that disappearing is not a way to pass.
    live = set(comparison_names((work / "BUILD.bazel").read_text(), project))
    deleted = [n for n in names if n not in live]
    # No comparison targets is legitimate, not an error: a project that builds
    # only a library produces no runnable binary to compare. tinyxml2 is that
    # shape and still ships an sh_test of its own, so bailing here would skip
    # the only check it has. What IS an error is having neither — see the
    # green condition, which requires at least one test of some kind to run.
    passed = failed = 0
    if names:
        # Only the surviving labels are handed to Bazel — an undefined one is
        # a load-time error that would stop the others from running at all.
        # The deleted ones are counted below without being run, which is the
        # honest reading: they were expected and are not there.
        runnable = [n for n in names if n in live]
        combined = ""
        if runnable:
            tests = bazel(
                "test",
                *[f"//:{n}" for n in runnable],
                "--override_module=cc_config=" + str(REPO / "cc_config"),
                "--keep_going",
                cwd=work,
            )
            combined = tests.stdout + tests.stderr
        # Counted per NAMED target, so a summary line mentioning "PASSED"
        # cannot inflate the number.
        passed = sum(
            1 for n in runnable if re.search(rf"//:{re.escape(n)}\s+.*PASSED", combined)
        )
        # Everything expected and not passing, deletions included.
        failed = len(names) - passed
        if deleted:
            print(
                f"\n{len(deleted)} comparison target(s) the conversion produced "
                "are GONE from the workspace — counted as failed:",
                file=sys.stderr,
            )
            for n in sorted(deleted):
                print(f"  {n}", file=sys.stderr)
            print(
                "A resolution may not reach green by deleting what was failing. "
                "Restore the target and make it pass, or record in the generated "
                "BUILD.bazel why it cannot exist.",
                file=sys.stderr,
            )

    # The module's OWN tests, which the comparisons do not cover and which
    # nothing else runs. A converted module ships config-header assertions and
    # any test the project registered; a resolution that satisfies the runtime
    # comparison can still leave one of those red — and for a config-header
    # resolution the assertion is the ONLY thing checking the work, since a
    # wrong value often still compiles.
    own = bazel(
        "test",
        f"@{module}//:all",
        "--override_module=cc_config=" + str(REPO / "cc_config"),
        "--keep_going",
        cwd=work,
    )
    own_out = own.stdout + own.stderr
    own_passed = len(re.findall(r"^@\S+\s+.*\bPASSED\b", own_out, re.M))
    own_failed = len(re.findall(r"^@\S+\s+.*\b(?:FAILED|TIMEOUT)\b", own_out, re.M))
    # rc 4 is "no tests found", which is a legitimate answer for a module that
    # registered none — distinct from a failure, and not counted as one.
    if own.returncode not in (0, 4) and own_failed == 0:
        own_failed = 1

    return {
        "module_tests_passed": own_passed,
        "module_tests_failed": own_failed,
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


def report(sweep: dict, history: pathlib.Path | None = None) -> str:
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

    changed = escalation_content_drift(rows, history)
    if changed:
        # The whole point of the digest: these projects' counts did not
        # move, so nothing else in this report mentions them.
        out += [
            "",
            "escalation CONTENT changed (same count, different text):",
        ]
        out += [f"  {p}" for p in changed]

    out.append(f"\npre-agent sweep in {sweep['seconds']}s at {sweep['commit']}")
    return "\n".join(out)


def escalation_content_drift(
    rows: list[dict], history: pathlib.Path | None
) -> list[str]:
    """Projects whose escalation TEXT differs from the last recorded sweep.

    Compared against the most recent history row rather than a frozen
    expectation, because the corpus legitimately moves and a frozen list
    would need updating on every intentional change — the way to make a
    check get ignored. Drift here is a prompt to look, not a failure.

    Silent when there is no history to compare against, which is also the
    first-run case. A project absent from the previous sweep is new, and new
    is reported by the count columns already.
    """
    if history is None or not history.exists():
        return []
    rows_by_project = {r["project"]: r for r in rows}
    previous = [
        json.loads(line) for line in history.read_text().splitlines() if line.strip()
    ]
    if not previous:
        return []
    last = previous[-1]

    changed = []
    for old in last.get("projects", []):
        new = rows_by_project.get(old["project"])
        if new is None or "escalation_digest" not in old:
            continue
        # Only when the COUNT is unchanged. A project that gained or lost an
        # escalation already shows up in the table above, and reporting it
        # twice would bury the case this exists for.
        if (
            old["escalation_digest"] != new["escalation_digest"]
            and old["escalations"] == new["escalations"]
        ):
            changed.append(old["project"])
    return sorted(changed)


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


def record_post_agent(path: pathlib.Path, result: dict) -> None:
    """Appends one post-agent run to its own JSONL, separate from the sweep.

    Separate on purpose. A pre-agent row is a property of the COMMIT — same
    commit, same numbers — while a post-agent result is a property of
    (commit, workspace, agent run), and re-resolving is expected rather than
    exceptional: resolutions are deliberately ephemeral (bzl-b9b). Merging
    the two would present a non-deterministic result in a column readers
    treat as reproducible.

    Keyed by (commit, project) so re-running the stage on one project
    replaces its row rather than accumulating attempts, but a DIFFERENT
    commit keeps its own — which is what makes "was this project ever green,
    and at which commit" answerable.
    """
    row = {
        "commit": result["commit"],
        "project": result["project"],
        "date": time.strftime("%Y-%m-%d"),
        "timestamp": int(time.time()),
        "resolved": result["resolved"],
        "open_items": [i["kind"] for i in result["open_items"]],
        "comparisons_passed": result["comparisons_passed"],
        "comparisons_failed": result["comparisons_failed"],
        "module_tests_passed": result["module_tests_passed"],
        "module_tests_failed": result["module_tests_failed"],
        "green": result["green"],
    }
    rows = []
    if path.exists():
        rows = [json.loads(l) for l in path.read_text().splitlines() if l.strip()]
    rows = [
        r
        for r in rows
        if (r.get("commit"), r.get("project")) != (row["commit"], row["project"])
    ]
    rows.append(row)
    rows.sort(key=lambda r: (r.get("timestamp", 0), r.get("project", "")))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in rows))
    print(f"\nrecorded to {path} ({len(rows)} runs)")


def report_post_agent(r: dict) -> str:
    out = [f"project:   {r['project']}", f"workspace: {r['workspace']}", ""]
    if r["open_items"]:
        out.append(f"{len(r['open_items'])} OPEN escalation(s) — resolve in the")
        out.append("GENERATED output under the workspace above, then re-run:")
        for i in r["open_items"]:
            out.append(f"  {i['kind']:<28} {i['subject']}")
        out.append("")
        # Each item carries its own guidance; there is no separate recipe
        # directory to point at. `project_notes/` is different and only
        # exists for some projects, so it is named only when present —
        # pointing at a directory that is not there is what this line used
        # to do.
        notes = (
            pathlib.Path(r["workspace"]) / "fixtures" / r["project"] / "project_notes"
        )
        if notes.is_dir():
            out.append("This project has NOTES — read them before resolving, they")
            out.append("record where the obvious answer is wrong:")
            for note in sorted(notes.glob("*.md")):
                out.append(f"  {note}")
    else:
        out.append("no open escalations")
    out.append("")
    out.append(
        f"comparisons:  {r['comparisons_passed']} passed, "
        f"{r['comparisons_failed']} failed"
    )
    out.append(
        f"module tests: {r['module_tests_passed']} passed, "
        f"{r['module_tests_failed']} failed"
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
        ran = result["comparisons_passed"] + result["module_tests_passed"]
        result["commit"] = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=REPO,
            capture_output=True,
            text=True,
        ).stdout.strip()
        green = (
            result["resolved"]
            and result["comparisons_failed"] == 0
            and result["module_tests_failed"] == 0
            and ran > 0
        )
        # `ran > 0` is the gate-looking-at-nothing guard, and it counts BOTH
        # kinds rather than requiring one of each. Either alone is a real
        # shape: a library-only project (tinyxml2) has no runnable binary to
        # compare and still ships an sh_test; a fixture with no config header
        # and no registered test has only its comparison. What must not pass
        # is a project where everything failed to BUILD — that reports 0
        # passed and 0 failed on both counts and would otherwise read green.
        result["green"] = green
        if args.append:
            # Its own file beside the sweep history — see record_post_agent
            # for why a post-agent result must not share a row with the
            # pre-agent numbers.
            record_post_agent(
                args.append.with_name(args.append.stem + "_post_agent.jsonl"), result
            )
        return 0 if green else 1

    sweep = run_pre_agent()
    # Compared against the committed history even without `--append`, because
    # `python3 tools/sweep/sweep.py` with no flags is the invocation CLAUDE.md
    # prescribes after a change — and a drift check that only runs when you
    # remember a flag is one that reports nothing on the run that mattered.
    print(report(sweep, args.append or REPO / "metrics" / "history.jsonl"))
    if args.json:
        args.json.write_text(json.dumps(sweep, indent=2, sort_keys=True) + "\n")
    if args.append:
        append_history(args.append, sweep)
    return 0


if __name__ == "__main__":
    sys.exit(main())
