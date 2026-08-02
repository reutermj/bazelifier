AUTOMAKE_OPTIONS = foreign
bin_PROGRAMS = spliced
spliced_SOURCES = src_main.c
spliced_CPPFLAGS = -I$(builddir)/gl

BUILT_SOURCES = gl/mystring.h
CLEANFILES = gl/mystring.h

# The generation recipe gnulib uses: substitutions, then a `r` splice of a
# whole helper file at a marker comment, then an atomic rename.
gl/mystring.h: $(srcdir)/gl/mystring.in.h $(srcdir)/gl/helper-defs.h
	$(AM_V_GEN)$(MKDIR_P) gl && \
	sed -e 's|@''GUARD_PREFIX''@|GL|g' \
	    -e 's|@''PACKAGE_VERSION''@|$(PACKAGE_VERSION)|g' \
	    -e '/definitions of HELPER_MACROS/r $(srcdir)/gl/helper-defs.h' \
	    < $(srcdir)/gl/mystring.in.h > gl/mystring.h-t && \
	mv gl/mystring.h-t gl/mystring.h
