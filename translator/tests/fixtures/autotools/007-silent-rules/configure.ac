AC_INIT([silent], [1.0])
AM_INIT_AUTOMAKE([foreign])
# The construct under test. With silent rules on, make prints "  CC main.o"
# instead of the compile command, and a frontend reading the command stream
# sees nothing at all.
AM_SILENT_RULES([yes])
AC_PROG_CC
AC_CONFIG_FILES([Makefile])
AC_OUTPUT
