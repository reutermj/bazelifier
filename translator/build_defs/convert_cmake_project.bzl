"""Rule that runs the bazelifier translator against a CMake project fixture,
producing a standalone Bazel module (its own MODULE.bazel + BUILD.bazel,
plus copied sources) as a tree artifact.

See docs/architecture/bazel-codegen.md: the whole point of this rule is
that its output must be a genuinely independent Bazel module, buildable
with no reference back to bazelifier's own MODULE.bazel/toolchains. This
rule only produces that output; validating it as its own workspace happens
out-of-band (see docs/architecture/build-verification.md).
"""

def _derived_source_dir(srcs):
    """Returns the execroot-relative dir of the top-level CMakeLists.txt.

    Used when the BUILD author can't name the staged path (a corpus project
    fetched via git_repository stages under external/<repo>+/).

    A project can carry nested CMakeLists.txt files — some pulled in via
    add_subdirectory, some (like tinyxml2's test/) standalone sub-projects
    that find_package() the main one. The shallowest path is the project
    root unambiguously; deeper ones are subdirectories of it, so picking the
    minimum-depth CMakeLists.txt is correct regardless of which kind they
    are. A tie at the shallowest depth would be two roots at once, which is
    genuinely ambiguous and fails. See bzl-c54.4.
    """
    roots = [f.dirname for f in srcs if f.basename == "CMakeLists.txt"]
    if not roots:
        fail("convert_cmake_project: to derive source_dir, srcs must contain " +
             "a CMakeLists.txt, found none")
    min_depth = min([len(r.split("/")) for r in roots])
    shallowest = [r for r in roots if len(r.split("/")) == min_depth]
    if len(shallowest) != 1:
        fail("convert_cmake_project: ambiguous project root — multiple " +
             "top-level CMakeLists.txt at the same depth: %s" % shallowest)
    return shallowest[0]

def _convert_cmake_project_impl(ctx):
    out_dir = ctx.actions.declare_directory(ctx.attr.name)

    # A scratch directory for the CMake configure step (`cmake -B`). Kept
    # separate from out_dir so the translator's declared output only ever
    # contains the generated module, not CMake's own build byproducts.
    build_scratch = ctx.actions.declare_directory(ctx.attr.name + "_cmake_build")

    srcs = ctx.files.srcs
    if not srcs:
        fail("convert_cmake_project: srcs must not be empty")

    # source_dir is an execroot-relative path. An in-repo fixture names it
    # with package_name(); a corpus project fetched via git_repository stages
    # under a path the BUILD author can't name, so it leaves source_dir empty
    # and the rule derives it from where the CMakeLists.txt actually landed.
    # See bzl-c54.4.
    source_dir = ctx.attr.source_dir or _derived_source_dir(srcs)

    # An explicit deliverable_root is only meaningful for the in-repo case
    # (it names a sibling directory relative to package_name()); a derived
    # corpus source has no such wider deliverable yet, so it always converts
    # on its own (deliverable_root == source_dir).
    deliverable_root = ctx.attr.deliverable_root or source_dir

    args = ctx.actions.args()
    args.add(source_dir)
    args.add("--build-dir", build_scratch.path)
    args.add("--out-module", out_dir.path)
    args.add("--deliverable-root", deliverable_root)

    ctx.actions.run(
        outputs = [out_dir, build_scratch],
        inputs = srcs,
        executable = ctx.executable._bazelifier,
        arguments = [args],
        mnemonic = "ConvertCmakeProject",
        progress_message = "Converting CMake project %s to a Bazel module" % source_dir,
        # The translator shells out to `cmake` (see
        # docs/architecture/cmake-frontend.md: File API frontend, not yet
        # hermetic on the CMake side — docs/architecture/build-verification.md).
        # use_default_shell_env exposes the host PATH so the sandboxed
        # action can find it; this is the accepted current limitation, not
        # the end state.
        use_default_shell_env = True,
    )

    return [DefaultInfo(files = depset([out_dir]))]

convert_cmake_project = rule(
    implementation = _convert_cmake_project_impl,
    attrs = {
        "srcs": attr.label_list(
            allow_files = True,
            mandatory = True,
            doc = "All files belonging to the CMake project (CMakeLists.txt and sources).",
        ),
        "source_dir": attr.string(
            doc = "Path (relative to the execroot) to the CMake project's root directory, i.e. the directory containing its CMakeLists.txt. Leave empty to derive it from the single CMakeLists.txt in srcs — required when the sources come from an external repo (a corpus project) whose staged path the BUILD author can't name.",
        ),
        "deliverable_root": attr.string(
            default = "",
            doc = "Path (relative to the execroot) to the root of the source deliverable being converted — the directory the project ships as its sources. The generated module may grow to cover anything the build references inside it, so set this wider than source_dir when the CMake project compiles sources from a sibling directory that ships alongside it. Anything referenced from outside it is escalated via needs_attention/ rather than quietly packaged. Defaults to source_dir, i.e. the project converts on its own.",
        ),
        "_bazelifier": attr.label(
            default = Label("//translator:bazelifier"),
            executable = True,
            cfg = "exec",
        ),
    },
)
