# Pinned URLs rot, and the downloader config is the fix

**Symptom.** On a machine whose Bazel repository cache is cold, every build
fails in analysis, before any of our code runs:

    no such package '@@rules_rs++toolchains+rustc_linux_x86_64_1_92_0//':
    Error downloading [https://archive.ubuntu.com/ubuntu/pool/main/z/zlib/
    zlib1g_1.3.dfsg-3.1ubuntu2.1_amd64.deb] ... GET returned 404 Not Found

A warm machine never sees it, because a sha256-pinned download is served
from the content-addressed repository cache without touching the network.
That is why it looks like it appeared overnight.

**Cause.** rules_rs 0.0.98 fetches the Rust toolchain's zlib as an Ubuntu
`.deb` by URL (`rs/private/rustc_repository.bzl`, `_LINUX_ZLIB`). Ubuntu
DELETES superseded package versions from `archive.ubuntu.com` (and
`ports.ubuntu.com`), so any URL naming an exact version eventually 404s.

**Fix.** `bazel_downloader.cfg` at the repo root, loaded by `.bazelrc`'s
`common --downloader_config`, rewrites the URL to `snapshot.ubuntu.com`,
which keeps every version ever published at a timestamped path. The sha256
in rules_rs still pins the bytes, so the mirror cannot change what is built.
Measured identical 2026-10-06.

The arm64 copy has no such mirror (`snapshot.ubuntu.com` has no
`ubuntu-ports`), so an arm64 host needs a rules_rs upgrade instead.

**Same session, same shape:** `ftp.gnu.org` timed out on every connection
for hours while `mirrors.kernel.org/gnu/` served byte-identical tarballs.
The config lists both rewrites, canonical first, so the mirror is only a
fallback.

**Why not patch MODULE.bazel or the ruleset.** The URL is inside a fetched
ruleset (see never-mutate-bazels-external-repo-cache.md), and bumping
rules_rs drags in a toolchain and `Cargo.lock` change
(docs/runbooks/001-regenerate-translator-cargo-lock.md) to fix what is a
hosting problem. A downloader rewrite is one line per dead host, keeps the
pinned hash authoritative, and is deleted when the upstream moves on.
