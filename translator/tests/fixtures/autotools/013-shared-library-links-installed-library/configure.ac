# A SHARED library that links a library it does not build — the third
# member of the cross-module set (011 installs, 012's program links). A
# shared library carries the edge differently from a program: Bazel's
# cc_library has no dynamic_deps, so it goes on the cc_shared_library
# wrapper, and without it Bazel links the dependency's archive INTO this
# library and refuses the first binary that uses both ("Two shared
# libraries in dependencies link the same library statically"). PMIx's
# libpmix on the converted hwloc and libevent is the corpus instance.
AC_INIT([use-greet], [0.1])
AM_INIT_AUTOMAKE([foreign subdir-objects])
AC_PROG_CC
AM_PROG_AR
LT_INIT
PKG_CHECK_MODULES([GREET], [greet >= 1.0])
AC_CONFIG_FILES([Makefile])
AC_OUTPUT
