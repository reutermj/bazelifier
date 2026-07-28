"""Assembles a tarball containing every converted fixture module (each
renamed to fixtures/<fixture-target-name>/) plus a root MODULE.bazel that
depends on all of them via local_path_override.

See docs/architecture/build-verification.md: unpacking this tarball and
running `bazel build`/`bazel test` from its root is how we validate that
converted fixture modules are genuinely independent — and, since they're
all real bazel_deps of the same root module, that they can depend on and
build against each other as bazelifier converts more projects.
"""

load("@tar.bzl//tar:mtree.bzl", "mtree_mutate", "mtree_spec")
load("@tar.bzl//tar:tar.bzl", "tar")
load(":convert_cmake_project.bzl", "ConvertedCmakeModuleInfo")

_ROOT_MODULE_TEMPLATE = """module(
    name = "{name}",
    version = "0.0.0",
)

bazel_dep(name = "rules_shell", version = "0.8.0")

{deps}
"""

_DEP_TEMPLATE = """bazel_dep(name = "{module_name}", version = "0.0.0")
local_path_override(
    module_name = "{module_name}",
    path = "fixtures/{fixture_dir}",
)
"""

def _fixture_dir_name(label):
    """Returns the name used for a fixture's directory inside the tarball.

    This is the basename of the package the convert_cmake_project target
    lives in (e.g. "001-hello-world"), not the target's own name (which is
    conventionally just "converted" for every fixture, so it can't be used
    to distinguish them).
    """
    return label.package.rsplit("/", 1)[-1]

def _write_root_file(ctx, filename, content):
    """Writes one generated file destined for the tarball's root.

    Declared inside a same-named directory (rather than as a bare file) so
    its mtree path's basename is already exactly `filename` — mtree_mutate's
    package_dir prepends a directory prefix, it can't rename a file's
    basename, so a literal "MODULE.bazel"/"BUILD.bazel" name has to come
    from the declared path itself.
    """
    out = ctx.actions.declare_file(ctx.attr.name + "/" + filename)
    ctx.actions.write(out, content)
    return [DefaultInfo(files = depset([out]))]

def _root_module_bazel_impl(ctx):
    deps = []
    for target in ctx.attr.fixtures:
        info = target[ConvertedCmakeModuleInfo]
        deps.append(_DEP_TEMPLATE.format(
            module_name = info.module_name,
            fixture_dir = _fixture_dir_name(target.label),
        ))

    return _write_root_file(ctx, "MODULE.bazel", _ROOT_MODULE_TEMPLATE.format(
        name = ctx.attr.name_of_root_module,
        deps = "\n".join(deps),
    ))

_root_module_bazel = rule(
    implementation = _root_module_bazel_impl,
    attrs = {
        "fixtures": attr.label_list(
            mandatory = True,
            providers = [ConvertedCmakeModuleInfo],
            doc = "convert_cmake_project targets to depend on from the root module.",
        ),
        "name_of_root_module": attr.string(mandatory = True),
    },
)

_SH_TEST_TEMPLATE = """sh_test(
    name = "{test_name}",
    srcs = ["compare_runtime_output.sh"],
    args = [
        "$(location @{module_name}//:{target_name})",
        "$(location @{module_name}//ground_truth:{target_name})",
        "{module_name}+/needs_attention",
    ],
    data = [
        "@{module_name}//:{target_name}",
        "@{module_name}//ground_truth:{target_name}",
        "@{module_name}//needs_attention:all",
    ],
)
"""

_ROOT_BUILD_TEMPLATE = """load("@rules_shell//shell:sh_test.bzl", "sh_test")

{tests}
test_suite(
    name = "all_ground_truth_comparisons",
    tests = [
{test_labels}
    ],
)
"""

def _root_build_bazel_impl(ctx):
    tests = []
    test_labels = []
    for target in ctx.attr.fixtures:
        info = target[ConvertedCmakeModuleInfo]
        for target_name in info.expected_targets:
            test_name = _fixture_dir_name(target.label) + "_" + target_name + "_matches_ground_truth"
            tests.append(_SH_TEST_TEMPLATE.format(
                test_name = test_name,
                module_name = info.module_name,
                target_name = target_name,
            ))
            test_labels.append('        ":{}",'.format(test_name))

    return _write_root_file(ctx, "BUILD.bazel", _ROOT_BUILD_TEMPLATE.format(
        tests = "\n".join(tests),
        test_labels = "\n".join(test_labels),
    ))

_root_build_bazel = rule(
    implementation = _root_build_bazel_impl,
    attrs = {
        "fixtures": attr.label_list(
            mandatory = True,
            providers = [ConvertedCmakeModuleInfo],
            doc = "convert_cmake_project targets to generate ground-truth comparison sh_tests for.",
        ),
    },
)

def _combined_mtree_impl(ctx):
    out = ctx.actions.declare_file(ctx.attr.name + ".mtree")

    # mtree(5) is a plain-text, line-delimited format (one entry per line,
    # `#mtree` header optional per-file) — safe to concatenate. Each
    # per-fixture mtree independently synthesizes a "fixtures type=dir"
    # parent-directory entry (mtree_mutate's package_dir does this for
    # every path component — see tar.bzl's default.awk), so concatenating
    # N fixtures declares that same shared parent directory N times.
    # bsdtar doesn't dedupe that itself and produces a spurious nested
    # fixtures/fixtures/ directory — `awk '!seen[$0]++'` drops exact
    # duplicate lines (safe here: every field, including timestamps, is
    # produced deterministically by the same rule, so a real difference
    # would only ever come from an actual distinct entry, never noise).
    ctx.actions.run_shell(
        outputs = [out],
        inputs = ctx.files.mtrees + ctx.files.extra_files,
        command = "cat {mtrees} | awk '!seen[$0]++' > {out}".format(
            mtrees = " ".join([f.path for f in ctx.files.mtrees]),
            out = out.path,
        ),
        mnemonic = "CombineMtree",
    )
    return [DefaultInfo(files = depset([out]))]

_combined_mtree = rule(
    implementation = _combined_mtree_impl,
    attrs = {
        "mtrees": attr.label_list(mandatory = True, allow_files = True),
        # Not read by this rule directly, but declared as inputs so the
        # combined mtree's content= references resolve when tar() runs.
        "extra_files": attr.label_list(allow_files = True),
    },
)

def _entry_mtree(name, src, strip_prefix, package_dir = None):
    """Declares the spec+mutate pair placing one entry into the tarball.

    Returns the label of the mutated mtree. Every entry needs its own pair
    rather than one shared `mtree_spec` over all of them, because
    `strip_prefix` is per-entry: they come from different source packages.
    `package_dir` re-roots the entry under a directory; omit it to land at
    the tarball root.
    """
    spec_name = name + "_spec"
    mtree_spec(
        name = spec_name,
        srcs = [src],
    )

    # Passed through only when set, rather than defaulted to "" here, so
    # this doesn't encode an assumption about mtree_mutate's own default.
    package_dir_kwarg = {"package_dir": package_dir} if package_dir else {}

    mutated_name = name + "_renamed"
    mtree_mutate(
        name = mutated_name,
        mtree = ":" + spec_name,
        strip_prefix = strip_prefix,
        **package_dir_kwarg
    )
    return ":" + mutated_name

def validation_workspace(name, fixtures, **kwargs):
    """Packages `fixtures` (convert_cmake_project targets) into a tarball.

    Each fixture is renamed to `fixtures/<fixture target name>/` inside the
    tarball. The tarball's root also contains a MODULE.bazel that
    bazel_deps on every fixture via local_path_override, so unpacking the
    tarball and running `bazel build //...`/`bazel test //...` from its
    root builds/tests every converted fixture — and lets fixtures depend on
    each other, once bazelifier converts projects with real inter-project
    dependencies.

    Args:
        name: name of the tar target producing the tarball.
        fixtures: list of convert_cmake_project target labels to include.
        **kwargs: passed through to the underlying tar rule.
    """
    root_module_name = name + "_MODULE.bazel"
    _root_module_bazel(
        name = root_module_name,
        fixtures = fixtures,
        name_of_root_module = name,
    )

    root_build_name = name + "_BUILD.bazel"
    _root_build_bazel(
        name = root_build_name,
        fixtures = fixtures,
    )

    compare_script = "//translator/build_defs:compare_runtime_output.sh"

    all_srcs = [":" + root_module_name, ":" + root_build_name, compare_script]

    # Each root-level entry (MODULE.bazel, BUILD.bazel, the comparison
    # script) is mutated individually since they come from different
    # source packages/paths, then concatenated with the root entries FIRST:
    # bsdtar's mtree format treats slash-less entries as *relative* to the
    # last-seen top-level relative directory, so if a top-level directory
    # entry (e.g. "fixtures") appeared first, a later bare "MODULE.bazel"
    # entry would be created *inside* it instead of at the tarball root.
    # See the tar.bzl default.awk mutation pipeline's own comment on this
    # exact pitfall.
    #
    # A generated root file's declared output lives at
    # <this package>/<its target name>/<FILENAME>, so stripping that
    # directory leaves the bare filename at the tarball root. The
    # comparison script is a plain source file, so its own package is what
    # gets stripped instead.
    mtrees = [
        _entry_mtree(
            name + "_root_module",
            ":" + root_module_name,
            native.package_name() + "/" + root_module_name,
        ),
        _entry_mtree(
            name + "_root_build",
            ":" + root_build_name,
            native.package_name() + "/" + root_build_name,
        ),
        _entry_mtree(
            name + "_script",
            compare_script,
            Label(compare_script).package,
        ),
    ]

    for fixture in fixtures:
        label = Label(fixture)
        fixture_dir = _fixture_dir_name(label)

        # A fixture's tree artifact has the natural mtree path
        # "<fixture's package>/<fixture target name>/...". Strip that down
        # and re-root it under fixtures/<fixture dir name>/ so the unpacked
        # tarball has a clean, uniform layout regardless of where the
        # fixture actually lives in this repo.
        mtrees.append(_entry_mtree(
            name + "_" + fixture_dir,
            fixture,
            label.package + "/" + label.name,
            package_dir = "fixtures/" + fixture_dir,
        ))
        all_srcs.append(fixture)

    combined_name = name + "_combined_mtree"
    _combined_mtree(
        name = combined_name,
        mtrees = mtrees,
        extra_files = all_srcs,
    )

    tar(
        name = name,
        srcs = all_srcs,
        mtree = ":" + combined_name,
        **kwargs
    )
