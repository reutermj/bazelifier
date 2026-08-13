# The test harness is autotools-era, and locates data via `$srcdir`

**Applies to:** json-c 0.19 (`tests/*.test`, `tests/test-defs.sh`)

json-c builds with CMake and has no `configure.ac`, but its 29 test wrappers
predate that: they were written for an autotools build and kept through the
migration. `tests/CMakeLists.txt` just runs them —

```cmake
add_test(NAME ${TESTNAME} COMMAND ${PROJECT_SOURCE_DIR}/tests/${TESTNAME}.test)
```

— and never sets the variables they expect, which is why `test-defs.sh`
carries `srcdir=${srcdir-.}` with the comment *"Supply the variable if it
does not exist."*

Two consequences, and the second is the one that costs time.

## Data is located through `$srcdir`, so it is invisible to the build system

`run_output_test` diffs against `"${srcdir}/${TEST_OUTPUT}.expected"`, and
several binaries build their own paths in C:

```c
testdir = argv[1];                                     /* test_util_file.c:148 */
snprintf(filename, sizeof(filename), "%s/valid.json", testdir);
```

`valid.json` appears in **no** `CMakeLists.txt` — only inside that format
string. So nothing the translator reads (the File API, `ctest --show-only`)
can name it. Everything a test reads is staged under `tests/` in this
module; what it reads is not derivable, so check the wrapper.

Beyond the obvious `.expected` file, a wrapper may also need: `test-defs.sh`
(sourced), sibling `.expected` files for its sub-invocations
(`test1Formatted_plain.expected` and friends), the `tests/*.json` fixtures,
and **a second binary** — `test1.test` runs both `test1` and
`test1Formatted`.

## The wrappers cannot be run directly under Bazel

`test-defs.sh` resolves `$top_builddir` to an absolute path and *then*
appends `/tests`:

```sh
top_builddir=`cd ${top_builddir-..} && pwd`
top_builddir=${top_builddir}/tests
```

That `/tests` is CMake's and automake's build layout. Bazel puts a
`cc_binary` at the package root, and no value of `$top_builddir` bridges it
because the resolution happens before the append.

So reproduce the **check** rather than the script — which the escalation
names as an equally good resolution. A small runner taking
`(binary, expected, args...)` and diffing covers 27 of the 28 wrappers.

`test_json_parse_cli` is the exception: it pipes two data files into the
`json_parse` app and asserts a non-zero exit, which that shape does not
express.

## One more thing that is invisible until it fails

25 of the wrappers `export _JSON_C_STRERROR_ENABLE=1`. It switches
`strerror_override.c` to the `ERRNO=EBADF` form the `.expected` files
contain; without it every affected test differs by one line, which reads
like a real output mismatch rather than a missing environment variable.

## Scope

This is json-c, not CMake. `$srcdir` is an autoconf variable — CMake has no
such concept — and of the CMake projects in this corpus only json-c uses it
(31 files, against 0 for zlib, tinyxml2 and fmt, whose shell scripts locate
themselves with `dirname "$0"` like ordinary shell). Do not carry this
expectation to another CMake project; carry it to any project whose tests
came from an autotools build.
