# `cc_config` is supplied by a flag, not shipped in the validation tarball

## The symptom

Unpack `validation_workspace.tar` outside the repo, build any fixture that
reproduces a `configure_file` config header (012–015, json-c), and Bazel
fails before doing any work:

```
ERROR: Error computing the main repository mapping: in module dependency
chain <root> -> configure_file_fixture@_ -> cc_config@0.0.0: module
cc_config@0.0.0 not found in registries:
* https://bcr.bazel.build/modules/cc_config/0.0.0/MODULE.bazel: not found
```

**This is the expected state of the tarball.** It is not a packaging bug.

## The fix is on the invocation

```sh
bazel test //:all_ground_truth_comparisons \
    --override_module=cc_config=<bazelifier-checkout>/cc_config
```

`cc_config` stays in bazelifier's normal source tree and is referenced from
the validation run by flag. It is not published to any registry yet
(bzl-fxa.4 tracks that), and until it is, the flag is how the unpacked
workspace resolves it.

## Why not "just" put it in the tarball

Two fixes suggest themselves at the moment you hit the error, and both are
wrong in the same way — they trade the property the unpack step exists to
prove for a shorter command line:

- **Emit `bazel_dep(cc_config)` + `local_path_override` into the generated
  root `MODULE.bazel`.** The override needs a path to a bazelifier checkout,
  so the "portable" deliverable now only works on the machine that produced
  it. The tarball is supposed to be path-free.
- **Stage a copy of `cc_config/` inside the tarball.** Now every conversion
  carries its own fork of the probe catalog, and they drift. `cc_config` is
  deliberately *one shared module* that converted projects reference rather
  than redeclare — see
  [configure-file-and-toolchain-probes.md](../architecture/configure-file-and-toolchain-probes.md).

The flag models the real end state: a consumer resolves `cc_config` from a
registry as an ordinary third-party `bazel_dep`. Overriding a genuine
third-party dep to a local checkout is the standard Bzlmod dev-mode
mechanism, and — this is the part that matters for build-verification — it
does **not** reference bazelifier's own module, so it doesn't weaken the
independence claim the way inheriting from bazelifier's `MODULE.bazel`
would.

## Why this needed writing down

The decision was already documented, correctly, in
[build-verification.md](../architecture/build-verification.md) under "Unpack,
fully outside this repo." It still got re-litigated: an agent hit the module
resolution error while onboarding json-c, diagnosed it as a packaging gap,
filed a P1 bug, and started implementing the tarball-staging version before
being corrected.

The doc wasn't wrong — it was just nowhere near the point of failure. You
hit this error from *inside the unpacked workspace*, where none of this
repo's docs are in view, and the natural next move is to edit the generating
rule. So the rationale now lives at both places the mistake gets made:

- a "deliberately NOT staged / do not fix it this way" block in
  `translator/build_defs/validation_workspace.bzl`'s module docstring, and
- a comment emitted **into the generated root `MODULE.bazel` itself**, which
  ships in the tarball — Bazel names that file in the dependency chain it
  prints, so the answer is readable from where the reader actually is.

Both halves are pinned by
`//translator/tests:root_module_cc_config_note_test`, which asserts the note
is present *and* that no `cc_config` override is ever emitted. The negative
half is the one that matters: without it, the next person to hit the error
quietly bakes a checkout path into the deliverable and nothing notices.
