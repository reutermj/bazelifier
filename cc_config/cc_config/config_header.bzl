"""Rule that expands a CMake `configure_file` template into a config header.

Consumes probe results (see probe.bzl) plus a plain `@VAR@` values map and
runs the expand_config_header.py helper to produce the header at build time —
so the probe-derived defines reflect the toolchain the consumer built with.
This is the config-header slice of `configure_file`, not all of it. See
../../docs/architecture/configure-file-and-toolchain-probes.md.
"""

load(":probe.bzl", "ProbeResultInfo")

def _config_header_impl(ctx):
    output = ctx.actions.declare_file(ctx.attr.output)

    values_file = ctx.actions.declare_file(ctx.label.name + "_values.json")
    ctx.actions.write(output = values_file, content = json.encode(ctx.attr.values))

    args = ctx.actions.args()
    args.add("--template", ctx.file.template)
    args.add("--output", output)
    args.add("--values", values_file)
    for name in ctx.attr.unresolved:
        args.add("--unresolved", name)

    result_files = []
    for probe in ctx.attr.probes:
        info = probe[ProbeResultInfo]
        result_files.append(info.result)

        # MACRO=path so the helper keys results by macro rather than by the
        # result file's name (which is the probe target's name).
        args.add("--result", "%s=%s" % (info.define, info.result.path))

    # Keyed by the FILE and valued by the marker, which is the direction the
    # relationship actually runs: gnulib splices one `c++defs.h` at a marker
    # naming it, and the same file is spliced into fourteen different headers.
    # A marker identifies exactly one file; a file serves many markers.
    splice_files = []
    for file, marker in ctx.attr.splices.items():
        spliced = file.files.to_list()[0]
        splice_files.append(spliced)
        args.add("--splice", "%s=%s" % (marker, spliced.path))

    ctx.actions.run(
        outputs = [output],
        inputs = depset(direct = [ctx.file.template, values_file] + result_files + splice_files),
        executable = ctx.executable._expander,
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
        "unresolved": attr.string_list(
            doc = "Names the template references that nothing here answers — " +
                  "no probe, no value, and an open needs_attention item. Each " +
                  "becomes an `#error` in the generated header rather than an " +
                  "`#undef`, because `#undef` ASSERTS the feature is absent " +
                  "while these are merely unknown. Without it the module " +
                  "compiles against a header that silently decided.",
        ),
        "splices": attr.label_keyed_string_dict(
            allow_files = True,
            doc = "Whole files inserted after a marker line — sed's `r`. " +
                  "Keyed by the file (a label) and valued by the marker text, " +
                  "because one file may be spliced at several markers and a " +
                  "marker names exactly one file.",
        ),
        "_expander": attr.label(
            default = Label("@cc_config//cc_config:expand_config_header"),
            executable = True,
            cfg = "exec",
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
