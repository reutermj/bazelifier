"""Autoconf-style compile probes as Bazel rules.

Each probe resolves the C/C++ toolchain of whatever configuration builds it
and runs the compiler against a generated snippet, capturing *success or
failure* as data — a `true`/`false` result file — rather than letting a
failed compile abort the build. This is what lets a converted CMake module's
`configure_file` config header be computed for the CONSUMER's toolchain
instead of baked from the conversion host. See
../../docs/architecture/configure-file-and-toolchain-probes.md.

The `cc_common`/toolchain mechanics here follow the pattern the `@llvm`
toolchain uses for its own libstdc++ autoconf port (see
../../docs/lore/llvm-toolchain-ships-autoconf-probes.md); this is written
fresh for our needs, not copied.
"""

load("@rules_cc//cc:action_names.bzl", "ACTION_NAMES")
load("@rules_cc//cc:find_cc_toolchain.bzl", "CC_TOOLCHAIN_TYPE", "find_cc_toolchain", "use_cc_toolchain")
load("@rules_cc//cc/common:cc_common.bzl", "cc_common")

# The result of a probe: the macro it defines, and a file containing "true"
# or "false" — the probe's answer, produced by an action that itself always
# succeeds (see _run_compile_probe).
ProbeResultInfo = provider(
    doc = "A single autoconf-style probe's outcome.",
    fields = {
        "define": "The preprocessor macro this probe controls (e.g. HAVE_ENDIAN_H).",
        "result": "A File containing \"true\" or \"false\": whether the probe compiled.",
    },
)

# The shell wrapper. It reconstructs the compile command from the toolchain's
# own command line — substituting the real source path for the placeholder
# and a throwaway object path for the output — runs it, and writes the exit
# status as a word rather than propagating it. A nonzero compile is the
# probe answering "no", not a build failure.
_PROBE_RUNNER = """set -u
tool="$1"
source="$2"
result="$3"
log="$4"
source_placeholder="$5"
output_placeholder="$6"
shift 6

object="$(mktemp "${TMPDIR:-/tmp}/cc-config-probe.XXXXXX.o")"
trap 'rm -f "${object}"' EXIT

cmd=("${tool}")
for arg in "$@"; do
    case "${arg}" in
        "${source_placeholder}") cmd+=("${source}") ;;
        "${output_placeholder}") cmd+=("${object}") ;;
        *) cmd+=("${arg}") ;;
    esac
done

if "${cmd[@]}" > "${log}" 2>&1; then
    echo true > "${result}"
else
    echo false > "${result}"
fi
"""

def _run_compile_probe(ctx, source_content, define):
    """Compiles `source_content` against the resolved toolchain.

    Returns a ProbeResultInfo whose result file is "true"/"false" depending
    on whether the snippet compiled — a failed compile is the answer "no",
    not a build failure.
    """
    cc_toolchain = find_cc_toolchain(ctx)
    feature_configuration = cc_common.configure_features(
        ctx = ctx,
        cc_toolchain = cc_toolchain,
        requested_features = ctx.features,
        unsupported_features = ctx.disabled_features,
    )

    # A placeholder in the toolchain-generated command line, swapped for the
    # real path in the runner. Using a placeholder (rather than the real path)
    # keeps the generated command line free of this target's package path, so
    # two projects probing the same header produce the same command — the
    # basis for sharing, though sharing itself comes from a shared TARGET
    # (see the design doc), not from this alone.
    source_placeholder = "%{probe_source}"
    output_placeholder = "%{probe_object}"

    compile_variables = cc_common.create_compile_variables(
        feature_configuration = feature_configuration,
        cc_toolchain = cc_toolchain,
        source_file = source_placeholder,
        output_file = output_placeholder,
    )
    command_line = cc_common.get_memory_inefficient_command_line(
        feature_configuration = feature_configuration,
        action_name = ACTION_NAMES.c_compile,
        variables = compile_variables,
    )
    env = cc_common.get_environment_variables(
        feature_configuration = feature_configuration,
        action_name = ACTION_NAMES.c_compile,
        variables = compile_variables,
    )
    tool = cc_common.get_tool_for_action(
        feature_configuration = feature_configuration,
        action_name = ACTION_NAMES.c_compile,
    )

    stem = ctx.label.name
    source = ctx.actions.declare_file(stem + "_probe.c")
    ctx.actions.write(output = source, content = source_content)

    result = ctx.actions.declare_file(stem + ".result")
    log = ctx.actions.declare_file(stem + ".log")

    ctx.actions.run_shell(
        inputs = depset(direct = [source]),
        outputs = [result, log],
        tools = cc_toolchain.all_files,
        command = _PROBE_RUNNER,
        arguments = [
            tool,
            source.path,
            result.path,
            log.path,
            source_placeholder,
            output_placeholder,
        ] + command_line,
        env = env,
        mnemonic = "CcConfigProbe",
        progress_message = "Probing %s" % define,
        toolchain = CC_TOOLCHAIN_TYPE,
    )
    return ProbeResultInfo(define = define, result = result)

def _check_include_file_impl(ctx):
    info = _run_compile_probe(
        ctx,
        source_content = "#include <%s>\n" % ctx.attr.header,
        define = ctx.attr.define,
    )
    return [
        info,
        DefaultInfo(files = depset([info.result])),
    ]

check_include_file = rule(
    implementation = _check_include_file_impl,
    doc = "Defines `define` iff `#include <header>` compiles under the resolved toolchain — the Bazel equivalent of CMake's check_include_file().",
    attrs = {
        "header": attr.string(
            mandatory = True,
            doc = "The header to test, as it would appear in #include <...> (e.g. \"endian.h\").",
        ),
        "define": attr.string(
            mandatory = True,
            doc = "The preprocessor macro to control (e.g. \"HAVE_ENDIAN_H\").",
        ),
    },
    toolchains = use_cc_toolchain(),
    fragments = ["cpp"],
)

def _assert_probe_result_impl(ctx):
    result = ctx.attr.probe[ProbeResultInfo].result

    # A test that compares the probe's answer to what's expected. Written as
    # a shell test so it needs no runtime deps beyond a POSIX shell — the
    # probe result rides in via runfiles.
    test_script = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(
        output = test_script,
        is_executable = True,
        content = """#!/usr/bin/env bash
set -eu
actual="$(cat "{result}")"
expected="{expected}"
if [ "${{actual}}" != "${{expected}}" ]; then
    echo "FAIL: probe {define} = ${{actual}}, expected ${{expected}}" >&2
    exit 1
fi
echo "PASS: probe {define} = ${{actual}}"
""".format(
            result = result.short_path,
            expected = "true" if ctx.attr.expected else "false",
            define = ctx.attr.probe[ProbeResultInfo].define,
        ),
    )
    return [DefaultInfo(
        executable = test_script,
        runfiles = ctx.runfiles(files = [result]),
    )]

assert_probe_result_test = rule(
    implementation = _assert_probe_result_impl,
    doc = "Test rule: asserts a probe compiled (expected=True) or didn't (expected=False). Lets the probe mechanics be tested without a real config header.",
    attrs = {
        "probe": attr.label(
            mandatory = True,
            providers = [ProbeResultInfo],
            doc = "The check_* target whose result to assert.",
        ),
        "expected": attr.bool(
            mandatory = True,
            doc = "True if the probe is expected to compile.",
        ),
    },
    test = True,
)
