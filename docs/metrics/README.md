# Pipeline metrics site

`index.html` is generated, not hand-written:

```sh
python3 tools/sweep/sweep.py --append metrics/history.jsonl
python3 tools/sweep/report.py metrics/history.jsonl
```

It is committed on purpose. This repo otherwise keeps generated files out of
git, and the exception is deliberate: GitHub Pages serves what is committed,
so the alternative is a workflow that rebuilds it — and there is no CI here
(see the epic, bzl-ccv: token cost outside the subscription is why the sweep
runs locally).

Regenerate it in the same commit as the sweep that changed it. A stale page
is worse than no page, because it looks current.

## Enabling Pages

Settings → Pages → Source: *Deploy from a branch*, branch `main`, folder
`/docs`. Nothing else; no workflow, no Actions minutes.

The published site is then the whole of `docs/`, which includes
`architecture/` and `lore/`. That is fine for a public repo and worth knowing
rather than discovering: it is design prose, not secrets. `.nojekyll` stops
Pages running the markdown through Jekyll, which would otherwise try to
process the `{{` and `{%` sequences that appear in code samples.

The report lands at `<user>.github.io/bazelifier/metrics/`.
