lib_LTLIBRARIES = libgreet.la
libgreet_la_SOURCES = src/greet.c
libgreet_la_LDFLAGS = -version-info 1:0:0

include_HEADERS = src/greet.h

pkgconfig_DATA = greet.pc

# A binary of its own, so this fixture's ground truth has something to run.
bin_PROGRAMS = greet-demo
greet_demo_SOURCES = src/demo.c
greet_demo_LDADD = libgreet.la
