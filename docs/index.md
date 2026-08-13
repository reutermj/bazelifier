# bazelifier

Converts build scripts from other build systems into Bazel `BUILD` files.
See the [repository](https://github.com/reutermj/bazelifier) for the code and
[README](https://github.com/reutermj/bazelifier#readme) for the overview.

- **[Pipeline metrics](metrics/)** — escalations, targets and tests across
  the corpus over time, from the sweep in `tools/sweep/`.

## Design docs

- [Architecture](architecture/) — one document per component or decision area
- [Lore](lore/) — non-obvious things that cost real effort to work out
- [Runbooks](runbooks/) — recurring repo-maintenance procedures

Project-specific findings do not live here: a note about a project we convert
ships INTO that project's module, under `translator/project_notes/<project>/`,
so it reaches the agent resolving it. That includes bugs found in the
project's own build — the report and the conversion fact are two halves of
one note.
