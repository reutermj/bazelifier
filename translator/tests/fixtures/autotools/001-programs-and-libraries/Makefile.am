noinst_LIBRARIES = libgreet.a
libgreet_a_SOURCES = src/greet.c src/greet.h

lib_LTLIBRARIES = libshout.la
libshout_la_SOURCES = src/shout.c

include_HEADERS = src/greet.h

bin_PROGRAMS = greeter
greeter_SOURCES = src/main.c
greeter_LDADD = libgreet.a libshout.la
