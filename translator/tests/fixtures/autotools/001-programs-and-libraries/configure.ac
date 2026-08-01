# First Autotools fixture. Deliberately covers all three automake target
# primaries at once, because the point is to exercise the frontend/codegen
# boundary rather than any one construct:
#
#   noinst_LIBRARIES  -> a static archive (ar cru)
#   lib_LTLIBRARIES   -> a libtool library, the concept with no CMake analogue
#   bin_PROGRAMS      -> an executable linking both
#
# It also declares a public header via include_HEADERS, which is how automake
# states what CMake states with install(FILES ... DESTINATION include).
AC_INIT([greeter], [1.0])
AM_INIT_AUTOMAKE([foreign subdir-objects])
AC_PROG_CC
AC_PROG_RANLIB
AM_PROG_AR
LT_INIT
AC_CONFIG_FILES([Makefile])
AC_OUTPUT
