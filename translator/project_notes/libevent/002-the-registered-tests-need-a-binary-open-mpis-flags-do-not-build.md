# The ten registered tests drive `test/regress`, which Open MPI's configure flags do not build

**Applies to:** libevent 2.1.12-stable, as converted for the Open MPI chain

automake registers ten `TESTS` (`test_runner_epoll`, `_select`, `_kqueue`,
`_evport`, `_devpoll`, `_poll`, `_win32`, `_timerfd`, `_changelist`,
`_timerfd_changelist`). None is a file: each is a make target running
`test/test.sh -b <BACKEND>`, and `test.sh` runs `test/regress` under an
`EVENT_NO<backend>` environment.

`test/regress` is declared `EXTRA_PROGRAMS` and built only when
`BUILD_REGRESS` is set, and this conversion configures libevent the way Open
MPI configures its bundled copy — including `--disable-libevent-regress`
(see `translator/tests/corpus/libevent/BUILD.bazel`). So the binary the
runners need is not part of this build, on purpose, and the runners cannot
be reproduced as `sh_test`s. Record that in the generated `BUILD.bazel` and
close the item; do not add `test/regress` by hand — it is a 33-file test
program with its own generated `regress.gen.c`, and building it would be
converting a configuration this module deliberately does not replicate.

Two things about the comparisons that DO run, since they look wrong on
first sight:

- Four of the seven samples never exit on their own (`hello-world` and
  `http-server` listen, `signal-test` waits for SIGINT, `event-read-fifo`
  blocks on a fifo it creates). The comparison runs every binary under a
  bounded window and compares both builds over it, so these pass after
  forty seconds each with a NOTE in the log. That is the intended check for
  a server, not a hang.
- `sample_dns-example` and `sample_http-server` exit immediately with a
  usage line; that is their behaviour with no arguments, and the comparison
  matches it.
