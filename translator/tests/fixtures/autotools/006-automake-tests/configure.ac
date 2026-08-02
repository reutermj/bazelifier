AC_INIT([amtests], [1.0])
AM_INIT_AUTOMAKE([foreign])
AC_PROG_CC
# Deliberately substituted with a variable reference make cannot resolve:
# this is what drives the classifier's Unresolved branch.
AC_SUBST([EXTRA_TEST_LIST], ['$(NOT_A_REAL_VAR)'])
AC_CONFIG_FILES([Makefile])
AC_OUTPUT
