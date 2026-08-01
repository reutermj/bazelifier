# The capability under test is bzl-kga: a module root WIDER than the
# autotools project directory.
#
# The project is proj/, but it compiles ../shared/helper.c, which sits outside
# proj/ and inside the deliverable. A Bazel label cannot reach above its own
# module, so the generated module must root at the fixture directory rather
# than at proj/ — and every path, including proj/src/main.c, is then rewritten
# against that wider root.
#
# The mirror of this case is the CMake fixture 006-sibling-sources, and the
# capped case is 008-sources-outside-deliverable-root. What decides which one
# applies is the DECLARED deliverable root, not where the file sits on disk:
# with deliverable_root left at proj/, this same project escalates instead.
#
# DO NOT "simplify" this by moving shared/ into proj/ or by widening the
# AC_CONFIG_FILES to cover it: the source outside the project root IS the case
# under test, and this file is immutable test input.
AC_INIT([widening], [1.0])
AM_INIT_AUTOMAKE([foreign subdir-objects])
AC_PROG_CC
AC_CONFIG_FILES([Makefile])
AC_OUTPUT
