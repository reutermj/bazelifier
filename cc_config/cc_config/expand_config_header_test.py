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


class TernaryUndefTest(unittest.TestCase):
    """`HAVE_DECL_*` is autoconf's one TERNARY config macro.

    Its template says so: "Define to 1 if you have the declaration of `X',
    and to 0 if you don't." autoconf ALWAYS defines it — there is no undefined
    state — because projects test it two ways in the same file:

        #ifdef HAVE_DECL_CPU_SETSIZE     <- presence
        #  if 0 == HAVE_DECL_CPU_SETSIZE <- value

    Rendering a false probe as `/* #undef ... */` makes the outer test fail
    where autoconf makes it succeed. libmicrohttpd survives that by luck (its
    inner branch undefs anyway); a project writing `#if HAVE_DECL_X` after an
    `#ifdef` guard would silently take the wrong branch.
    """

    def test_a_false_decl_is_defined_to_zero(self):
        # `values` carries the EMPTY STRING, which is what main() writes for a
        # probe that answered false. An earlier version of this test passed
        # `{}` — a state the pipeline never produces — and so stayed green
        # while the real path still emitted `/* #undef ... */`.
        values = {"HAVE_DECL_CPU_SETSIZE": ""}
        out = expand("#undef HAVE_DECL_CPU_SETSIZE\n", lambda n: False, values)
        self.assertEqual(out, "#define HAVE_DECL_CPU_SETSIZE 0\n")

    def test_a_decl_with_no_probe_at_all_is_also_zero(self):
        # And the name absent from `values` entirely — an unresolved decl.
        # Same answer: autoconf always defines this family.
        out = expand("#undef HAVE_DECL_CPU_SETSIZE\n", lambda n: False, {})
        self.assertEqual(out, "#define HAVE_DECL_CPU_SETSIZE 0\n")

    def test_a_true_decl_is_defined_to_one(self):
        values = {"HAVE_DECL_CPU_SETSIZE": "1"}
        out = expand("#undef HAVE_DECL_CPU_SETSIZE\n", lambda n: True, values)
        self.assertEqual(out, "#define HAVE_DECL_CPU_SETSIZE 1\n")

    def test_an_ordinary_macro_still_undefs(self):
        # The negative, and the one that keeps this narrow: only HAVE_DECL_*
        # is ternary. Everything else must keep the comment form, or every
        # `#ifdef HAVE_FOO` in every project starts taking the true branch.
        out = expand("#undef HAVE_UNISTD_H\n", lambda n: False, {})
        self.assertEqual(out, "/* #undef HAVE_UNISTD_H */\n")

    def test_a_name_merely_containing_have_decl_is_not_ternary(self):
        # Anchored at the start: `MHD_HAVE_DECL_X` is a project's own macro,
        # not autoconf's AC_CHECK_DECLS output.
        out = expand("#undef MHD_HAVE_DECL_THING\n", lambda n: False, {})
        self.assertEqual(out, "/* #undef MHD_HAVE_DECL_THING */\n")


class UnresolvedNames(unittest.TestCase):
    """`#error`, not `#undef`, for a name nothing answered.

    `#undef` asserts the feature is ABSENT. A name with no probe, no value
    and an open escalation is UNKNOWN, and rendering it as a negative
    commits the header to a decision nobody made — json-c's two unresolved
    aliases made `json_inttypes.h` take its "old MS compilers" branch, so
    every source failed with `unknown type name '__int32'` on a Linux
    build, three files from the cause.
    """

    def test_an_unresolved_cmakedefine_errors(self):
        out = expand(
            "#cmakedefine JSON_C_HAVE_STDINT_H @JSON_C_HAVE_STDINT_H@\n",
            lambda n: False,
            {},
            ["JSON_C_HAVE_STDINT_H"],
        )
        self.assertIn("#error", out)
        self.assertIn("JSON_C_HAVE_STDINT_H", out)
        self.assertNotIn(
            "#undef",
            out,
            "an unresolved name must not render as a negative probe result",
        )

    def test_an_unresolved_ac_undef_errors_too(self):
        # Both dialects reference names the same way; the answer cannot
        # depend on which syntax the project happens to write.
        out = expand("#undef HAVE_THING\n", lambda n: False, {}, ["HAVE_THING"])
        self.assertIn("#error", out)
        self.assertNotIn("#undef HAVE_THING", out)

    def test_a_genuinely_false_probe_still_undefs(self):
        # The direction that must NOT change. A probe that ran and answered
        # no is absent, and `#undef` is the correct rendering — if this
        # regressed, every real negative would become a build failure.
        out = expand("#undef HAVE_UNISTD_H\n", lambda n: False, {}, ["SOMETHING_ELSE"])
        self.assertEqual(out, "/* #undef HAVE_UNISTD_H */\n")

    def test_a_resolved_name_is_unaffected(self):
        out = expand("#cmakedefine HAVE_IT\n", lambda n: True, {"HAVE_IT": "1"}, [])
        self.assertEqual(out, "#define HAVE_IT\n")


if __name__ == "__main__":
    unittest.main()
