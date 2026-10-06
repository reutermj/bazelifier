# A link that names -lgreet with no -L of its own: tool/ overrides LDFLAGS,
# as Open MPI's test/mpool does, and the directory comes from libmid.la's
# dependency_libs, which libtool adds to the link.
AC_INIT([inherited-dirs], [1.0])
AM_INIT_AUTOMAKE([foreign subdir-objects])
AC_PROG_CC
AM_PROG_AR
LT_INIT
AC_CONFIG_FILES([Makefile tool/Makefile])
AC_OUTPUT
