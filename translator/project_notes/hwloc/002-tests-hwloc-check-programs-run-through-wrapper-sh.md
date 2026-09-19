# `tests/hwloc`'s check programs run through `wrapper.sh`, and two of them need it

**Applies to:** hwloc 2.7.1 (`tests/hwloc/Makefile.am`, `LOG_COMPILER`)

The translator turns every `check_PROGRAMS` entry in `TESTS` into an
`sh_test` that runs the binary bare. For 43 of the 45 programs under
`tests/hwloc` that passes, and it is still not what the build does:
`tests/hwloc/Makefile.am` sets `LOG_COMPILER = $(builddir)/wrapper.sh`, so
automake runs every one through the configure-generated
`tests/hwloc/wrapper.sh`, which exports `HWLOC_TOP_SRCDIR`,
`HWLOC_PLUGINS_PATH`, `HWLOC_DEBUG_CHECK` and `HWLOC_LIBXML_CLEANUP` and,
for `xmlbuffer` alone, runs the binary four times with its libxml
import/export on/off arguments.

Run bare, `hwloc_get_obj_with_same_locality` exits with
`HWLOC_TOP_SRCDIR missing in the environment` and `xmlbuffer` refuses its
missing arguments — both look like broken tests and are a missing wrapper.
The resolution is to run the check programs the way automake does:
substitute the wrapper's `@HWLOC_top_srcdir@`/`@HWLOC_top_builddir@` and
hand it the binary at its automake path (`run_hwloc_script_test.sh` in
the module builds that layout). Do not patch the two programs' arguments
into their own rules; the other 43 would then be running differently from
the build for no reason.

The `doc/examples` programs have no `LOG_COMPILER` and run bare correctly.
