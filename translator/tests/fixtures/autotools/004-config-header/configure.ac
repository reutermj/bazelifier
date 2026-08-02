# The capability under test is the Autotools config-header path: an
# AC_CONFIG_HEADERS template read from config.status, expanded through the
# shared cc_config catalog, and asserted against the consumer's toolchain.
#
# Until this fixture existed that path was UNIT-ONLY. plan_config_header and
# parse_config_headers had tests over inputs we wrote; nothing ran a real
# autoconf, read a real config.status, or built the result. xz exercises it
# but xz is a corpus project, not a fixture, and it escalates ~137 macros —
# so a failure there is buried rather than pointed at.
#
# Deliberately small and FULLY MAPPED: every macro here has a catalog probe,
# so this fixture converts with no escalation and goes green on its own. The
# escalating direction (a macro the catalog does not cover) is what xz
# covers; both are needed and this is the one that can be a fixture.
#
# DO NOT add a macro without a catalog probe to "make it more realistic" —
# that would turn a green fixture into a permanently red one and lose the
# no-escalation direction entirely.
# Bug address and URL supplied so PACKAGE_BUGREPORT and PACKAGE_URL are real
# substitutions. Left off AC_INIT they are EMPTY, and an empty make variable
# is the absence of a value rather than a value — the frontend escalates it,
# correctly, which would make this fixture red for a reason it is not testing.
AC_INIT([confhdr], [2.1], [bugs@example.invalid], [confhdr], [https://example.invalid/confhdr])
AM_INIT_AUTOMAKE([foreign])
AC_CONFIG_HEADERS([config.h])
AC_PROG_CC
AC_CHECK_HEADERS([stdlib.h string.h unistd.h])
AC_CONFIG_FILES([Makefile])
AC_OUTPUT
