noinst_LTLIBRARIES = libpart_noinst.la
libpart_noinst_la_SOURCES = part.c

# Open MPI's shape: the installable name, pointed at the noinst build.
all-local:
	rm -f libpart.la; $(LN_S) libpart_noinst.la libpart.la

bin_PROGRAMS = app
app_SOURCES = main.c
app_LDADD = libpart.la
app_DEPENDENCIES = libpart_noinst.la
