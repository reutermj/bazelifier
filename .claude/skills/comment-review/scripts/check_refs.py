#!/usr/bin/env python3
"""Cross-check the references prose makes against what the tree contains.

Two classes of rot, both mechanical and both already shipped in this repo:
a comment pointing at an identifier that was renamed or deleted (`see the
`untranslatable` handling in ...`, `_combined_mtree already runs ...`), and
a markdown link or anchor that no longer resolves.

Reports candidates, not verdicts. Prose legitimately names things that do
not live in this tree -- Bazel rules, CMake commands, CMake File API
fields -- so a run with zero output is unusual and a handful of known-good
names is normal. Judge each hit; do not batch-fix them.

Usage:  python3 check_refs.py [repo_root]     (default: cwd)
Exit:   0 always. This informs a review, it is not a gate.
"""

import re
import subprocess
import sys
from pathlib import Path

# Prose lives in these; everything else is either generated or not ours.
COMMENT_GLOBS = ["*.rs", "*.bzl", "*.sh", "*.md", "*.bazel", "*.txt"]
SKIP_DIRS = {".git", "target", "tools", "node_modules", ".beads"}
SKIP_FILES = {"MODULE.bazel.lock", "Cargo.lock"}

# This skill's own docs name known-dead identifiers on purpose -- they are
# the fixtures its regression check looks for -- so scanning them reports
# the fixtures as findings. Skipping the directory keeps the signal clean;
# the tradeoff is that the skill's own prose is not itself checked.
SKIP_PATH_FRAGMENTS = (".claude/skills/comment-review",)

# Names that appear in backticks constantly and are never ours to resolve:
# Bazel rules and attrs, CMake commands and target types, File API fields,
# Rust std. Kept deliberately short -- over-filtering hides real rot.
KNOWN_EXTERNAL = {
    # Bazel
    "cc_library", "cc_binary", "cc_test", "filegroup", "genrule", "sh_test",
    "test_suite", "exports_files", "glob", "select", "module", "bazel_dep",
    "local_path_override", "register_toolchains", "srcs", "hdrs", "deps",
    "includes", "visibility", "data", "args", "tags", "linkshared",
    "allow_empty", "strip_prefix", "package_dir", "mtree_spec",
    "mtree_mutate", "tar", "DefaultInfo", "layering_check", "aquery",
    "crate.from_cargo", "crate.spec", "use_repo", "attr.string",
    # CMake
    "project", "add_executable", "add_library", "add_custom_target",
    "add_custom_command", "add_dependencies", "target_sources",
    "target_link_libraries", "target_include_directories", "find_package",
    "execute_process", "cmake_minimum_required", "FILE_SET", "BASE_DIRS",
    "EXECUTABLE", "STATIC_LIBRARY", "SHARED_LIBRARY", "OBJECT_LIBRARY",
    "MODULE_LIBRARY", "INTERFACE_LIBRARY", "UTILITY", "PUBLIC", "INTERFACE",
    "PRIVATE", "HEADERS", "CMAKE_PROJECT_VERSION", "codemodel-v2",
    "cache-v2", "compile_commands.json", "isGenerated", "fileSetIndex",
    "backtraceGraph", "compileGroups", "dependencies", "artifacts",
    "sources", "fileSets", "paths.source", "OUTPUT_NAME", "VERSION",
    # Rust / std
    "assert!", "debug_assert!", "Display", "Debug", "None", "Some", "Ok",
    "Err", "Result", "Vec", "String", "HashMap", "HashSet", "PathBuf",
    "Path", "fs::canonicalize", "ctx.actions.write", "ctx.actions.run",
    "main", "clap",
    # Tools invoked by name
    "cargo", "rustup", "rustc", "rustfmt", "clippy", "cmake", "ninja",
    "bazel", "bazelisk", "buildifier", "git", "diff", "nm", "readelf",
}

# An identifier worth checking: snake_case function-ish, `mod::path` form,
# or a private Starlark helper. Bare CamelCase and single short words are
# too noisy to be useful.
IDENT_RE = re.compile(r"`([A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+|_[a-z][a-z0-9_]{3,}|[a-z][a-z0-9_]{3,}_[a-z0-9_]+)`")
LINK_RE = re.compile(r"\[[^\]]*\]\(([^)]+)\)")

# A bare backticked word -- `untranslatable`, `discover` -- is usually just
# prose and not worth checking. It becomes checkable when the sentence
# points at it: "see the `X` handling in `Y`" asserts that X exists. That
# exact phrasing outlived its subject here for several commits, and the
# structured pattern above misses it because the name has no underscore.
REFERRING_RE = re.compile(r"\b(see|handling|handled|defined|lives in|documented|described)\b", re.I)
BARE_WORD_RE = re.compile(r"`([a-z][a-z0-9]{4,})`")


def tracked_files(root: Path):
    """Prefer git's view so ignored build output never enters the search."""
    paths = []
    try:
        out = subprocess.run(
            ["git", "-C", str(root), "ls-files"],
            capture_output=True, text=True, check=True,
        ).stdout.split("\n")
        paths = [root / p for p in out if p]
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass

    # An empty result means git ran but this is not a populated checkout
    # (an unpacked archive, a fresh `git init`). Falling through to a plain
    # walk matters more than it looks: without it the scan silently covers
    # nothing and reports a clean bill of health, which is worse than not
    # running at all.
    if not paths:
        paths = [p for p in root.rglob("*") if p.is_file()]

    for p in paths:
        if not p.is_file():
            continue
        if SKIP_DIRS & set(p.relative_to(root).parts):
            continue
        if p.name in SKIP_FILES:
            continue
        if any(frag in p.as_posix() for frag in SKIP_PATH_FRAGMENTS):
            continue
        if any(p.match(g) for g in COMMENT_GLOBS):
            yield p


def anchor_of(heading: str) -> str:
    """GitHub's slug rules, which is what the relative links assume."""
    s = heading.strip().lower()
    s = re.sub(r"[^\w\s-]", "", s)
    return re.sub(r"[\s_]+", "-", s).strip("-")


def check_links(root: Path, files):
    """A link that 404s, or an anchor naming a heading that no longer exists."""
    problems = []
    for path in files:
        if path.suffix != ".md":
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.split("\n"), 1):
            for target in LINK_RE.findall(line):
                if target.startswith(("http://", "https://", "mailto:")):
                    continue
                file_part, _, anchor = target.partition("#")
                dest = (path.parent / file_part).resolve() if file_part else path
                if not dest.exists():
                    problems.append((path, lineno, f"link target missing: {target}"))
                    continue
                if anchor and dest.suffix == ".md":
                    headings = {
                        anchor_of(m)
                        for m in re.findall(
                            r"^#{1,6}\s+(.*)$",
                            dest.read_text(encoding="utf-8", errors="replace"),
                            re.M,
                        )
                    }
                    if anchor.lower() not in headings:
                        problems.append(
                            (path, lineno, f"anchor not found in {file_part or dest.name}: #{anchor}")
                        )
    return problems


def check_identifiers(root: Path, files):
    """Backticked identifiers in prose that resolve nowhere in the tree.

    Deliberately loose about *where* it resolves: prose says `see X in Y`
    across language boundaries all the time, and the useful signal is
    simply "this name is gone", not "this name is gone from this file".
    """
    corpus = {}
    for path in files:
        corpus[path] = path.read_text(encoding="utf-8", errors="replace")
    everything = "\n".join(corpus.values())

    problems = []
    for path, text in corpus.items():
        for lineno, line in enumerate(text.split("\n"), 1):
            if not is_prose(line, path):
                continue
            candidates = list(IDENT_RE.findall(line))
            if REFERRING_RE.search(line):
                candidates += BARE_WORD_RE.findall(line)

            for ident in candidates:
                if ident in KNOWN_EXTERNAL:
                    continue
                bare = ident.split("::")[-1]
                if bare in KNOWN_EXTERNAL:
                    continue
                if not defined_anywhere(bare, everything):
                    problems.append((path, lineno, f"identifier resolves nowhere: `{ident}`"))
    return problems


def defined_anywhere(name: str, corpus: str) -> bool:
    """Does this name exist as something, anywhere in the tree?

    Generous on purpose. The question worth answering is "did this name
    disappear", not "is it declared in the canonical way" -- a reference
    that resolves to a struct field, a local binding, a CLI flag, or a
    quoted module name is a reference that still means something. Being
    strict here just buries the two real hits under fifty explicable ones.
    """
    escaped = re.escape(name)
    patterns = [
        rf"(fn|struct|enum|const|static|mod|def|impl|trait|type)\s+{escaped}\b",
        rf"\blet\s+(mut\s+)?{escaped}\b",       # local binding
        rf"^\s*(pub\s+)?{escaped}\s*:",         # struct field / param / attr key
        rf"^\s*{escaped}\s*=",                  # Starlark global, incl. rules
        rf'"{escaped}"',                        # bazel_dep name, attr string, flag
        rf"--{escaped.replace('_', '-')}\b",    # CLI flag spelled with dashes
        rf"\b{escaped}\.(rs|bzl|sh|md|py)\b",   # a filename
    ]
    return any(re.search(p, corpus, re.M) for p in patterns)


def is_prose(line: str, path: Path) -> bool:
    """Only look at comments and markdown, never at code."""
    stripped = line.strip()
    if path.suffix == ".md":
        return True
    return stripped.startswith(("//", "///", "//!", "#", '"""'))


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    files = list(tracked_files(root))

    link_problems = check_links(root, files)
    ident_problems = check_identifiers(root, files)

    def show(title, problems, note):
        print(f"\n=== {title} ({len(problems)}) ===")
        if note:
            print(note)
        for path, lineno, msg in problems:
            print(f"{path.relative_to(root)}:{lineno}: {msg}")

    show("Broken links and anchors", link_problems,
         "These are real: a relative link or heading anchor that no longer resolves.")
    show("Unresolved identifier references", ident_problems,
         "Candidates only. External names (Bazel/CMake/File API/std) show up here\n"
         "and are fine. Look for ones this repo used to define.")

    print(f"\nScanned {len(files)} files under {root}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
