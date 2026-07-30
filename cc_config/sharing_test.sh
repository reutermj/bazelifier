#!/usr/bin/env bash
# Proves cc_config's load-bearing sharing property: a probe is a shared
# TARGET, so its compile action is analyzed exactly once, and every config
# header that references it consumes that one action's output — hence a probe
# runs once per toolchain across the whole build graph, not once per converted
# project. See docs/architecture/configure-file-and-toolchain-probes.md ("Run
# once per toolchain, not once per project").
#
# Why a script rather than a Bazel test: the assertion is about Bazel's own
# action graph, read via `bazel aquery`; running aquery from inside a Bazel
# test would be bazel-inside-bazel. Run this in review (cheap enough for CI).
# Run from the repo root.
set -euo pipefail

probe='@cc_config//cc_config:have_stdlib_h'
probe_result='have_stdlib_h.result'

# shared_consumer_a and shared_consumer_b are two separate config_header
# targets that both reference the same probe target.
echo "Building two config headers that share the ${probe} probe ..."
bazel build \
    @cc_config//cc_config:shared_consumer_a \
    @cc_config//cc_config:shared_consumer_b >/dev/null 2>&1

# 1. The shared probe target has exactly one compile action.
probe_actions="$(bazel aquery "mnemonic(\"CcConfigProbe\", ${probe})" 2>/dev/null \
    | grep -c '^  Mnemonic: CcConfigProbe' || true)"
if [[ "${probe_actions}" != "1" ]]; then
    echo "FAIL: the shared probe has ${probe_actions} actions, expected 1." >&2
    exit 1
fi

# 2. The real proof: BOTH consumers' header actions take that probe's result
# FILE as an input — so they share the one probe's output, not a per-consumer
# copy. If config_header ever minted its own probe per consumer, a consumer's
# header action would depend on a different result file (or a duplicate probe
# action), and this would fail.
for consumer in shared_consumer_a shared_consumer_b; do
    inputs="$(bazel aquery "mnemonic(\"CcConfigHeader\", @cc_config//cc_config:${consumer})" 2>/dev/null \
        | grep 'Inputs:' || true)"
    if ! grep -qF "${probe_result}" <<<"${inputs}"; then
        echo "FAIL: ${consumer}'s header does not consume the shared probe result ${probe_result}." >&2
        echo "It is using a separate probe, defeating once-per-toolchain sharing." >&2
        exit 1
    fi
done

echo "PASS: one shared probe action, consumed by both config headers (shared once per toolchain)."
