# The container dies mid-build, and nothing logs why

## The symptom

A `bazel build` of the validation workspace kills the container. Not a Bazel
error — the shell dies, the session drops, and on restart everything looks
fine. `/tmp` is empty, so any unpacked workspace is gone too.

The misleading part is the evidence trail:

- Bazel's `java.log` says nothing. The JVM was not the process killed.
- `cat /sys/fs/cgroup/memory.events` reports `oom_kill 0`, which reads as
  "not an OOM". It is not: the counter lives in the cgroup, and the cgroup
  dies with the container.
- `dmesg` is not readable from inside, so the kernel's own record is out of
  reach.

So the one hypothesis with no supporting evidence is the correct one.

## The cause

`bazel info --show_make_env` says it plainly:

```
local_resources: RAM=31220MB, CPU=32.0
```

Bazel sizes its scheduler from the host. This container is 32 cores and
30 GB — a ratio that is fine for ordinary code and wrong for this workload,
because **every generated module builds libc++ and libunwind from source**
via the llvm toolchain. That is a hard hermeticity requirement (CLAUDE.md),
and it means a large share of the action graph is C++ compiles rather than
cheap file operations.

There was no `.bazelrc` at all, so nothing capped it.

## The multiplier, which is the part that actually bites

The Autotools frontend runs `make -j<cores>` **inside a Bazel action** to
capture ground truth. So the two schedulers multiply: Bazel runs N actions,
each spawning up to `<cores>` compilers. Capping either one alone leaves the
product unbounded.

`build()`'s comment already warned that bare `-j` "starves the machine" and
passed the core count explicitly — correct in isolation, and still wrong one
level up.

The CMake frontend does NOT contribute: `cmake --build` without `--parallel`
is serial. Worth knowing before "fixing" it for symmetry.

## The fix

`.bazelrc` caps the scheduler, and `build()` halves the inner job count. Both
halves are needed.

`--local_resources=memory=` is the only knob that limits concurrency by
memory rather than by action count; `--jobs` alone still lets 16 memory-heavy
compiles run at once.

Measured after: a clean build of all 1139 actions finishes in 171s and holds
at 5 GB — well inside the cap, and no crash.

## If it recurs

Check the budget is actually applied, since `--show_make_env` reports the
HOST's capacity either way and looks unchanged:

```sh
bazel build --announce_rc <target> 2>&1 | grep "'build' options"
```

That prints the flags Bazel inherited from the rc file.
