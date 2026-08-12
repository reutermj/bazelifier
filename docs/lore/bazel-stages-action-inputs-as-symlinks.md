# Bazel stages an action's inputs as symlinks

Every file in an action's sandbox is a **symlink** into Bazel's execroot, not
a regular file. Measured inside a `ConvertProject` action:

```
entry="test_visit.test" is_symlink=true
entry="parse_flags.h"   is_symlink=true
entry="test_visit.c"    is_symlink=true
```

Code that copies a tree and skips symlinks therefore copies **nothing** under
Bazel, while working perfectly when run by hand over a real checkout.

## What it cost here

`copy_runtime_tree` skipped every symlink, for a good reason: libidn2's
`configure` leaves a `GNUmakefile` wrapper pointing at a path that stops
existing when the action ends, and copying it makes Bazel reject the whole
tree artifact — *"child GNUmakefile is a dangling symbolic link"*.

That guard silently disabled `copy_test_runtime_data` for every corpus
project. json-c shipped none of its 38 `.expected` files and no
`test-defs.sh`, so its test wrappers could not run; tinyxml2's `xmltest` lost
all 9 of its resources. Both looked like translator bugs and neither was.

The fix is to test whether the target **resolves**, not whether the entry is a
link:

```rust
if entry.file_type().is_ok_and(|t| t.is_symlink()) && !child_src.exists() {
    continue;
}
```

`exists()` follows the link, which is what distinguishes libidn2's dangling
wrapper from an ordinary staged input.

## The debugging lesson, which is the more valuable half

The symptom was: **the input is demonstrably present and the output is
demonstrably empty.** A probe inside the action showed 103 entries including
38 `.expected` under the exact path the copier reads, and immediately after
the copy the output had none.

That contradiction was the finding, and it was in hand early. What wasted the
time was continuing to audit the copier — which a unit test then proved
innocent, because the test used real files.

Two rules worth keeping:

- **When output contradicts your model, suspect the inputs before the code.**
  Not the input *values* (those were right) but the input *shape*: regular
  files versus symlinks.
- **A test that constructs its own fixtures does not reproduce the sandbox.**
  `copy_runtime_tree_skips_a_symlink` built a dangling link beside a real
  file and passed throughout. Nothing exercised a *live* link, which is the
  only shape Bazel ever produces. When a function's behaviour depends on how
  files got there, the test has to build them that way.

## What is NOT the lesson

An earlier version of this note blamed Bazel's action caching. That was wrong
and worth stating, because it would misdirect the next person: a real content
change to `translator/src/main.rs` **does** invalidate the conversion action
(the rule declares the binary via `executable = ctx.executable._bazelifier`),
and every instrumented run did re-execute and print.

What actually happened was re-issuing an *identical* `bazel build` after
already consuming its output, getting a cache hit that printed nothing, and
reading that silence as evidence. A repeated identical build is not a second
observation. `INFO: 1 process: 1 internal` means nothing ran.
