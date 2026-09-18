"""Rule that runs the bazelifier translator against a CMake project fixture,
producing a standalone Bazel module (its own MODULE.bazel + BUILD.bazel,
plus copied sources) as a tree artifact.

See docs/architecture/bazel-codegen.md: the whole point of this rule is
that its output must be a genuinely independent Bazel module, buildable
with no reference back to bazelifier's own MODULE.bazel/toolchains. This
rule only produces that output; validating it as its own workspace happens
out-of-band (see docs/architecture/build-verification.md).
"""

ConvertedProjectInfo = provider(
    doc = "The two trees a conversion produces: the standalone module, and the project's own ground-truth build installed under /usr/local as a DESTDIR. The install tree exists ONLY so a dependent project's conversion can configure against it (see the translator's `dependencies` module and docs/architecture/build-verification.md); it never ships.",
    fields = {
        "module": "File: the generated module tree (MODULE.bazel, BUILD.bazel, sources, ground_truth/, needs_attention/).",
        "install": "File: the ground-truth build's `make install DESTDIR=` tree.",
    },
)

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

# What each frontend's projects are recognised and described by. The marker
# both locates the project root and is the file whose presence the BUILD author
# is asserting; the chosen key is also passed to the translator as
# `--frontend`, overriding its own detection — see where that is added below
# for why detection is not enough.
_FRONTENDS = {
    "cmake": struct(marker = "CMakeLists.txt", label = "CMake"),
    "autotools": struct(marker = "configure.ac", label = "Autotools"),
}

def _convert_cmake_project_impl(ctx):
    frontend = _FRONTENDS[ctx.attr.frontend]
    marker = frontend.marker
    out_dir = ctx.actions.declare_directory(ctx.attr.name)

    # Declared as a real output, unlike the build scratch below, because a
    # DEPENDENT's action has to receive it as an input. Under a fixed
    # /usr/local prefix inside it, so the .pc files stay path-free and
    # pkg-config's sysroot relocation does the rest.
    install_dir = ctx.actions.declare_directory(ctx.attr.name + "_install")

    # A scratch directory for the CMake configure step (`cmake -B`). Kept
    # separate from out_dir so the translator's declared output only ever
    # contains the generated module, not CMake's own build byproducts.
    # NOT a declared output. Nothing consumes it — the rule returns only
    # out_dir — and declaring it makes Bazel VALIDATE it, which fails on any
    # symlink the project's own configure leaves behind (libidn2 creates a
    # GNUmakefile wrapper pointing into the source tree, whose target stops
    # existing when the action ends). A sibling path under the action's own
    # output directory is writable and Bazel discards it with the sandbox.
    build_scratch_path = out_dir.path + "_build"

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
    args.add("--build-dir", build_scratch_path)
    args.add("--out-module", out_dir.path)
    args.add("--deliverable-root", deliverable_root)

    # Passed explicitly rather than left to detection. A project can ship BOTH
    # build systems — xz has CMakeLists.txt and configure.ac — and which one to
    # convert is then a real choice the BUILD author is making, not something
    # to infer. Without this the rule silently converted xz with the CMake
    # frontend and failed deep inside it on a File API reply that was never
    # written.
    args.add("--frontend", ctx.attr.frontend)
    args.add("--install-dir", install_dir.path)

    # `--configure-arg=<value>` as one token: the values are themselves
    # flags (`--disable-openssl`), and as a separate token clap reads one as
    # the next option.
    args.add_all(ctx.attr.configure_args, format_each = "--configure-arg=%s")

    # Each converted dependency arrives as both of its trees: the module
    # (for its name, version and library targets) and the install tree (for
    # the headers and libraries this project's configure has to find). The
    # translator merges the install trees into one sysroot and resolves link
    # inputs back to the module that installed them, by path. This is the
    # conversion ORDER, expressed as Bazel's own graph: a dependency converts
    # before its dependents, and re-converting it re-converts them.
    dep_inputs = []
    for dep in ctx.attr.deps:
        info = dep[ConvertedProjectInfo]
        args.add("--dependency", "%s:%s" % (info.module.path, info.install.path))
        dep_inputs += [info.module, info.install]

    ctx.actions.run(
        outputs = [out_dir, install_dir],
        inputs = srcs + dep_inputs,
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

    # DefaultInfo carries only the module: everything downstream that lists
    # a conversion in `srcs` or `data` (the validation workspace, the
    # fixtures' own consumers) wants the module and nothing else. The install
    # tree is reachable only through the provider, i.e. only by another
    # conversion's `deps`.
    return [
        DefaultInfo(files = depset([out_dir])),
        ConvertedProjectInfo(module = out_dir, install = install_dir),
    ]

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
        "deps": attr.label_list(
            providers = [ConvertedProjectInfo],
            doc = "Other conversions this project links libraries from. Each is given to the translator as a converted module plus its install tree, the project's configure is pointed at them, and a library the link line names from inside one becomes a bazel_dep + `@module//:target` edge in the generated output. A library the link line names that resolves into none of them is escalated (unconverted_dependency), never linked from the host.",
        ),
        "configure_args": attr.string_list(
            doc = "Arguments for the project's configure step (Autotools frontend), e.g. [\"--disable-openssl\"]. These are the build DECISIONS a consumer of this project makes — Open MPI configures its bundled libevent with eight of them — and the conversion replicates those rather than whatever the host's installed packages would make configure choose on its own. Recording them here is what makes the choice visible; see docs/architecture/overview.md on replicating the build's behaviour, not this host's outcome.",
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

    A thin wrapper rather than a separate rule: the pipeline is identical. The
    `frontend` attribute is what differs, and it selects both the root marker
    (`configure.ac`) and the frontend the translator is told to use.

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
