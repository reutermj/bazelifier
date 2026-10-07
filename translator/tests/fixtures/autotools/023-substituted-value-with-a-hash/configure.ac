# An AC_SUBST value that is itself a preprocessor line, substituted into an
# AC_CONFIG_FILES header — romio's mpio.h.in has @DEFINE_HAVE_MPI_GREQUEST@.
AC_INIT([hashsubst], [1.0], [bugs@example.invalid], [], [https://example.invalid/hashsubst])
AM_INIT_AUTOMAKE([foreign])
AC_PROG_CC
DEFINE_FEATURE="#define FEATURE 1"
AC_SUBST([DEFINE_FEATURE])
AC_CONFIG_FILES([Makefile feature.h])
AC_OUTPUT
