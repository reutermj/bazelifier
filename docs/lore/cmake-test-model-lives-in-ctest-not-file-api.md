# The test model is in CTest, not the File API

## What we hit

Getting tinyxml2's `xmltest` verified meant recovering three things: which
binary the test runs, the working directory it needs, and how CMake decides
it passed (`xmltest` returns `gFail`, so exit 0 = pass — but it also prints
`Pass N, Fail M`, and CMake keys on that string, not the exit code alone).
The obvious first guess — read it from the File API like everything else —
is wrong.

## What's actually true

**The codemodel-v2/cache-v2 File API has no test model.** Query every
object kind (`codemodel-v2`, `cache-v2`, `cmakeFiles-v1`, `toolchains-v1`)
and there is no `add_test`/`ctest` reply. The `*Test` entries that show up
(`NightlyTest`, `ContinuousTest`, ...) are CTest *dashboard* UTILITY targets
(see [cmake-include-ctest-injects-utility-targets.md](cmake-include-ctest-injects-utility-targets.md)),
not the project's registered tests, and `target-xmltest-*.json` is the
executable, not the test registration. `PASS_REGULAR_EXPRESSION`,
`WORKING_DIRECTORY`, and the `add_test` command line are simply absent from
the File API.

**CTest has its own model, and it is queryable as JSON.** After configure
(which the translator already runs), CTest can emit the whole test set,
fully resolved, without running anything:

```
ctest --show-only=json-v1
```

For tinyxml2's `xmltest` this yields (trimmed):

```json
{ "tests": [ {
    "name": "xmltest",
    "command": [ "/abs/build/xmltest" ],
    "properties": [
      { "name": "PASS_REGULAR_EXPRESSION", "value": [", Fail 0"] },
      { "name": "WORKING_DIRECTORY", "value": "/abs/src" }
    ]
} ] }
```

So the two things needed to make the test meaningful — the working
directory and the pass criterion — are the *project's own declarations*,
read and translated, not invented by us. (The same principle as reading
`install(FILES ... TYPE INCLUDE)` for public headers: translate the
declaration, don't guess a policy.) The un-JSON `CTestTestfile.cmake`
carries the same data if the JSON interface is ever unavailable, but it
needs parsing; `--show-only=json-v1` is the structured source.

## Why the exit code alone is a trap here

`xmltest` returns `gFail` — 0 when all pass, nonzero on any failure. That
sounds sufficient, but it also returns **1 when it can't find its data**
(`resources/dream.xml`), printing "Error opening... Is your working
directory..." So if the test binary is run *without* its data staged and
without the working directory set, it fails 1 — and if BOTH the ground-truth
and Bazel binaries are run that way, they fail *identically* and a
stdout/exit diff reads as PASS. The exit code isn't lying; the setup is
load-bearing. Two guards close the hole: stage the runtime data and set the
declared `WORKING_DIRECTORY`, and additionally assert the declared
`PASS_REGULAR_EXPRESSION` (`, Fail 0`) — a silent-failure run says "Error
opening", which cannot match it.

## How to check this sort of thing

Configure into a scratch build dir (enable the project's test option if it
gates tests — tinyxml2 needs `-Dtinyxml2_BUILD_TESTING=ON`), then
`cd <build> && ctest --show-only=json-v1`. It does not build or run the
tests; it prints the registered set. Compare against `CTestTestfile.cmake`
in the same build dir to see the raw form.
