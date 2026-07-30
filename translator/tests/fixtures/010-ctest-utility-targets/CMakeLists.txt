cmake_minimum_required(VERSION 3.20)
project(ctest_utility_targets CXX)

# The capability under test: include(CTest) injects ~28 UTILITY "dashboard"
# targets (Continuous, Experimental, Nightly*, ...) into the File API
# codemodel. Before the provenance filter, each produced its own
# unsupported_target_needs_attention item — 28 escalations for CMake's own
# machinery, none of which is real work, burying any genuine gap. The
# translator now recognizes them by provenance (defined under CMAKE_ROOT,
# no artifacts, no dependents) and drops them SILENTLY. So this fixture,
# despite triggering all those injected targets, must convert with ZERO
# needs_attention items and validate green. A regression that stopped
# dropping them would flood needs_attention and turn this fixture red.
#
# See docs/lore/cmake-include-ctest-injects-utility-targets.md. The
# aggregated fallback for a project's OWN inert convenience targets
# (add_custom_target that CMake did not inject) is covered by unit tests
# rather than here, because that path DOES emit an escalation and a fixture
# with an unresolved escalation would be red — green is the only passing
# state for a fixture.
#
# DO NOT resolve a red conversion by removing include(CTest): it is
# immutable test input, and a real project using CTest is the normal case
# the translator must handle.
include(CTest)

# A plain library + executable, no headers, so the ONLY thing that could
# escalate is the CTest machinery — isolating what this fixture tests. calc
# exposes its one function via a forward declaration in main.cpp rather than
# a header, deliberately, to avoid the header-visibility escalation (covered
# by fixture 003) muddying this one.
add_library(calc src/calc.cpp)

add_executable(app src/main.cpp)
target_link_libraries(app PRIVATE calc)
