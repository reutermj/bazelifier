cmake_minimum_required(VERSION 3.20)
project(sources_outside_deliverable_root CXX)

# app references a source from `../not-shipped/`, a sibling directory that
# is NOT part of this fixture's declared deliverable_root (see
# BUILD.bazel: deliverable_root is `proj/` itself here, unlike
# 006-sibling-sources where it widens to the whole fixture directory).
# `not-shipped/` cannot be reproduced from what the deliverable ships, so
# the translator cannot widen the module root to cover it and escalates
# instead of silently dropping or misresolving the reference — see
# docs/architecture/cmake-frontend.md.
#
# not_shipped_answer() is deliberately NOT called from main.cpp — same
# reason as 007-generated-source: the needs_attention/ gate must be the
# ONLY thing failing this fixture. If main.cpp referenced it, the
# Bazel-generated cc_binary (missing not-shipped/extra.cpp from srcs)
# would fail to LINK, conflating "the escalation fired" with "the
# conversion happened to break for an unrelated reason".
#
# DO NOT "fix" this by moving not-shipped/ under proj/ or widening
# deliverable_root in BUILD.bazel. This file and the fixture's directory
# layout are immutable test input: a project referencing a file outside
# its declared deliverable is the input shape under test. Resolution
# belongs in the GENERATED BUILD.bazel (vendor the file into this module,
# or make it available as a deps edge on another module) — never by
# widening what this fixture declares as its deliverable.
add_executable(app src/main.cpp ../not-shipped/extra.cpp)
