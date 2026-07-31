# The ground-truth `.so` lives at `ground_truth/`, not beside the binary

## The symptom

A ground-truth comparison fails with an exit-code mismatch where the
ground-truth side never ran at all:

```
FAIL: exit code mismatch: ground_truth=127 bazel=0
< .../ground_truth/apps/json_parse: error while loading shared libraries:
    libjson-c.so.5: cannot open shared object file: No such file or directory
```

Exit 127 from the loader means the binary died before `main`. The comparison
then reports a stdout/stderr mismatch too, which is noise — the real failure
is that one side produced no output because it never started.

## The cause

Two staging conventions that only conflict in combination:

- `copy_ground_truth_artifacts` stages the shared-library chain at the
  **`ground_truth/` root**.
- A target CMake builds under a **subdirectory** gets a build-relative
  artifact path, so its binary lands at `ground_truth/<subdir>/<name>`.

`compare_runtime_output.sh` derived `LD_LIBRARY_PATH` from
`dirname "${ground_truth_bin}"` — the *binary's* directory. For a flat
project that is also the `ground_truth/` root, so it worked. For json-c:

```
ground_truth/libjson-c.so.5        <- libraries here
ground_truth/apps/json_parse       <- binary here, LD_LIBRARY_PATH pointed here
```

The fix walks from the binary's directory up to the `ground_truth/` root and
puts every level on `LD_LIBRARY_PATH`, so both layouts resolve.

## Why it survived two fixtures that each cover half of it

This is the part worth remembering. Fixture **016-shared-library** has a
shared library (binary at the root). Fixture **018-subdir-target** has a
subdirectory binary (no shared library). **Both passed.** The bug lives only
in their intersection, and nothing had that shape until a real project did —
json-c, whose `apps/json_parse` links `libjson-c.so.5`.

The comment above the offending line asserted the wrong half outright:

> `copy_ground_truth_artifacts` staged the .so chain into `ground_truth/`
> **alongside this binary**, so its own directory is where the loader must
> look.

"Alongside this binary" was true of every fixture that existed when it was
written and false as soon as one wasn't flat — a checkable claim about where
a file *is*, which is exactly the kind this repo keeps getting wrong (see
CLAUDE.md on stale frames of reference). It reads as justification, so the
next reader trusts it instead of checking the layout.

Fixture **021-subdir-binary-shared-library** now pins the intersection: it
fails with the old script (`exit 127`, same loader error) and passes with the
fix, while 016 and 018 keep passing either way.

## The generalizable lesson

Two fixtures covering two capabilities do not cover their combination. When a
capability is about a *path or a layout*, the combinations are where the
assumptions collide — and each individual fixture will keep passing and tell
you nothing is wrong. A real corpus project is what surfaces these, which is
the argument for the corpus tier existing at all: synthetic fixtures test the
shapes we thought of.
