"""Unit tests for the parts of expand_config_header the Bazel tests cannot see.

`assert_config_header_test` can only ask whether a needle is PRESENT in a
generated header. Two of sed's `r` behaviours are about how many times and in
what order text appears, so they are invisible to it and pinned here.
"""

import os
import sys
import tempfile
import unittest

# The module under test sits beside this file. Added explicitly because the
# runfiles layout does not put the package directory on sys.path, so a plain
# `import expand_config_header` resolves when run directly and not under Bazel.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from expand_config_header import expand, splice_files


def _write(directory, name, content):
    path = os.path.join(directory, name)
    with open(path, "w") as fh:
        fh.write(content)
    return path


class SpliceTest(unittest.TestCase):
    def test_fires_at_every_matching_line(self):
        # sed's `r` is not once-only. gnulib templates carry the same marker
        # comment more than once, and a first-match-wins implementation
        # produces a header missing the later copies — which still compiles
        # until something between them uses a macro.
        with tempfile.TemporaryDirectory() as d:
            helper = _write(d, "helper.h", "HELPER\n")
            out = splice_files("a\nMARK\nb\nMARK\nc\n", [("MARK", helper)])
            self.assertEqual(out, "a\nMARK\nHELPER\nb\nMARK\nHELPER\nc\n")

    def test_keeps_the_matched_line(self):
        # `r` appends after the line; it does not replace it. Replacing would
        # delete a template comment the project ships.
        with tempfile.TemporaryDirectory() as d:
            helper = _write(d, "helper.h", "X\n")
            self.assertEqual(splice_files("MARK\n", [("MARK", helper)]), "MARK\nX\n")

    def test_applies_splices_in_order(self):
        # Order decides what lands where, and two markers may name one file.
        with tempfile.TemporaryDirectory() as d:
            first = _write(d, "first.h", "FIRST\n")
            second = _write(d, "second.h", "SECOND\n")
            out = splice_files(
                "A\nB\n", [("A", first), ("B", second)]
            )
            self.assertEqual(out, "A\nFIRST\nB\nSECOND\n")

    def test_a_helper_without_a_trailing_newline_does_not_join_lines(self):
        # A file not ending in \n would otherwise glue itself to the next
        # template line, silently corrupting both.
        with tempfile.TemporaryDirectory() as d:
            helper = _write(d, "helper.h", "NO_EOL")
            out = splice_files("MARK\nafter\n", [("MARK", helper)])
            self.assertEqual(out, "MARK\nNO_EOL\nafter\n")

    def test_splicing_after_expansion_leaves_the_helpers_vars_alone(self):
        # The ordering claim, pinned. libidn2's c++defs.h carries seven @VAR@
        # references that reach the generated header verbatim, because the
        # recipe's `r` commands run in its LAST sed pass. Splicing first would
        # substitute them and emit a header the project's build never does.
        with tempfile.TemporaryDirectory() as d:
            helper = _write(d, "helper.h", "/* @VERSION@ stays */\n")
            values = {"VERSION": "1.2.3"}
            expanded = expand("MARK\n@VERSION@\n", lambda n: n in values, values)
            out = splice_files(expanded, [("MARK", helper)])
            self.assertEqual(out, "MARK\n/* @VERSION@ stays */\n1.2.3\n")


if __name__ == "__main__":
    unittest.main()
