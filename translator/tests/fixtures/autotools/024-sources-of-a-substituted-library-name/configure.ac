# A library whose NAME comes from configure, as Open MPI's lib@OPAL_LIB_NAME@.la
# (open-pal): automake canonicalises the variable name before the
# substitution, so its _SOURCES variable keeps the substituted hyphen.
AC_INIT([subname], [1.0], [bugs@example.invalid], [], [https://example.invalid/subname])
AM_INIT_AUTOMAKE([foreign subdir-objects])
AC_PROG_CC
AM_PROG_AR
LT_INIT
MY_NAME=my-lib
AC_SUBST([MY_NAME])
AC_CONFIG_FILES([Makefile])
AC_OUTPUT
