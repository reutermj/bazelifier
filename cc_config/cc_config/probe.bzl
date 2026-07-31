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

# The result of a probe: the macro it defines, and a file holding the
# probe's answer, produced by an action that itself always succeeds. For a
# boolean probe (check_include_file / check_symbol_exists) the file contains
# "true"/"false"; for check_type_size it contains the size as a number
# string (e.g. "8"). The config-header expander treats a "true"/"false"
# result as a #cmakedefine and any other value as a substitution value.
ProbeResultInfo = provider(
    doc = "A single autoconf-style probe's outcome.",
    fields = {
        "define": "The preprocessor macro this probe controls (e.g. HAVE_ENDIAN_H, SIZEOF_LONG).",
        "result": "A File holding the answer: \"true\"/\"false\" for a boolean probe, or a number for a size probe.",
    },
)

# Placeholders in the toolchain-generated command lines, swapped for real
# paths in the runner. Using placeholders (rather than the real paths) keeps
# the generated command lines free of this target's package path — so two
# projects probing the same thing produce the same command. Sharing itself
# comes from a shared TARGET (see the design doc), but this keeps the door
# open to action-level dedup too.
_SOURCE_PLACEHOLDER = "%{probe_source}"
_OBJECT_PLACEHOLDER = "%{probe_object}"
_BINARY_PLACEHOLDER = "%{probe_binary}"

# The shell runner. It reconstructs the compile command (and, for a link
# probe, the link command) from the toolchain's own command lines,
# substituting real paths for the placeholders, runs them, and writes the
# exit status as a word rather than propagating it. A nonzero result is the
# probe answering "no", not a build failure.
#
# Arg contract: tool paths and bookkeeping, then the compile command line,
# then (link probes only) a `--` separator and the link command line. `link`
# selects the mode so one runner serves both.
_PROBE_RUNNER = """set -u
mode="$1"
compile_tool="$2"
link_tool="$3"
result="$4"
log="$5"
source="$6"
shift 6

tmp="$(mktemp -d "${TMPDIR:-/tmp}/cc-config-probe.XXXXXX")"
trap 'rm -rf "${tmp}"' EXIT
object="${tmp}/probe.o"
binary="${tmp}/probe.bin"

compile_cmd=("${compile_tool}")
while [ "$#" -gt 0 ]; do
    if [ "$1" = "--" ]; then shift; break; fi
    case "$1" in
        "%{probe_source}") compile_cmd+=("${source}") ;;
        "%{probe_object}") compile_cmd+=("${object}") ;;
        *) compile_cmd+=("$1") ;;
    esac
    shift
done

ok=false
if "${compile_cmd[@]}" > "${log}" 2>&1; then
    if [ "${mode}" = "link" ]; then
        link_cmd=("${link_tool}")
        for arg in "$@"; do
            case "${arg}" in
                "%{probe_binary}") link_cmd+=("${binary}") ;;
                *) link_cmd+=("${arg}") ;;
            esac
        done
        link_cmd+=("${object}")
        if "${link_cmd[@]}" >> "${log}" 2>&1; then ok=true; fi
    else
        ok=true
    fi
fi
echo "${ok}" > "${result}"
"""

def _toolchain(ctx):
    cc_toolchain = find_cc_toolchain(ctx)
    feature_configuration = cc_common.configure_features(
        ctx = ctx,
        cc_toolchain = cc_toolchain,
        requested_features = ctx.features,
        unsupported_features = ctx.disabled_features,
    )
    return cc_toolchain, feature_configuration

def _compile_command_line(cc_toolchain, feature_configuration, defines = []):
    # A probe's `defines` become preprocessor_defines on the compile variables,
    # so the toolchain renders them as it would any -D (correct flag spelling
    # per compiler). This is how a probe is compiled under a feature-test macro
    # like _GNU_SOURCE: glibc gates symbols (vasprintf, strdup, uselocale, ...)
    # behind it, and the autoconf projects the catalog serves set it globally
    # (json-c via CMAKE_REQUIRED_DEFINITIONS), so a bare probe would answer
    # "absent" for a symbol the consumer's real build can see. See bzl-fxa.7.
    compile_variables = cc_common.create_compile_variables(
        feature_configuration = feature_configuration,
        cc_toolchain = cc_toolchain,
        source_file = _SOURCE_PLACEHOLDER,
        output_file = _OBJECT_PLACEHOLDER,
        preprocessor_defines = depset(defines),
    )
    return struct(
        command_line = cc_common.get_memory_inefficient_command_line(
            feature_configuration = feature_configuration,
            action_name = ACTION_NAMES.c_compile,
            variables = compile_variables,
        ),
        env = cc_common.get_environment_variables(
            feature_configuration = feature_configuration,
            action_name = ACTION_NAMES.c_compile,
            variables = compile_variables,
        ),
        tool = cc_common.get_tool_for_action(
            feature_configuration = feature_configuration,
            action_name = ACTION_NAMES.c_compile,
        ),
    )

def _link_command_line(cc_toolchain, feature_configuration):
    link_variables = cc_common.create_link_variables(
        feature_configuration = feature_configuration,
        cc_toolchain = cc_toolchain,
        output_file = _BINARY_PLACEHOLDER,
        is_using_linker = True,
        is_linking_dynamic_library = False,
    )
    return struct(
        command_line = cc_common.get_memory_inefficient_command_line(
            feature_configuration = feature_configuration,
            action_name = ACTION_NAMES.cpp_link_executable,
            variables = link_variables,
        ),
        env = cc_common.get_environment_variables(
            feature_configuration = feature_configuration,
            action_name = ACTION_NAMES.cpp_link_executable,
            variables = link_variables,
        ),
        tool = cc_common.get_tool_for_action(
            feature_configuration = feature_configuration,
            action_name = ACTION_NAMES.cpp_link_executable,
        ),
    )

def _run_probe(ctx, source_content, define, link, defines = []):
    """Compiles (and, when `link`, links) `source_content` against the toolchain.

    Returns a ProbeResultInfo whose result file is "true"/"false" depending
    on whether the snippet built — a failed build is the answer "no", not a
    build failure. `link` distinguishes check_include_file (compile only,
    enough to know a header parses) from check_symbol_exists (must link, so a
    declared-but-undefined symbol fails). `defines` are preprocessor defines
    the snippet compiles under (e.g. `_GNU_SOURCE`).
    """
    cc_toolchain, feature_configuration = _toolchain(ctx)
    compile = _compile_command_line(cc_toolchain, feature_configuration, defines)

    stem = ctx.label.name
    source = ctx.actions.declare_file(stem + "_probe.c")
    ctx.actions.write(output = source, content = source_content)
    result = ctx.actions.declare_file(stem + ".result")
    log = ctx.actions.declare_file(stem + ".log")

    link = _link_command_line(cc_toolchain, feature_configuration) if link else None
    arguments = [
        "link" if link else "compile",
        compile.tool,
        link.tool if link else "",
        result.path,
        log.path,
        source.path,
    ] + compile.command_line
    env = dict(compile.env)
    if link:
        arguments = arguments + ["--"] + link.command_line
        env.update(link.env)

    ctx.actions.run_shell(
        inputs = depset(direct = [source]),
        outputs = [result, log],
        tools = cc_toolchain.all_files,
        command = _PROBE_RUNNER,
        arguments = arguments,
        env = env,
        mnemonic = "CcConfigProbe",
        progress_message = "Probing %s" % define,
        toolchain = CC_TOOLCHAIN_TYPE,
    )
    return ProbeResultInfo(define = define, result = result)

# The size-probe runner. A type's size can't be read by running a program (we
# may be cross-compiling — the point is the CONSUMER's toolchain, which we
# can't execute), so it's found at COMPILE time: the snippet declares an array
# whose length is `sizeof(type) == CANDIDATE ? 1 : -1`, which compiles only
# when the guess is right (a negative length is an error). The runner tries
# each candidate size and writes the first that compiles. An unknown size (no
# candidate matched) writes empty, which surfaces as an unresolved value
# rather than a wrong one.
#
# Arg contract: compile_tool, result, log, source, then the candidate sizes
# (space-free, one per arg) up to `--`, then the compile command line. The
# candidate is injected as -DCC_CONFIG_PROBE_SIZE=N before the source.
_SIZE_RUNNER = """set -u
compile_tool="$1"
result="$2"
log="$3"
source="$4"
shift 4

candidates=()
while [ "$#" -gt 0 ]; do
    if [ "$1" = "--" ]; then shift; break; fi
    candidates+=("$1")
    shift
done

tmp="$(mktemp -d "${TMPDIR:-/tmp}/cc-config-size.XXXXXX")"
trap 'rm -rf "${tmp}"' EXIT
object="${tmp}/probe.o"

: > "${log}"
size=""
for candidate in "${candidates[@]}"; do
    cmd=("${compile_tool}")
    for arg in "$@"; do
        case "${arg}" in
            "%{probe_source}") cmd+=("-DCC_CONFIG_PROBE_SIZE=${candidate}" "${source}") ;;
            "%{probe_object}") cmd+=("${object}") ;;
            *) cmd+=("${arg}") ;;
        esac
    done
    if "${cmd[@]}" >> "${log}" 2>&1; then
        size="${candidate}"
        break
    fi
done
printf '%s' "${size}" > "${result}"
"""

def _run_size_probe(ctx, type_name, define, candidates):
    """Determines sizeof(`type_name`) at compile time (no execution).

    Returns a ProbeResultInfo whose result file holds the matching size as a
    number, or empty if no candidate matched.
    """
    cc_toolchain, feature_configuration = _toolchain(ctx)
    compile = _compile_command_line(cc_toolchain, feature_configuration)

    stem = ctx.label.name
    source = ctx.actions.declare_file(stem + "_probe.c")
    ctx.actions.write(
        output = source,
        content = """{includes}
int probe[sizeof({type}) == CC_CONFIG_PROBE_SIZE ? 1 : -1];
""".format(
            includes = "".join(["#include <%s>\n" % h for h in ctx.attr.headers]),
            type = type_name,
        ),
    )
    result = ctx.actions.declare_file(stem + ".result")
    log = ctx.actions.declare_file(stem + ".log")

    arguments = [
        compile.tool,
        result.path,
        log.path,
        source.path,
    ] + [str(c) for c in candidates] + ["--"] + compile.command_line

    ctx.actions.run_shell(
        inputs = depset(direct = [source]),
        outputs = [result, log],
        tools = cc_toolchain.all_files,
        command = _SIZE_RUNNER,
        arguments = arguments,
        env = compile.env,
        mnemonic = "CcConfigSizeProbe",
        progress_message = "Probing sizeof %s" % type_name,
        toolchain = CC_TOOLCHAIN_TYPE,
    )
    return ProbeResultInfo(define = define, result = result)

# The sizes a C type can plausibly have, tried in order. Covers every standard
# integer, pointer, and common aggregate; a type larger than this is not
# something a config header's SIZEOF_ macro describes.
_CANDIDATE_SIZES = [1, 2, 4, 8, 16, 32]

def _check_type_size_impl(ctx):
    info = _run_size_probe(
        ctx,
        type_name = ctx.attr.type,
        define = ctx.attr.define,
        candidates = _CANDIDATE_SIZES,
    )
    return [
        info,
        DefaultInfo(files = depset([info.result])),
    ]

check_type_size = rule(
    implementation = _check_type_size_impl,
    doc = "Sets `define` to the size of `type` under the resolved toolchain, found at compile time (no execution, so it works cross-target) — the Bazel equivalent of CMake's check_type_size().",
    attrs = {
        "type": attr.string(
            mandatory = True,
            doc = "The type whose size to determine (e.g. \"long\", \"size_t\", \"void *\").",
        ),
        "headers": attr.string_list(
            default = [],
            doc = "Headers that declare the type, in #include <...> form (e.g. [\"stddef.h\"]).",
        ),
        "define": attr.string(
            mandatory = True,
            doc = "The macro to set to the size (e.g. \"SIZEOF_LONG\").",
        ),
    },
    toolchains = use_cc_toolchain(),
    fragments = ["cpp"],
)

def _check_include_file_impl(ctx):
    info = _run_probe(
        ctx,
        source_content = "#include <%s>\n" % ctx.attr.header,
        define = ctx.attr.define,
        link = False,
        defines = ctx.attr.defines,
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
        "defines": attr.string_list(
            default = [],
            doc = "Feature-test macros to compile the probe under, without the -D " +
                  "(e.g. [\"_GNU_SOURCE\"]). A header gated behind one is only visible " +
                  "when the probe compiles with it set.",
        ),
    },
    toolchains = use_cc_toolchain(),
    fragments = ["cpp"],
)

def _check_symbol_exists_impl(ctx):
    includes = "".join(["#include <%s>\n" % h for h in ctx.attr.headers])

    # Mirrors CMake's check_symbol_exists snippet: if the symbol is a macro,
    # the `#ifndef` body is dropped and just referencing it in the ternary
    # confirms it; if it's a function, taking its address forces the linker
    # to resolve it — so a declared-but-undefined symbol fails at link. Hence
    # this is a link probe, not compile-only.
    source = includes + """
int main(void) {{
#ifndef {symbol}
  (void)((void *)(&{symbol}));
#endif
  return 0;
}}
""".format(symbol = ctx.attr.symbol)

    info = _run_probe(
        ctx,
        source_content = source,
        define = ctx.attr.define,
        link = True,
        defines = ctx.attr.defines,
    )
    return [
        info,
        DefaultInfo(files = depset([info.result])),
    ]

check_symbol_exists = rule(
    implementation = _check_symbol_exists_impl,
    doc = "Defines `define` iff `symbol` (declared by `headers`) can be referenced and linked under the resolved toolchain — the Bazel equivalent of CMake's check_symbol_exists(). Links, so a declared-but-undefined function fails.",
    attrs = {
        "symbol": attr.string(
            mandatory = True,
            doc = "The function or macro to test (e.g. \"getrandom\").",
        ),
        "headers": attr.string_list(
            mandatory = True,
            doc = "Headers that declare the symbol, in #include <...> form (e.g. [\"sys/random.h\"]).",
        ),
        "define": attr.string(
            mandatory = True,
            doc = "The preprocessor macro to control (e.g. \"HAVE_GETRANDOM\").",
        ),
        "defines": attr.string_list(
            default = [],
            doc = "Feature-test macros to compile the probe under, without the -D " +
                  "(e.g. [\"_GNU_SOURCE\"]). glibc gates symbols like vasprintf/strdup " +
                  "behind _GNU_SOURCE, so a bare probe reports them absent even though a " +
                  "consumer setting _GNU_SOURCE (the autoconf norm) can use them.",
        ),
    },
    toolchains = use_cc_toolchain(),
    fragments = ["cpp"],
)

def _probe_alias_impl(ctx):
    # Re-publishes the underlying probe's EXISTING result file under a new
    # macro name. No new action, so the aliased fact stays one shared probe
    # per toolchain — re-probing under the alias name would answer the same
    # question a second time and silently allow the two to disagree.
    return [
        ProbeResultInfo(
            define = ctx.attr.define,
            result = ctx.attr.probe[ProbeResultInfo].result,
        ),
        DefaultInfo(files = depset([ctx.attr.probe[ProbeResultInfo].result])),
    ]

probe_alias = rule(
    implementation = _probe_alias_impl,
    doc = "Republishes an existing probe's result under a different macro name — for a project that stamps a catalog fact into a project-prefixed define (e.g. JSON_C_HAVE_INTTYPES_H from HAVE_INTTYPES_H).",
    attrs = {
        "probe": attr.label(
            mandatory = True,
            providers = [ProbeResultInfo],
            doc = "The probe whose result to republish (e.g. @cc_config//catalog:have_inttypes_h).",
        ),
        "define": attr.string(
            mandatory = True,
            doc = "The macro name to publish the result as (e.g. \"JSON_C_HAVE_INTTYPES_H\").",
        ),
    },
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
