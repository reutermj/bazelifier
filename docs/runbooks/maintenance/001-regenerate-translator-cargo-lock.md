# Runbook: regenerate translator/Cargo.lock

- **Status:** resolved (this is a recurring/standing runbook, not a one-off — re-run whenever the trigger below applies)
- **Trigger:** `translator/Cargo.toml` dependencies changed (added, removed,
  or version bumped) and `translator/Cargo.lock` needs to be regenerated to
  match.
- **Stage:** repo maintenance / Bazel module wiring (not CMake translation).

## Gap

`translator/BUILD.bazel`'s third-party deps are resolved via `rules_rs`'s
`crate.from_cargo` extension in `MODULE.bazel`, which requires a real,
`cargo`-generated `Cargo.lock` (see
[docs/architecture/bazel-codegen.md](../../architecture/bazel-codegen.md)
and the `rules_rs` `bazel_dep` in `MODULE.bazel`). `rules_rs` explicitly does
**not** support generating or updating a lockfile itself:

- Its own docs state: "`crate.spec` and vendoring mode are not currently
  supported" — `crate.from_cargo` is the only supported path, and it always
  requires a pre-existing lockfile.
- Reading `rules_rs`'s actual source
  (`rs/extensions.bzl`, `_generate_hub_and_spokes`) confirms it only ever
  runs `cargo metadata --no-deps --locked` against the lockfile you give
  it — never `cargo generate-lockfile` or `cargo update`.
- `rules_rs` does bundle its own hermetic `cargo` binary for internal use
  (see `rs/toolchains/module_extension.bzl`, `host_tools_repository`), but
  it's explicitly commented as "an implementation detail of rules_rs
  itself" and is deliberately not re-exported to user modules via
  `use_repo`. There is no stable, public Bazel target that exposes it.

So there is no way to produce/update `translator/Cargo.lock` through Bazel
alone. A real, locally-installed `cargo` is required.

## What was tried

Looked for a Bazel-native lockfile-generation path in `rules_rs` (a
`crate.spec()` declarative form, a `bazel run //:crates_repin`-style target,
a `CARGO_BAZEL_REPIN`-equivalent env var as `rules_rust`'s own
`crate_universe` supports). None exist in `rules_rs` — confirmed by reading
its source directly rather than trusting README summaries (see
[docs/lore/](../../lore/) note on preferring local `.bzl` source over
WebFetch once a repo is already fetched into the Bazel cache).

## Resolution

Install a local Rust toolchain via `rustup` (this machine already has one
installed for this exact purpose) and regenerate the lockfile with real
`cargo`:

```sh
# One-time, if rustup isn't already installed on this machine:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
source "$HOME/.cargo/env"

# Whenever translator/Cargo.toml changes:
cd translator
cargo generate-lockfile   # or `cargo update <crate>` for a targeted bump
```

Then verify Bazel picks up the new lockfile correctly:

```sh
bazel build //translator:bazelifier
bazel test //translator:bazelifier_test
```

This local `cargo`/`rustc` install is **only** used to produce
`Cargo.lock`. It is never invoked by any Bazel build — actual compilation
always goes through the hermetic `rules_rs`/`llvm` toolchains registered in
`MODULE.bazel`. Keeping rustup installed is a deliberate, standing decision
(not a temporary workaround) since this will recur every time the
translator's dependencies change.
