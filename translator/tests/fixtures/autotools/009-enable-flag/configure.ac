AC_INIT([flagged], [1.0])
AM_INIT_AUTOMAKE([foreign])
AC_PROG_CC
AC_CONFIG_HEADERS([config.h])

# A build OPTION, not a toolchain probe. Nothing about the host decides
# this — only whether the person building passed the flag. Defaults ON,
# which is what the conversion must reproduce.
AC_ARG_ENABLE([greeting],
  [AS_HELP_STRING([--disable-greeting], [omit the greeting])],
  [], [enable_greeting=yes])
AS_IF([test "x$enable_greeting" = xyes],
  [AC_DEFINE([HAVE_GREETING], [1], [Define to 1 to include the greeting.])])

AC_CONFIG_FILES([Makefile])
AC_OUTPUT
