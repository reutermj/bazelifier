"""Assembles a tarball containing every converted fixture module (each
placed at fixtures/<fixture directory name>/, see _fixture_dir_name) plus a
root MODULE.bazel that depends on all of them via local_path_override.

See docs/architecture/build-verification.md: unpacking this tarball and
running `bazel build`/`bazel test` from its root is how we validate that
converted fixture modules are genuinely independent — and, since they're
all real bazel_deps of the same root module, that they can depend on and
build against each other as bazelifier converts more projects.

The layout is staged as a directory first, then that one directory is
archived. Assembling it in the mtree instead — one `mtree_spec`/
`mtree_mutate` pair per entry, concatenated — is what this used to do, and
it made the tarball's layout a property of how N separate mtrees combine
rather than of anything you can look at. Three distinct bsdtar behaviors
had to be worked around to get it right: entry ordering deciding whether a
root file landed at the root or inside the last-seen directory, duplicate
synthesized parent-directory entries producing a nested `fixtures/fixtures/`,
and `package_dir` being unable to rename a basename. Each failed by
producing a subtly wrong tarball rather than an error. Staging on disk
removes the class: one directory in, one mtree out.
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

# The third arg's `{module_name}+` is Bazel's canonical repo name for a
# Bzlmod module, which is what names its directory in a test's runfiles
# tree. The trailing `+` is the separator before a version component that
# is empty here, because every fixture comes in via local_path_override
# rather than a registry. It is spelled out rather than $(location)-expanded
# for the reason compare_runtime_output.sh documents: needs_attention/ is
# legitimately empty most of the time, and $(location) cannot expand to zero
# files.
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

def _fixture_dir_name(label):
    """Returns the name used for a fixture's directory inside the tarball.

    This is the basename of the package the convert_cmake_project target
    lives in (e.g. "001-hello-world"), not the target's own name (which is
    conventionally just "converted" for every fixture, so it can't be used
    to distinguish them).
    """
    return label.package.rsplit("/", 1)[-1]

def _root_module_bazel(ctx):
    deps = []
    for target in ctx.attr.fixtures:
        deps.append(_DEP_TEMPLATE.format(
            module_name = target[ConvertedCmakeModuleInfo].module_name,
            fixture_dir = _fixture_dir_name(target.label),
        ))

    return _ROOT_MODULE_TEMPLATE.format(
        name = ctx.attr.name_of_root_module,
        deps = "\n".join(deps),
    )

def _root_build_bazel(ctx):
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

    return _ROOT_BUILD_TEMPLATE.format(
        tests = "\n".join(tests),
        test_labels = "\n".join(test_labels),
    )

def _validation_tree_impl(ctx):
    out = ctx.actions.declare_directory(ctx.attr.name)

    module_bazel = ctx.actions.declare_file(ctx.attr.name + ".MODULE.bazel")
    ctx.actions.write(module_bazel, _root_module_bazel(ctx))

    build_bazel = ctx.actions.declare_file(ctx.attr.name + ".BUILD.bazel")
    ctx.actions.write(build_bazel, _root_build_bazel(ctx))

    # `cp` without -p takes the source's permission bits but not its
    # timestamps, which is what the archive wants: the comparison script
    # stays executable, while every entry's mtime comes from mtree_spec
    # rather than from whatever the execroot happened to hold.
    commands = [
        "set -e",
        "mkdir -p {}/fixtures".format(out.path),
        "cp {} {}/MODULE.bazel".format(module_bazel.path, out.path),
        "cp {} {}/BUILD.bazel".format(build_bazel.path, out.path),
        "cp {} {}/compare_runtime_output.sh".format(ctx.file.compare_script.path, out.path),
    ]

    inputs = [module_bazel, build_bazel, ctx.file.compare_script]
    for target in ctx.attr.fixtures:
        # A convert_cmake_project target's DefaultInfo is exactly the one
        # tree artifact holding its generated module.
        tree = target[DefaultInfo].files.to_list()[0]
        inputs.append(tree)
        staged = "{}/fixtures/{}".format(out.path, _fixture_dir_name(target.label))
        commands.append("mkdir -p {}".format(staged))
        commands.append("cp -R {}/. {}/".format(tree.path, staged))

    ctx.actions.run_shell(
        outputs = [out],
        inputs = inputs,
        command = "\n".join(commands),
        mnemonic = "StageValidationWorkspace",
        progress_message = "Staging validation workspace %s" % ctx.attr.name,
    )

    return [DefaultInfo(files = depset([out]))]

_validation_tree = rule(
    implementation = _validation_tree_impl,
    doc = "Stages the unpacked workspace's exact layout as one directory.",
    attrs = {
        "compare_script": attr.label(
            allow_single_file = True,
            mandatory = True,
            doc = "The ground-truth comparison script, placed at the tarball root.",
        ),
        "fixtures": attr.label_list(
            mandatory = True,
            providers = [ConvertedCmakeModuleInfo],
            doc = "convert_cmake_project targets to stage under fixtures/.",
        ),
        "name_of_root_module": attr.string(
            mandatory = True,
            doc = "Name for the generated root MODULE.bazel's own module() declaration. Never referenced by a fixture — the root module is the thing being built from, not depended on — so it only has to be a valid module name.",
        ),
    },
)

def validation_workspace(name, fixtures, **kwargs):
    """Packages `fixtures` (convert_cmake_project targets) into a tarball.

    Each fixture lands at `fixtures/<fixture directory name>/` inside the
    tarball (see `_fixture_dir_name`). The tarball's root also contains a
    MODULE.bazel that bazel_deps on every fixture via local_path_override,
    so unpacking the tarball and running `bazel build //...`/`bazel test
    //...` from its root builds/tests every converted fixture — and lets
    fixtures depend on each other, once bazelifier converts projects with
    real inter-project dependencies.

    Args:
        name: name of the tar target producing the tarball.
        fixtures: list of convert_cmake_project target labels to include.
        **kwargs: passed through to the underlying tar rule.
    """
    tree_name = name + "_tree"
    _validation_tree(
        name = tree_name,
        compare_script = "//translator/build_defs:compare_runtime_output.sh",
        fixtures = fixtures,
        name_of_root_module = name,
    )

    spec_name = name + "_mtree"
    mtree_spec(
        name = spec_name,
        srcs = [":" + tree_name],
    )

    # The staged directory's own path is the only prefix to strip: every
    # entry is already at its final location beneath it.
    rooted_name = name + "_mtree_rooted"
    mtree_mutate(
        name = rooted_name,
        mtree = ":" + spec_name,
        strip_prefix = native.package_name() + "/" + tree_name,
    )

    tar(
        name = name,
        srcs = [":" + tree_name],
        mtree = ":" + rooted_name,
        **kwargs
    )
