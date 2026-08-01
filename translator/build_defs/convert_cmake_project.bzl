"""Rule that runs the bazelifier translator against a CMake project fixture,
producing a standalone Bazel module (its own MODULE.bazel + BUILD.bazel,
plus copied sources) as a tree artifact.

See docs/architecture/bazel-codegen.md: the whole point of this rule is
that its output must be a genuinely independent Bazel module, buildable
with no reference back to bazelifier's own MODULE.bazel/toolchains. This
rule only produces that output; validating it as its own workspace happens
out-of-band (see docs/architecture/build-verification.md).
"""

def _derived_source_dir(srcs, marker):
    """Returns the execroot-relative dir of the top-level `marker` file.

    Used when the BUILD author can't name the staged path (a corpus project
    fetched via git_repository stages under external/<repo>+/).

    A project can carry nested marker files — CMakeLists.txt pulled in via
    add_subdirectory, or (like tinyxml2's test/) standalone sub-projects that
    find_package() the main one; automake's SUBDIRS produces the same shape.
    The shallowest path is the project root unambiguously; deeper ones are
    subdirectories of it, so picking the minimum-depth marker is correct
    regardless of which kind they are. A tie at the shallowest depth would be
    two roots at once, which is genuinely ambiguous and fails. See bzl-c54.4.
    """
    roots = [f.dirname for f in srcs if f.basename == marker]
    if not roots:
        fail("convert_cmake_project: to derive source_dir, srcs must contain " +
             "a %s, found none" % marker)
    min_depth = min([len(r.split("/")) for r in roots])
    shallowest = [r for r in roots if len(r.split("/")) == min_depth]
    if len(shallowest) != 1:
        fail("convert_cmake_project: ambiguous project root — multiple " +
             "top-level %s at the same depth: %s" % (marker, shallowest))
    return shallowest[0]

# What each frontend's projects are recognised and described by. The rule is
# otherwise identical for both — the translator detects the frontend from the
# source directory itself (see main.rs::detect_frontend), so nothing here
# needs to tell it which to use.
_FRONTENDS = {
    "cmake": struct(marker = "CMakeLists.txt", label = "CMake"),
    "autotools": struct(marker = "configure.ac", label = "Autotools"),
}

def _convert_cmake_project_impl(ctx):
    frontend = _FRONTENDS[ctx.attr.frontend]
    marker = frontend.marker
    out_dir = ctx.actions.declare_directory(ctx.attr.name)

    # A scratch directory for the CMake configure step (`cmake -B`). Kept
    # separate from out_dir so the translator's declared output only ever
    # contains the generated module, not CMake's own build byproducts.
    build_scratch = ctx.actions.declare_directory(ctx.attr.name + "_build")

    srcs = ctx.files.srcs
    if not srcs:
        fail("convert_cmake_project: srcs must not be empty")

    # source_dir is an execroot-relative path. An in-repo fixture names it
    # with package_name(); a corpus project fetched via git_repository stages
    # under a path the BUILD author can't name, so it leaves source_dir empty
    # and the rule derives it from where the marker file actually landed.
    # See bzl-c54.4.
    source_dir = ctx.attr.source_dir or _derived_source_dir(srcs, marker)

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

    # Passed explicitly rather than left to detection. A project can ship BOTH
    # build systems — xz has CMakeLists.txt and configure.ac — and which one to
    # convert is then a real choice the BUILD author is making, not something
    # to infer. Without this the rule silently converted xz with the CMake
    # frontend and failed deep inside it on a File API reply that was never
    # written.
    args.add("--frontend", ctx.attr.frontend)

    ctx.actions.run(
        outputs = [out_dir, build_scratch],
        inputs = srcs,
        executable = ctx.executable._bazelifier,
        arguments = [args],
        mnemonic = "ConvertProject",
        progress_message = "Converting %s project %s to a Bazel module" % (frontend.label, source_dir),
        # The translator shells out to the project's own build system (see
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
            doc = "All files belonging to the project (its build files and sources).",
        ),
        "frontend": attr.string(
            default = "cmake",
            values = ["cmake", "autotools"],
            doc = "Which build system to convert the project with. Passed through to the translator, so a project shipping both CMakeLists.txt and configure.ac converts with the one named here rather than whichever detection prefers. Also selects the file this rule looks for when deriving source_dir.",
        ),
        "source_dir": attr.string(
            doc = "Path (relative to the execroot) to the project's root directory, i.e. the directory containing its CMakeLists.txt or configure.ac. Leave empty to derive it from the single marker file in srcs — required when the sources come from an external repo (a corpus project) whose staged path the BUILD author can't name.",
        ),
        "deliverable_root": attr.string(
            default = "",
            doc = "Path (relative to the execroot) to the root of the source deliverable being converted — the directory the project ships as its sources. The generated module may grow to cover anything the build references inside it, so set this wider than source_dir when the project compiles sources from a sibling directory that ships alongside it. Anything referenced from outside it is escalated via needs_attention/ rather than quietly packaged. Defaults to source_dir, i.e. the project converts on its own.",
        ),
        "_bazelifier": attr.label(
            default = Label("//translator:bazelifier"),
            executable = True,
            cfg = "exec",
        ),
    },
)

def convert_autotools_project(name, **kwargs):
    """Converts an Autotools project into a standalone Bazel module.

    A thin wrapper rather than a separate rule: the pipeline is identical, and
    the translator detects the frontend from the source directory itself. What
    differs is only which file marks the project root, which the `frontend`
    attribute selects.

    The project must be BOOTSTRAPPED — `configure` present, not just
    `configure.ac`. A released tarball already is; a git checkout needs
    `autoreconf -i` first, and the translator says so rather than failing with
    a bare "no such file or directory".
    """
    convert_cmake_project(
        name = name,
        frontend = "autotools",
        **kwargs
    )
