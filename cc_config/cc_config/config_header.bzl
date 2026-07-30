"""Rule that expands a CMake `configure_file` template into a config header.

Consumes probe results (see probe.bzl) plus a plain `@VAR@` values map and
produces the header. `@VAR@`/`${VAR}` substitution is done in Starlark (the
values are known at analysis time); `#cmakedefine` resolution is done in a
shell action, since it depends on probe result *files* produced at build
time. This is the config-header slice of `configure_file`, not all of it. See
../../docs/architecture/configure-file-and-toolchain-probes.md.

Deliberately no Python/other-language helper: keeping the build-time step to
POSIX shell + awk means a converted module building this rule needs nothing
beyond the C toolchain the probes already require — no host or hermetic
interpreter to wire through the exec configuration.
"""

load(":probe.bzl", "ProbeResultInfo")

# awk program resolving #cmakedefine directives. The set macros (those whose
# probe reported "true") are passed as a space-delimited SET string it looks
# names up in. #cmakedefine NAME [rest] -> "#define NAME [rest]" if set, else
# "/* #undef NAME */"; #cmakedefine01 NAME -> "#define NAME 1|0". @VAR@ values
# are already substituted before this runs.
_RESOLVE_CMAKEDEFINE = r"""
BEGIN { n = split(SET, a, " "); for (i = 1; i <= n; i++) is_set[a[i]] = 1 }
/^[[:space:]]*#cmakedefine01[[:space:]]/ {
    match($0, /^[[:space:]]*/); indent = substr($0, 1, RLENGTH)
    name = $2
    printf "%s#define %s %d\n", indent, name, (name in is_set ? 1 : 0)
    next
}
/^[[:space:]]*#cmakedefine[[:space:]]/ {
    match($0, /^[[:space:]]*/); indent = substr($0, 1, RLENGTH)
    name = $2
    rest = $0
    sub(/^[[:space:]]*#cmakedefine[[:space:]]+[^[:space:]]+/, "", rest)
    if (name in is_set) printf "%s#define %s%s\n", indent, name, rest
    else printf "%s/* #undef %s */\n", indent, name
    next
}
{ print }
"""

# Builds the space-delimited SET of macros that are "set" for #cmakedefine —
# a probe whose result file is "true", OR a name with a non-empty value (arg
# 4, space-delimited) — then runs the awk resolver over the (already
# @VAR@-substituted) template. Matching CMake, a #cmakedefine is defined when
# its name is truthy in ANY variable, probe-derived or plain.
_EXPAND_RUNNER = """set -eu
template="$1"
output="$2"
awk_program="$3"
value_set="$4"
shift 4

set_macros="${value_set}"
while [ "$#" -gt 0 ]; do
    macro="${1%%=*}"
    file="${1#*=}"
    if [ "$(cat "${file}")" = "true" ]; then
        set_macros="${set_macros} ${macro}"
    fi
    shift
done

awk -v SET="${set_macros}" -f "${awk_program}" "${template}" > "${output}"
"""

def _var_substitutions(values):
    """The {`@NAME@`: value, `${NAME}`: value} map expand_template applies.

    A template variable with no entry here is left untouched — an unresolved
    `@FOO@` then surfaces as a compile error in the config header rather than
    silently becoming empty, which is the safer failure while the value map is
    supplied by hand.
    """
    subs = {}
    for name, value in values.items():
        subs["@%s@" % name] = value
        subs["${%s}" % name] = value
    return subs

def _config_header_impl(ctx):
    # @VAR@ substitution: the values are known at analysis time, so
    # expand_template applies them (the template text itself is a file, hence
    # an action rather than a Starlark string replace).
    substituted_template = ctx.actions.declare_file(ctx.label.name + "_substituted.in")
    ctx.actions.expand_template(
        template = ctx.file.template,
        output = substituted_template,
        substitutions = _var_substitutions(ctx.attr.values),
    )

    awk_file = ctx.actions.declare_file(ctx.label.name + "_resolve.awk")
    ctx.actions.write(output = awk_file, content = _RESOLVE_CMAKEDEFINE)

    output = ctx.actions.declare_file(ctx.attr.output)
    args = ctx.actions.args()
    args.add(substituted_template)
    args.add(output)
    args.add(awk_file)

    # Names with a non-empty value are "set" for #cmakedefine too, not only
    # probes — so `#cmakedefine PACKAGE_NAME "@PACKAGE_NAME@"` becomes a
    # #define when PACKAGE_NAME has a value.
    value_set = " ".join([name for name, value in ctx.attr.values.items() if value])
    args.add(value_set)

    result_files = []
    for probe in ctx.attr.probes:
        info = probe[ProbeResultInfo]
        result_files.append(info.result)
        args.add("%s=%s" % (info.define, info.result.path))

    ctx.actions.run_shell(
        outputs = [output],
        inputs = depset(direct = [substituted_template, awk_file] + result_files),
        command = _EXPAND_RUNNER,
        arguments = [args],
        mnemonic = "CcConfigHeader",
        progress_message = "Generating %s" % output.short_path,
    )
    return [DefaultInfo(files = depset([output]))]

config_header = rule(
    implementation = _config_header_impl,
    doc = "Expands a configure_file template into a config header, resolving #cmakedefine from probe results and @VAR@ from `values`.",
    attrs = {
        "template": attr.label(
            mandatory = True,
            allow_single_file = True,
            doc = "The .in/.cmakein template.",
        ),
        "output": attr.string(
            mandatory = True,
            doc = "Name of the generated header (e.g. \"config.h\").",
        ),
        "probes": attr.label_list(
            providers = [ProbeResultInfo],
            doc = "check_* targets whose results resolve #cmakedefine directives.",
        ),
        "values": attr.string_dict(
            doc = "Plain @VAR@ substitutions (e.g. version strings), name -> value.",
        ),
    },
)

def _sh_single_quote(s):
    # Wrap in single quotes for the shell, escaping any embedded single quote.
    # The needles here contain " but not ', so this is exact; the escape keeps
    # it correct if that changes.
    return "'" + s.replace("'", "'\\''") + "'"

def _assert_config_header_impl(ctx):
    header = ctx.file.header
    checks = []
    for needle in ctx.attr.must_contain:
        q = _sh_single_quote(needle)
        checks.append(
            "if ! grep -qF -- {q} \"${{header}}\"; then printf 'FAIL: missing: %s\\n' {q} >&2; fail=1; fi".format(q = q),
        )
    for needle in ctx.attr.must_not_contain:
        q = _sh_single_quote(needle)
        checks.append(
            "if grep -qF -- {q} \"${{header}}\"; then printf 'FAIL: forbidden: %s\\n' {q} >&2; fail=1; fi".format(q = q),
        )

    script = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(
        output = script,
        is_executable = True,
        content = "#!/usr/bin/env bash\nset -u\nheader=\"{header}\"\nfail=0\n{checks}\nexit ${{fail}}\n".format(
            header = header.short_path,
            checks = "\n".join(checks),
        ),
    )
    return [DefaultInfo(
        executable = script,
        runfiles = ctx.runfiles(files = [header]),
    )]

assert_config_header_test = rule(
    implementation = _assert_config_header_impl,
    doc = "Test rule: asserts a generated config header contains / does not contain given fixed strings.",
    attrs = {
        "header": attr.label(
            mandatory = True,
            allow_single_file = True,
            doc = "The config_header target to check.",
        ),
        "must_contain": attr.string_list(
            doc = "Fixed strings the header must contain.",
        ),
        "must_not_contain": attr.string_list(
            doc = "Fixed strings the header must not contain.",
        ),
    },
    test = True,
)
