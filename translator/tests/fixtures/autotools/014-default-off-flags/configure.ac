AC_INIT([flagvalues], [1.0], [bugs@example.invalid], [],
  [https://example.invalid/flagvalues])
AM_INIT_AUTOMAKE([foreign])
AC_PROG_CC
AC_CONFIG_HEADERS([config.h])

# Three build options, none of them a toolchain fact. Two default OFF —
# the shape a probe that only ever DISABLES a flag cannot see, because
# disabling an option that is already off changes nothing.

# Off by default, and the macro is always DEFINED: the flag moves its
# VALUE between 0 and 1 rather than making it appear. PMIx, PRRTE and
# Open MPI write every *_ENABLE_DEBUG this way.
AC_ARG_ENABLE([debug],
  [AS_HELP_STRING([--enable-debug], [build the debugging code])])
AS_IF([test "x$enable_debug" = xyes], [flag_debug=1], [flag_debug=0])
AC_DEFINE_UNQUOTED([FLAGVALUES_ENABLE_DEBUG], [$flag_debug],
  [1 to build the debugging code, 0 otherwise.])

# Off by default, and the macro APPEARS with the flag.
AC_ARG_ENABLE([extra],
  [AS_HELP_STRING([--enable-extra], [build the extra feature])])
AS_IF([test "x$enable_extra" = xyes],
  [AC_DEFINE([HAVE_EXTRA], [1], [Define to 1 for the extra feature.])])

# On by default, and the macro VANISHES without it — the shape fixture 009
# already covers, kept here so the two directions are probed side by side
# and must not claim each other's macros.
AC_ARG_ENABLE([greeting],
  [AS_HELP_STRING([--disable-greeting], [omit the greeting])],
  [], [enable_greeting=yes])
AS_IF([test "x$enable_greeting" = xyes],
  [AC_DEFINE([HAVE_GREETING], [1], [Define to 1 to include the greeting.])])

AC_CONFIG_FILES([Makefile])
AC_OUTPUT
