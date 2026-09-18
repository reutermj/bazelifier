# The consumer half of the cross-module pair: a program that links a library
# it does not build, found the way most autotools projects find one — through
# pkg-config. Converted with fixture 011 as a dependency it must become a
# module that bazel_deps on 011's; converted without, it must escalate the
# missing dependency by name rather than link a host library.
AC_INIT([greet-user], [0.3])
AM_INIT_AUTOMAKE([foreign subdir-objects])
AC_PROG_CC
PKG_CHECK_MODULES([GREET], [greet >= 1.0])
AC_CONFIG_FILES([Makefile])
AC_OUTPUT
