AC_INIT([percdir], [1.0])
AM_INIT_AUTOMAKE([foreign subdir-objects])
AC_PROG_CC

# Two conditionals, both TRUE. automake numbers its per-conditional
# indirection PER DIRECTORY, so both tests/ and examples/ get an
# `am__EXEEXT_1` -- the same name meaning two different programs. That
# collision is the whole point of this fixture.
AM_CONDITIONAL([BUILD_CHECKS], [true])
AM_CONDITIONAL([BUILD_EXAMPLES], [true])

AC_CONFIG_FILES([Makefile tests/Makefile examples/Makefile])
AC_OUTPUT
