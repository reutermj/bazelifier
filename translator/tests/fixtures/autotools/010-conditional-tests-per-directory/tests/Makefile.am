# An unconditional program PLUS a conditional one in the same variable.
# That mix is what makes automake emit the `am__EXEEXT_N` indirection --
# a conditional alone is inlined as `@COND_TRUE@check_PROGRAMS = ...`.
check_PROGRAMS = check_real
if BUILD_CHECKS
check_PROGRAMS += check_extra
endif
check_real_SOURCES = check_real.c
check_extra_SOURCES = check_extra.c
TESTS = $(check_PROGRAMS)
