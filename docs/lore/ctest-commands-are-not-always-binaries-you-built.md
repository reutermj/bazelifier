# A CTest command is not always a binary you built

## What `ctest --show-only=json-v1` actually reports

Two real corpus projects, and they are mirror images of each other:

**tinyxml2** — command is a built executable in the BUILD tree, working
directory is in the SOURCE tree:

```json
"command": [ "/abs/build/xmltest" ],
"properties": [ { "name": "WORKING_DIRECTORY", "value": "/abs/src" } ]
```

**json-c** — command is a checked-in shell script in the SOURCE tree, working
directory is in the BUILD tree:

```json
"command": [ "<src>/tests/test1.test" ],
"properties": [ { "name": "WORKING_DIRECTORY", "value": "<build>/tests" } ]
```

All 28 of json-c's tests take the second form. Zero take the first.

## Why this broke the translator twice over

`ctest_reply_to_tests` reduced the command to its **basename** and assumed the
result named a `cc_binary` the module emits. Both halves are wrong for json-c:

1. The basename discards the directory, so `tests/test1.test` became
   `test1.test`.
2. There is no target of that name at all — it's a data file.

The generated `sh_test` therefore referenced `:test1.test`, a label that
cannot resolve. It failed only at analysis time in the unpacked workspace:

```
ERROR: RunfilesTree external/json-c+/test1_test.runfiles failed:
missing input file '@@json-c+//:test1.test'
```

That message says nothing about the construct that wasn't understood, and it
took down `bazel build //...` for the entire module — 28 targets at once.

## The distinguishing test is the target list, not the path

Tempting heuristics that all fail:

- **Extension.** `.test` looks script-ish, but the extension is arbitrary —
  a project can register `add_test(NAME x COMMAND some_binary)` where the
  binary has no extension, or a script with none either.
- **Which tree it lives in.** "Build tree = binary, source tree = script"
  happens to hold for these two projects and is not a rule. A CMake project
  can `add_test` a prebuilt tool checked into the source tree, and a
  `configure_file`d script lands in the build tree.

The only thing that actually answers "can I emit an `sh_test` wrapping this?"
is whether the command's basename matches an **executable target the
translator emitted**. That's what `partition_tests_by_buildable_command`
checks.

## Why escalate instead of translating the script

The translator can't mechanically derive the wiring. Each json-c `.test`
script sources `test-defs.sh`, derives `srcdir` from `$0`, copies fixture
files into place, and diffs against a checked-in `.expected`. And CTest's
`WORKING_DIRECTORY` for those tests points into the **build tree**, which has
no counterpart in the converted module — so it can't be rebased and isn't
carried into the generated output at all.

Guessing that wiring is how you get a test that runs, passes, and checks
nothing. Escalating states the gap and hands an agent the command path,
which is the one thing it cannot recover from the test name.

Worth being clear about the cost: **escalating loses the tests.** json-c's
suite is far stronger equivalence evidence than the stdout comparison on
`json_parse`, and until the agent stage closes that item, none of it runs.
An absent test is invisible in a way a failing one isn't — nothing reports
it. That is a reason to close the item, not a reason to call the escalation
sufficient.

## The command has to be rebased too

`rebase_tests_to_module_root` originally only rebased `working_directory`.
The command is quoted verbatim into the escalation an agent reads in the
unpacked workspace, where this machine's absolute path — under a Bazel
sandbox hash that changes every run — names nothing. It's now rebased as
well, with one deliberate asymmetry: a command *outside* the module root
stays absolute, where `working_directory` would be emptied. Emptying it would
leave the escalation naming no command at all.
