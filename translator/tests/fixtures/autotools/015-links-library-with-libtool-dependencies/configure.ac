# A library linking a converted library that itself links another converted
# library — the TRANSITIVE edge. 013's libusegreet.la is installed with
# dependency_libs naming libgreet.la by its install path
# (/usr/local/lib/libgreet.la), and libtool follows that path whenever a
# library link names -lusegreet. PRRTE linking PMIx (which links libevent and hwloc)
# is the corpus instance.
AC_INIT([use-use-greet], [0.1])
AM_INIT_AUTOMAKE([foreign subdir-objects])
AC_PROG_CC
LT_INIT
AC_CONFIG_FILES([Makefile])
AC_OUTPUT
