bin_PROGRAMS = amtests
amtests_SOURCES = src/main.c

# The three shapes an automake TESTS entry takes, per the classifier in
# autotools.rs. All three in one fixture because the branches are chosen by
# one function and a fixture covering only one proves the least interesting
# part.
check_PROGRAMS = check_direct
check_direct_SOURCES = src/check_direct.c

# 1. A check_PROGRAMS binary run directly -- the MAJORITY shape across real
#    projects (libmicrohttpd, gsl, wget). Must become a real test.
# 2. A shell script that drives the tests itself -- jansson's and xz's shape.
#    Must escalate: it is not a target this module builds.
# 3. A variable the database cannot answer. Must escalate rather than being
#    guessed at or silently dropped.
# 3. A reference automake PASSES THROUGH because the variable is defined in
#    configure.ac via AC_SUBST -- so its value is decided at configure time
#    and make -p reports the literal @-substituted text. The database cannot
#    resolve it to target names, which is the Unresolved branch.
TESTS = $(check_PROGRAMS) src/run-suite.sh $(EXTRA_TEST_LIST)
EXTRA_TEST_LIST = @EXTRA_TEST_LIST@
