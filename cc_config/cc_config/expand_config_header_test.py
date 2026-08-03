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


class SpacedUndefTest(unittest.TestCase):
    """The spaced `# undef` is written by TWO dialects and the line cannot
    tell them apart, so the caller's resolved names are the discriminator.

    This asymmetry is deliberate and easy to "fix" into a bug: the Rust side's
    undef_names() accepts any whitespace, because it only ever reads the one
    AC_CONFIG_HEADERS template. This expander also runs over gnulib's *.in.h,
    where the same spelling is a header undefining its own guard (bzl-vj5).
    """

    def test_a_resolved_name_is_a_declaration(self):
        # autoconf's AC_USE_SYSTEM_EXTENSIONS. Left as `# undef`, every GNU
        # extension stays off and libidn2 fails on program_invocation_name.
        values = {"_GNU_SOURCE": "1"}
        out = expand(
            "#ifndef _GNU_SOURCE\n# undef _GNU_SOURCE\n#endif\n",
            lambda n: n in values,
            values,
        )
        self.assertEqual(out, "#ifndef _GNU_SOURCE\n#define _GNU_SOURCE 1\n#endif\n")

    def test_an_unresolved_name_is_left_alone(self):
        # gnulib's split double-inclusion guard. The translator never passes
        # this name, so nothing here may touch the line.
        values = {"_GNU_SOURCE": "1"}
        line = "# undef _GL_ALREADY_INCLUDING_LIMITS_H\n"
        self.assertEqual(expand(line, lambda n: n in values, values), line)

    def test_a_nested_undef_is_a_declaration(self):
        # expat and libmicrohttpd both write WORDS_BIGENDIAN this way, two
        # levels deep. Neither is a gnulib project.
        values = {"WORDS_BIGENDIAN": "1"}
        out = expand("#  undef WORDS_BIGENDIAN\n", lambda n: n in values, values)
        self.assertEqual(out, "#define WORDS_BIGENDIAN 1\n")

    def test_a_resolved_but_false_name_is_commented_out(self):
        # Same contract as the unspaced form: a name the probe answered NO for
        # becomes the comment config.status writes, not a surviving directive.
        values = {"WORDS_BIGENDIAN": ""}
        out = expand("#  undef WORDS_BIGENDIAN\n", lambda n: False, values)
        self.assertEqual(out, "/* #undef WORDS_BIGENDIAN */\n")


if __name__ == "__main__":
    unittest.main()
