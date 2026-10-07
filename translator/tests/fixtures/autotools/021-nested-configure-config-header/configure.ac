# Two nested configures (AC_CONFIG_SUBDIRS), as Open MPI runs romio341's and
# the bundled PMIx's: `sub` writes a config header the program compiles
# against; `unused` writes one nothing includes.
AC_INIT([nested], [1.0])
AM_INIT_AUTOMAKE([foreign])
AC_PROG_CC
AC_CONFIG_SUBDIRS([sub unused])
AC_CONFIG_FILES([Makefile])
AC_OUTPUT
