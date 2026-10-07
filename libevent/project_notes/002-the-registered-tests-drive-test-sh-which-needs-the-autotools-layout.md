# The ten registered tests are make targets driving `test/test.sh`, which needs the autotools layout

**Applies to:** libevent 2.1.12-stable (`test/include.am`)

automake registers ten `TESTS` (`test_runner_epoll`, `_select`, `_kqueue`,
`_evport`, `_devpoll`, `_poll`, `_win32`, `_timerfd`, `_changelist`,
`_timerfd_changelist`). None is a file: each is a make target whose recipe
runs `$(top_srcdir)/test/test.sh -b <BACKEND>` (or `-b "" -t/-c/-T`), and
`test.sh` finds `regress`, `test-init`, `test-eof` and their siblings in
its own directory and drives them with `EVENT_NO<BACKEND>` set for every
backend but the one under test. So the escalation's first suggestion — an
`sh_test` whose `srcs` is the script — cannot work as written: the script
must sit beside the binaries, which Bazel puts at the package root.

The reproduction that works lays the automake tree out in a scratch
directory (symlink each binary to `test/<name>`, put `test.sh` and
`check-dumpevents.py` beside them) and runs the recipe's own arguments;
`run_layout_script_test.sh` in the module does that. A backend this
platform lacks (kqueue, evport, devpoll, win32) makes `test-init` fail and
the script report "Skipping test" with exit 0, which is what upstream's
`make check` sees too.

Two things about `test/regress` as this module builds it:

- The ground truth was built on a host with OpenSSL and zlib, so its
  `regress` includes `regress_ssl.c` and `regress_zlib.c`. No converted
  module provides either, so the module's `regress` is built without them
  (`enable_openssl` defaults off) and its ground-truth comparison is
  recorded as omitted — the one visible difference is `bufferevent_zlib`
  running upstream and skipped here.
- `regress` with no arguments runs for well over the comparison's bounded
  window; that is fine, since the comparison is omitted for the reason
  above and the ten runners are what validate it.

*(History: the first conversion, 2026-09-18, carried Open MPI's
`--disable-libevent-regress`, so `regress` was never built and this note
said the runners could not be reproduced at all. Converting libevent as it
ships builds it.)*
