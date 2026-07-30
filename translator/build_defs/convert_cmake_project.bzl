"""Rule that runs the bazelifier translator against a CMake project fixture,
producing a standalone Bazel module (its own MODULE.bazel + BUILD.bazel,
plus copied sources) as a tree artifact.

See docs/architecture/bazel-codegen.md: the whole point of this rule is
that its output must be a genuinely independent Bazel module, buildable
with no reference back to bazelifier's own MODULE.bazel/toolchains. This
rule only produces that output; validating it as its own workspace happens
out-of-band (see docs/architecture/build-verification.md).
"""

def _convert_cmake_project_impl(ctx):
    out_dir = ctx.actions.declare_directory(ctx.attr.name)

    # A scratch directory for the CMake configure step (`cmake -B`). Kept
    # separate from out_dir so the translator's declared output only ever
    # contains the generated module, not CMake's own build byproducts.
    build_scratch = ctx.actions.declare_directory(ctx.attr.name + "_cmake_build")

    srcs = ctx.files.srcs
    if not srcs:
        fail("convert_cmake_project: srcs must not be empty")

    source_dir = ctx.attr.source_dir

    args = ctx.actions.args()
    args.add(source_dir)
    args.add("--build-dir", build_scratch.path)
    args.add("--out-module", out_dir.path)
    args.add("--deliverable-root", ctx.attr.deliverable_root or source_dir)

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
            mandatory = True,
            doc = "Path (relative to the execroot) to the CMake project's root directory, i.e. the directory containing its CMakeLists.txt.",
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
