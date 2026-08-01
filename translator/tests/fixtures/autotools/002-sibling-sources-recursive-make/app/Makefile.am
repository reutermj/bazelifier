AM_CPPFLAGS = -I$(top_srcdir)/common

bin_PROGRAMS = tool
# The sibling source, compiled from app/ across the recursion boundary. This
# is the line the fixture exists for.
tool_SOURCES = main.c ../common/util.c
