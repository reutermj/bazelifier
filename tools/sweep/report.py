#!/usr/bin/env python3
"""Renders a sweep history as a self-contained HTML report.

    python3 tools/sweep/report.py metrics/history.jsonl -o report.html

One file, no dependencies, no server: charts are inline SVG generated here
rather than a library loaded at view time. That is the same constraint the
generated modules are held to, and it means the report still opens years from
now.

Reads the JSONL that `sweep.py --append` writes. One line per sweep, one
sweep per commit.
"""

import argparse
import html
import json
import pathlib
import re
import sys

# Deliberately few colours: the point is to read a trend, not to decorate.
INK = "#1f2933"
MUTED = "#7b8794"
GRID = "#e4e7eb"
SERIES = ["#2f6f9f", "#b0562a", "#4a7c59", "#8a5a9e", "#a03e52", "#7a6a3a"]


def load_post_agent(history: pathlib.Path) -> dict[str, dict]:
    """Latest post-agent run per project, from the sibling JSONL.

    Absent for most projects, and that is the point: "not measured" and "not
    green" are different states, and conflating them is what let a project sit
    unvalidated for three days without it being visible (bzl-spf).
    """
    path = history.with_name(history.stem + "_post_agent.jsonl")
    if not path.exists():
        return {}
    latest: dict[str, dict] = {}
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        prev = latest.get(row["project"])
        if prev is None or row.get("timestamp", 0) >= prev.get("timestamp", 0):
            latest[row["project"]] = row
    return latest


def validated_cell(row: dict | None) -> str:
    """How a project's post-agent state renders, including never-run."""
    if row is None:
        return '<span class="flat" title="the agent stage has not been run for this project">not measured</span>'
    if row.get("green"):
        return f'<span class="down" title="at {row["commit"]}">validated</span>'
    return '<span class="up">not green</span>'


def load(path: pathlib.Path) -> list[dict]:
    rows = [
        json.loads(line) for line in path.read_text().splitlines() if line.strip()
    ]
    if not rows:
        raise SystemExit(f"{path} has no sweeps in it — nothing to plot")
    rows.sort(key=lambda r: r.get("timestamp", 0))
    return rows


def totals(sweep: dict) -> dict:
    p = sweep["projects"]
    by_kind: dict[str, int] = {}
    for proj in p:
        for kind, n in proj["escalations_by_kind"].items():
            by_kind[kind] = by_kind.get(kind, 0) + n
    return {
        "projects": len(p),
        "clean": sum(1 for x in p if x["escalations"] == 0),
        "escalations": sum(x["escalations"] for x in p),
        "targets": sum(x["targets"] for x in p),
        "tests": sum(x["tests"] for x in p),
        "by_kind": by_kind,
    }


def line_chart(title: str, series: list[tuple[str, list[float]]], labels: list[str]) -> str:
    """A multi-series line chart as inline SVG.

    Hand-rolled rather than pulled from a library because the report has to be
    one self-contained file; a CDN script tag would make it stop working the
    moment it is opened offline or the CDN moves.
    """
    w, h = 720, 220
    pad_l, pad_r, pad_t, pad_b = 44, 12, 16, 34
    plot_w, plot_h = w - pad_l - pad_r, h - pad_t - pad_b

    values = [v for _, vs in series for v in vs]
    hi = max(values) if values else 0
    hi = max(hi, 1)
    # A single sweep is a point, not a line: x would divide by zero, and a
    # "trend" through one observation would be a lie either way.
    n = len(labels)

    def x(i: int) -> float:
        return pad_l if n == 1 else pad_l + plot_w * i / (n - 1)

    def y(v: float) -> float:
        return pad_t + plot_h * (1 - v / hi)

    parts = [f'<svg viewBox="0 0 {w} {h}" role="img" aria-label="{html.escape(title)}">']
    for frac in (0, 0.5, 1):
        gy = pad_t + plot_h * frac
        parts.append(
            f'<line x1="{pad_l}" y1="{gy:.1f}" x2="{w - pad_r}" y2="{gy:.1f}" '
            f'stroke="{GRID}"/>'
        )
        parts.append(
            f'<text x="{pad_l - 8}" y="{gy + 4:.1f}" text-anchor="end" '
            f'font-size="11" fill="{MUTED}">{hi * (1 - frac):.0f}</text>'
        )
    for idx, (name, vs) in enumerate(series):
        colour = SERIES[idx % len(SERIES)]
        pts = " ".join(f"{x(i):.1f},{y(v):.1f}" for i, v in enumerate(vs))
        if n > 1:
            parts.append(
                f'<polyline points="{pts}" fill="none" stroke="{colour}" '
                f'stroke-width="2" stroke-linejoin="round"/>'
            )
        for i, v in enumerate(vs):
            parts.append(
                f'<circle cx="{x(i):.1f}" cy="{y(v):.1f}" r="3.5" fill="{colour}">'
                f"<title>{html.escape(name)} — {labels[i]}: {v:g}</title></circle>"
            )
    # Only the ends are labelled; a dense axis is unreadable and the tooltips
    # carry the rest.
    if labels:
        parts.append(
            f'<text x="{pad_l}" y="{h - 10}" font-size="11" fill="{MUTED}">'
            f"{html.escape(labels[0])}</text>"
        )
        if n > 1:
            parts.append(
                f'<text x="{w - pad_r}" y="{h - 10}" text-anchor="end" '
                f'font-size="11" fill="{MUTED}">{html.escape(labels[-1])}</text>'
            )
    parts.append("</svg>")

    legend = " ".join(
        f'<span class="key"><i style="background:{SERIES[i % len(SERIES)]}"></i>'
        f"{html.escape(nm)}</span>"
        for i, (nm, _) in enumerate(series)
    )
    return (
        f'<section><h2>{html.escape(title)}</h2>'
        f'<div class="legend">{legend}</div>{"".join(parts)}</section>'
    )


STYLE = """<style>
  :root { color-scheme: light dark; }
  body { font: 15px/1.5 system-ui, sans-serif; margin: 0 auto; padding: 2rem 1.25rem;
         max-width: 60rem; color: {INK}; }
  h1 { font-size: 1.4rem; margin: 0 0 .25rem; }
  h2 { font-size: 1rem; margin: 0 0 .5rem; font-weight: 600; }
  .sub { color: {MUTED}; margin: 0 0 2rem; }
  section { margin: 0 0 2.25rem; }
  svg { width: 100%; height: auto; overflow: visible; }
  .legend { margin: 0 0 .5rem; font-size: .8rem; color: {MUTED}; }
  .key { margin-right: 1rem; white-space: nowrap; }
  .key i { display: inline-block; width: .6rem; height: .6rem; border-radius: 50%;
            margin-right: .35rem; }
  .wrap { overflow-x: auto; }
  table { border-collapse: collapse; width: 100%; font-size: .9rem; }
  th, td { text-align: left; padding: .35rem .6rem; border-bottom: 1px solid {GRID}; }
  th { color: {MUTED}; font-weight: 600; }
  td.n, th.n { text-align: right; font-variant-numeric: tabular-nums; }
  .up { color: #a03e52; } .down { color: #4a7c59; }
  .flat, .new { color: {MUTED}; }
  .cards { display: flex; flex-wrap: wrap; gap: 1.5rem; margin: 0 0 2rem; }
  .card b { display: block; font-size: 1.6rem; font-variant-numeric: tabular-nums; }
  .card span { color: {MUTED}; font-size: .8rem; }
  @media (prefers-color-scheme: dark) {
    body { color: #e4e7eb; background: #17202a; }
    th, td { border-color: #2c3946; }
  }
  :root[data-theme="dark"] body { color: #e4e7eb; background: #17202a; }
  :root[data-theme="light"] body { color: {INK}; background: #fff; }
</style>"""


def slug(project: str) -> str:
    """A filename for a project's page. Projects are already
    filesystem-safe directory names, so this only guards the separator."""
    return re.sub(r"[^A-Za-z0-9._-]", "_", project)


def series_for(rows: list[dict], project: str) -> list[tuple[dict, dict]]:
    """(sweep, that project's record) for every sweep the project appears in.

    Skips sweeps predating the project rather than plotting a zero, which
    would read as "it had no escalations" instead of "it did not exist".
    """
    out = []
    for sweep in rows:
        rec = next((p for p in sweep["projects"] if p["project"] == project), None)
        if rec is not None:
            out.append((sweep, rec))
    return out


def render_project(rows: list[dict], project: str, post: dict | None) -> str:
    pairs = series_for(rows, project)
    labels = [s.get("date", "?") + " " + s["commit"][:7] for s, _ in pairs]
    recs = [r for _, r in pairs]
    latest = recs[-1]

    kinds = sorted({k for r in recs for k in r["escalations_by_kind"]})
    charts = [
        line_chart(
            "Open escalations",
            [("escalations", [r["escalations"] for r in recs])],
            labels,
        ),
        line_chart(
            "What the translator produced",
            [
                ("targets", [r["targets"] for r in recs]),
                ("executables", [r["executables"] for r in recs]),
                ("tests", [r["tests"] for r in recs]),
            ],
            labels,
        ),
    ]
    if kinds:
        charts.append(
            line_chart(
                "By escalation kind",
                [(k, [r["escalations_by_kind"].get(k, 0) for r in recs]) for k in kinds],
                labels,
            )
        )

    if post is None:
        post_section = (
            '<p class="sub">Whether it is currently GREEN is a separate '
            "question, and the agent stage has never been run for it — so this "
            'page says "not measured" rather than implying "not green". Run '
            f"<code>python3 tools/sweep/sweep.py --post-agent {html.escape(project)} "
            "--append metrics/history.jsonl</code>.</p>"
        )
    else:
        verdict = "GREEN" if post.get("green") else "NOT green"
        post_section = (
            f'<p class="sub">Post-agent, at <code>{html.escape(post["commit"])}</code> '
            f'({html.escape(post.get("date", "?"))}): <strong>{verdict}</strong> — '
            f"{post['comparisons_passed']} of "
            f"{post['comparisons_passed'] + post['comparisons_failed']} comparisons "
            f"and {post['module_tests_passed']} of "
            f"{post['module_tests_passed'] + post['module_tests_failed']} module tests "
            "passed. Resolutions are ephemeral by design, so this describes one "
            "run at one commit rather than a standing property.</p>"
        )

    open_rows = "".join(
        f"<tr><td><code>{html.escape(k)}</code></td><td class=n>{n}</td></tr>"
        for k, n in sorted(latest["escalations_by_kind"].items(), key=lambda kv: -kv[1])
    ) or '<tr><td colspan="2" class="flat">none open</td></tr>'

    return f"""<title>{html.escape(project)} — bazelifier metrics</title>
{STYLE}
<p class="sub"><a href="index.html">← all projects</a></p>
<h1>{html.escape(project)}</h1>
<p class="sub">module <code>{html.escape(latest['module'])}</code> ·
  {len(pairs)} sweep{"s" if len(pairs) != 1 else ""} ·
  latest {html.escape(pairs[-1][0].get("date", "?"))}</p>

<div class="cards">
  <div class="card"><b>{latest['escalations']}</b><span>open escalations</span></div>
  <div class="card"><b>{latest['targets']}</b><span>targets</span></div>
  <div class="card"><b>{latest['executables']}</b><span>executables</span></div>
  <div class="card"><b>{latest['tests']}</b><span>tests</span></div>
  <div class="card"><b>{latest['config_headers']}</b><span>config headers</span></div>
</div>

{"".join(charts)}

<section><h2>Open escalations, by kind</h2>
<div class="wrap"><table>
<tr><th>kind</th><th class=n>count</th></tr>
{open_rows}
</table></div>
<p class="sub">These are <strong>pre-agent</strong> numbers — what the
translator produced on its own. An open escalation is unfinished work
awaiting the agent stage, not a defect, and resolutions are deliberately
ephemeral: a translation carries project-specific semantics, and the point
is that the process adapts to them rather than memorises them. So this page
shows what the translator alone achieves, which is the half that should be
reproducible.</p>
{post_section}
</section>
"""


def render(rows: list[dict], post: dict[str, dict]) -> str:
    labels = [r.get("date", "?") + " " + r["commit"][:7] for r in rows]
    tots = [totals(r) for r in rows]

    charts = [
        line_chart(
            "Open escalations across the corpus",
            [("escalations", [t["escalations"] for t in tots])],
            labels,
        ),
        line_chart(
            "Projects with no open escalations",
            [
                ("clean", [t["clean"] for t in tots]),
                ("total projects", [t["projects"] for t in tots]),
            ],
            labels,
        ),
        line_chart(
            "What the translator produced",
            [
                ("targets", [t["targets"] for t in tots]),
                ("tests", [t["tests"] for t in tots]),
            ],
            labels,
        ),
    ]

    kinds = sorted({k for t in tots for k in t["by_kind"]})
    if kinds:
        charts.append(
            line_chart(
                "Escalations by kind",
                [(k, [t["by_kind"].get(k, 0) for t in tots]) for k in kinds],
                labels,
            )
        )

    latest = rows[-1]
    per_project = sorted(
        latest["projects"], key=lambda p: (-p["escalations"], p["project"])
    )
    # Previous sweep for the same project, so the table shows MOVEMENT rather
    # than a snapshot — the question is "did this change", not "what is it".
    prev = (
        {p["project"]: p for p in rows[-2]["projects"]} if len(rows) > 1 else {}
    )
    body_rows = []
    for p in per_project:
        was = prev.get(p["project"], {}).get("escalations")
        if was is None:
            delta = '<span class="new">new</span>'
        elif was == p["escalations"]:
            delta = '<span class="flat">—</span>'
        else:
            arrow = "▲" if p["escalations"] > was else "▼"
            css = "up" if p["escalations"] > was else "down"
            delta = f'<span class="{css}">{arrow} {was} → {p["escalations"]}</span>'
        body_rows.append(
            f"<tr><td><a href=\"{slug(p['project'])}.html\">"
            f"{html.escape(p['project'])}</a></td>"
            f"<td class=n>{p['targets']}</td><td class=n>{p['tests']}</td>"
            f"<td class=n>{p['escalations']}</td><td>{delta}</td>"
            f"<td>{validated_cell(post.get(p['project']))}</td></tr>"
        )

    t = totals(latest)
    return f"""<title>bazelifier pipeline metrics</title>
{STYLE}
<h1>bazelifier pipeline metrics</h1>
<p class="sub">{len(rows)} sweep{"s" if len(rows) != 1 else ""} ·
  latest {html.escape(latest.get("date", "?"))} at
  <code>{html.escape(latest["commit"])}</code></p>

<div class="cards">
  <div class="card"><b>{sum(1 for r in post.values() if r.get("green"))}/{t['projects']}</b><span>projects validated</span></div>
  <div class="card"><b>{t['clean']}/{t['projects']}</b><span>no open escalations</span></div>
  <div class="card"><b>{t['escalations']}</b><span>open escalations</span></div>
  <div class="card"><b>{t['targets']}</b><span>targets</span></div>
  <div class="card"><b>{t['tests']}</b><span>tests</span></div>
</div>

{"".join(charts)}

<section><h2>Per project, latest sweep</h2>
<div class="wrap"><table>
<tr><th>project</th><th class=n>targets</th><th class=n>tests</th>
    <th class=n>escalations</th><th>change</th><th>validated</th></tr>
{"".join(body_rows)}
</table></div></section>
"""


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("history", type=pathlib.Path)
    ap.add_argument(
        "-o",
        "--out",
        type=pathlib.Path,
        default=pathlib.Path("docs/metrics/index.html"),
        help="where to write the report. Defaults to the GitHub Pages "
        "directory, which is the committed copy",
    )
    args = ap.parse_args()
    rows = load(args.history)
    post = load_post_agent(args.history)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(render(rows, post))

    # One page per project, beside the index. Written every run rather than
    # only for projects that changed: a stale page is worse than none, and a
    # project that vanished from the corpus should stop having a page at all.
    projects = sorted({p["project"] for r in rows for p in r["projects"]})
    for project in projects:
        (args.out.parent / f"{slug(project)}.html").write_text(
            render_project(rows, project, post.get(project))
        )
    print(f"wrote {args.out} and {len(projects)} project pages")
    return 0


if __name__ == "__main__":
    sys.exit(main())
