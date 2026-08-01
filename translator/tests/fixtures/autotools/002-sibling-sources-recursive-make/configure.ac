# The capability under test is bzl-yjn.7: RECURSIVE make, where a
# subdirectory compiles a source from a SIBLING directory.
#
# app/Makefile.am builds `tool` from app/main.c AND ../common/util.c. Because
# automake recurses, make runs app/'s commands with app/ as the working
# directory, so the command stream reports the sibling source as
#
#     gcc ... -c -o util.o ../common/util.c
#
# `../common/util.c` is neither absolute nor module-relative. A frontend that
# records command paths verbatim emits it into `srcs`, where codegen's
# module-relative assertion refuses it — correctly, since a Bazel label cannot
# reach above its own module. Every path must first be resolved against the
# directory its command ran in, which make announces with "Entering directory".
#
# Measured on real projects: xz has 26 such paths and expat 14, while GNU hello
# has none. This is the shape that separates a single-directory autotools
# project from a normal multi-directory one.
#
# DO NOT "simplify" this by moving util.c into app/ or by flattening to one
# Makefile.am: the sibling compile across a recursion boundary IS the case
# under test, and this file is immutable test input.
AC_INIT([sibling], [1.0])
AM_INIT_AUTOMAKE([foreign])
AC_PROG_CC
AC_PROG_RANLIB
AM_PROG_AR
AC_CONFIG_FILES([Makefile common/Makefile app/Makefile])
AC_OUTPUT
