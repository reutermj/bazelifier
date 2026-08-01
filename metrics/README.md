# Pipeline metrics

`history.jsonl` is the sweep's time series: one JSON object per line, one
line per commit. Written by

```sh
python3 tools/sweep/sweep.py --append metrics/history.jsonl
```

and rendered by

```sh
python3 tools/sweep/report.py metrics/history.jsonl -o report.html
```

The report is one self-contained HTML file — inline SVG, no dependencies, no
network — so it opens anywhere and still works offline.

## Why JSONL, and why re-running is safe

Appending is a write rather than a read-modify-write, so two sweeps cannot
lose each other and a truncated file loses one line rather than the series.

Re-running on the same commit **replaces** that row instead of adding one.
Re-running is normal — it is how you check that a change moved nothing — and
every run would otherwise add a point the graph reads as elapsed time.

## What the numbers mean, and what they miss

These are **pre-agent** numbers: what the translator alone produced. They do
not say whether an escalation is resolvable, or whether the module builds
once resolved. That is the post-agent half (`sweep.py --post-agent <project>`),
which is opt-in per project because it spends tokens.

The series is also coarse by construction. It sees a conversion's shape
change — a dropped target, a new escalation kind — and not a conversion
getting subtly wrong. `tools/sweep/sweep.py`'s module docstring records
exactly which real bugs it caught and which it missed when that was measured.

## History starts here

There is no backfill. Sweeps before this file existed cannot be
reconstructed faithfully — `CONVERSION.json` did not exist then, so the
numbers would come from counting files rather than from the record the
conversion wrote, and a series that changes measurement method partway is
worse than a short one.
