# A libtool LIBRARY linking -lusegreet: creating one makes libtool inherit
# libusegreet.la's dependency_libs into its own, opening every archive they
# name. A program link does not (link_all_deplibs=no on Linux), which is why
# this is a library and the program links it.
lib_LTLIBRARIES = libuseuse.la
libuseuse_la_SOURCES = src/useuse.c
libuseuse_la_LIBADD = -lusegreet
libuseuse_la_LDFLAGS = -version-info 1:0:0

bin_PROGRAMS = app
app_SOURCES = src/main.c
app_LDADD = libuseuse.la
