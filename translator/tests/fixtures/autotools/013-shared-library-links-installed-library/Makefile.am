lib_LTLIBRARIES = libusegreet.la
libusegreet_la_SOURCES = src/usegreet.c src/usegreet.h
libusegreet_la_CFLAGS = $(GREET_CFLAGS)
libusegreet_la_LIBADD = $(GREET_LIBS)
libusegreet_la_LDFLAGS = -version-info 1:0:0

# A binary through the shared library, so the ground truth has something to
# run and the comparison exercises BOTH edges: app -> libusegreet -> libgreet.
bin_PROGRAMS = app
app_SOURCES = src/main.c
app_LDADD = libusegreet.la
