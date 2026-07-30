cmake_minimum_required(VERSION 3.20)
project(configure_file_unresolved_var_fixture LANGUAGES C)

# The capability under test is bzl-fxa.9: a config-header template referencing
# a plain @VAR@ the translator can't resolve (neither a cc_config catalog fact
# nor a value in the CMake cache) must ESCALATE, not silently emit a literal
# @VAR@ into the generated header. json-c hit this: json.h.cmakein gates its
# umbrella includes on @JSON_H_JSON_PATCH@ / @JSON_H_JSON_POINTER@, which the
# project computes itself; left literal they break every source that includes
# json.h.
#
# This fixture STARTS RED on purpose: the translator emits a needs_attention/
# item for FEATURE_INCLUDE_GUARD, the agent stage resolves it (here, by adding
# a `values` entry in the GENERATED config_header), and only then does it turn
# green. That start-red -> agent-resolves -> green cycle is what an
# escalation-firing fixture tests; see
# docs/architecture/needs-attention-interface.md.
#
# DO NOT resolve the red by defining FEATURE_INCLUDE_GUARD here or hand-writing
# feature.h: this file is immutable test input. The @VAR@ the project sets
# itself is the normal case the translator must surface.

set(FEATURE_INCLUDE_GUARD 1)

configure_file(feature.h.in feature.h)

add_executable(app src/main.c)
target_include_directories(app PRIVATE ${PROJECT_BINARY_DIR})
