# Same shape, so automake gives THIS directory an `am__EXEEXT_1` too -- the
# same variable name, a different value. examples/ declares no TESTS.
noinst_PROGRAMS = demo_example
if BUILD_EXAMPLES
noinst_PROGRAMS += demo_extra
endif
demo_example_SOURCES = demo_example.c
demo_extra_SOURCES = demo_extra.c
