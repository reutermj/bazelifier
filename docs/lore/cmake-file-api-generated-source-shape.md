# What the File API actually reports for a generated source

## What we hit

`cmake_api.rs`'s `TargetSource::is_generated` says a generated source is
"reported as an ABSOLUTE path into the CMake build directory," and
`generated_sources_needs_attention` repeats that to the agent. Probing a
real project to check it (CMake 3.28.3 + Ninja) turned up two things that
neither the code nor
[cmake-frontend.md](../architecture/cmake-frontend.md) mentions.

## What's actually true

**Absolute is a consequence of where the build directory is, not a
property of generated sources.** With the build directory *inside* the
source tree — `cmake -B build -S .`, which is what a developer at a
terminal types — the same source comes back relative:

```
{'path': 'build/gen.cpp',  'isGenerated': True}      # cmake -B <srcdir>/build
{'path': '/tmp/out/gen.cpp', 'isGenerated': True}    # cmake -B <outside srcdir>
```

The translator always configures into a scratch directory outside the
project (`--build-dir`, and `convert_cmake_project.bzl` declares it as a
separate output), so the absolute form is the only one it can observe.
That makes the claim true where it is written and false as a general
statement about CMake — worth knowing before someone "simplifies"
generated-source handling to a `path.is_absolute()` check, which would
then hold only for how *this* pipeline invokes CMake.

**Every `add_custom_command()` output arrives with a phantom sibling.**
CMake reports an extra `<output>.rule` source alongside the real one:

```
{'path': '/tmp/out/gen.cpp',      'isGenerated': True}
{'path': '/tmp/out/gen.cpp.rule', 'isGenerated': True}
```

`.rule` is Ninja/Makefile bookkeeping — it names no file on disk. It is
caught by the same `is_generated` filter as the real output, so it never
reaches `srcs`, but it *does* get listed in the escalation, where it reads
as a second missing input an agent is expected to find a `genrule` for. A
fixture exercising generated sources should expect it (and filtering
`.rule` out of the escalation's file list is probably the right fix).

## How to check this sort of thing

The File API answers directly, without building: write an empty
`<build>/.cmake/api/v1/query/codemodel-v2`, run `cmake -G Ninja -B <build>
-S <src>`, and read `<build>/.cmake/api/v1/reply/target-*.json`. One
gotcha on the way in — `project(p VERSION 1.2.3 CXX)` is an error;
`VERSION` forces the explicit form, `project(p VERSION 1.2.3 LANGUAGES
CXX)`.
