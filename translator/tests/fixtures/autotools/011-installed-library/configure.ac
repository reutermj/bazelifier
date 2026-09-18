# The library half of the cross-module pair. It installs what a downstream
# configure needs to find it — a header, a libtool shared library and a
# pkg-config file — and nothing else about it is special. Fixture 012
# consumes it through pkg-config; the pair exists so a converted module can
# depend on another converted module, and so the escalation fires when the
# dependency is missing.
AC_INIT([greet], [1.2])
AM_INIT_AUTOMAKE([foreign subdir-objects])
AC_PROG_CC
AM_PROG_AR
LT_INIT
PKG_INSTALLDIR
AC_CONFIG_FILES([Makefile greet.pc])
AC_OUTPUT
