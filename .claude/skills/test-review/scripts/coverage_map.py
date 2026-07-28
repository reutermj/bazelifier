#!/usr/bin/env python3
"""Reports candidates for the two coverage questions a reader can't answer by
eye: which translator functions no test ever names, and whether the fixture
enrolment list in translator/tests/BUILD.bazel is complete.

Candidates, not verdicts. A function absent from every test module may be
covered through its caller, and that can be the right design — `configure`
and `build` are one-line `Command` wrappers whose behavior is CMake's. What
the list is for is the case nobody decided: a function that grew logic after
its caller's test was written, and a fixture directory that exists but was
never enrolled (which fails silently — nothing tests it and nothing says so).

Usage: coverage_map.py [repo_root]
"""

import re
import sys
from pathlib import Path

# A `#[cfg(test)] mod tests {` line and everything to end of file. Every test
# module in this crate is the last item in its file; the script checks that
# assumption rather than assuming it (see _split_test_module).
TEST_MOD_RE = re.compile(r"^#\[cfg\(test\)\]\s*$", re.MULTILINE)
FN_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)", re.MULTILINE)
# Order matters when these are applied: a char literal can hold a quote
# (`text.split('"')` in codegen's tests does), which swallows the rest of the
# file as a string and silently vouches for every function in it.
CHAR_LITERAL_RE = re.compile(r"'(?:[^'\\]|\\.)'")
STRING_LITERAL_RE = re.compile(r'"(?:[^"\\]|\\.)*"', re.DOTALL)
COMMENT_RE = re.compile(r"//[^\n]*")


def _split_test_module(text):
    """Returns (production source, test-module source) for one .rs file."""
    match = TEST_MOD_RE.search(text)
    if not match:
        return text, ""
    return text[: match.start()], text[match.start() :]


def report_untested_functions(src_dir):
    """Functions defined in production code that no test module mentions."""
    tests_text = []
    defined = []
    for path in sorted(src_dir.glob("*.rs")):
        production, tests = _split_test_module(path.read_text())
        tests_text.append(tests)
        for name in FN_RE.findall(production):
            defined.append((path.name, name))

    # Literals and comments are stripped before matching, or prose vouches for
    # code: an assertion on "cmake configure failed" would cover `configure`, a
    # path like "/abs/build/..." would cover `build`, and a test comment saying
    # "see `read_codemodel_reply`" would cover a function no test calls.
    all_tests = "\n".join(tests_text)
    all_tests = CHAR_LITERAL_RE.sub("''", all_tests)
    all_tests = STRING_LITERAL_RE.sub('""', all_tests)
    all_tests = COMMENT_RE.sub("", all_tests)
    # Word-boundary match: `render` must not be satisfied by `render_deps`.
    named = {name for name in {n for _, n in defined} if re.search(rf"\b{re.escape(name)}\b", all_tests)}

    missing = [(f, n) for f, n in defined if n not in named]
    print(f"Scanned {len(defined)} function definitions in {src_dir}")
    if not missing:
        print("  every function is named by some test module")
        return
    print(f"  {len(missing)} named by no test module:")
    for filename, name in missing:
        print(f"    {filename}: {name}")


def report_unenrolled_fixtures(tests_dir):
    """Fixture directories on disk vs. the validation_workspace fixtures list."""
    build_file = tests_dir / "BUILD.bazel"
    enrolled = set(re.findall(r"//translator/tests/fixtures/([^:]+):", build_file.read_text()))
    on_disk = {p.name for p in (tests_dir / "fixtures").iterdir() if p.is_dir()}

    print(f"\nFixtures on disk: {len(on_disk)}; enrolled in {build_file}: {len(enrolled)}")
    for name in sorted(on_disk - enrolled):
        print(f"  NOT ENROLLED (never converted, never tested, nothing reports it): {name}")
    for name in sorted(enrolled - on_disk):
        print(f"  ENROLLED BUT MISSING (the list names a directory that isn't there): {name}")
    if on_disk == enrolled:
        print("  the list is complete")


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    report_untested_functions(root / "translator" / "src")
    report_unenrolled_fixtures(root / "translator" / "tests")


if __name__ == "__main__":
    main()
