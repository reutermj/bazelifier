noinst_LTLIBRARIES = libone.la libtwo.la
libone_la_SOURCES = variant.c
libone_la_CPPFLAGS = -DWHICH=1
libone_la_CFLAGS = -msse4.1
libtwo_la_SOURCES = variant.c
libtwo_la_CPPFLAGS = -DWHICH=2

bin_PROGRAMS = app
app_SOURCES = main.c
app_LDADD = libone.la libtwo.la
