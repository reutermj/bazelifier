# Translating a CTest shell suite: three traps

json-c drives 28 tests through shell scripts rather than binaries, and
translating them to `sh_test` hits three problems in a row. Each looks like
the last one not being fixed, which is what makes them worth writing down.

The harness (`tests/test-defs.sh`) does two things that matter:

```sh
filename=$(basename "$0")     # after sourcing, unconditionally
...
eval "\"${top_builddir}/${TEST_COMMAND}\"" ...
```

## 1. `$0` is the target name, not the script

`run_output_test $filename` resolves the binary AND the `.expected` from
`basename $0`. Bazel always runs an `sh_test` through a wrapper named for
the TARGET, so `$0` is `test_cast_ctest`, and the harness looks for
`test_cast_ctest.expected` — which does not exist.

Presetting `filename` in `env` does not work: line 13 overwrites it after
sourcing. Naming the target `test_cast.test` collides with the source file.

**What works:** a generated shim that `exec`s the real script, so `$0`
becomes the script's own path and the derivation is correct again.

## 2. The binary is not where the harness looks

It runs `$top_builddir/tests/<binary>`. Bazel puts binaries at the module
root, so `top_builddir` has no directory that satisfies it — a genrule has
to stage them into `<name>_bin/tests/`.

## 3. Staging breaks the RUNPATH

A copied binary loses the `$ORIGIN`-relative RUNPATH that found
`libjson-c_shared.so` in runfiles, and fails with "error while loading
shared libraries". Setting `LD_LIBRARY_PATH` in the rule's `env` does not
help — `$$PWD` is not expanded there. It has to be set inside the shim,
where `$PWD` is the test's real working directory.

## How to notice quickly

The symptom is the same at every stage — the harness diffs against an
`.expected` and reports an empty exit status, because the binary never ran.
Read the diff's LEFT path: if it names `<target>.expected` rather than
`<script>.expected` you are at trap 1; if it names the right file but the
output is empty you are at 2 or 3, and the standalone run tells you which:

```sh
LD_LIBRARY_PATH=<runfiles> <staged>/tests/<binary> <srcdir>
```
