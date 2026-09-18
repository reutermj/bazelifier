# A dependent's ground-truth build finds its dependencies through a relocated sysroot

The problem: to convert a project that links a library another project
builds (fixture 012 links fixture 011's libgreet; PMIx links libevent and
hwloc), the translator has to run the project's REAL configure and make on
the conversion machine, and that configure has to find the library. The
library is not installed anywhere — its own conversion just built it in a
scratch directory.

What works, and the two details that cost a rebuild each:

1. **The dependency's conversion action also runs `make install`**, as a
   `DESTDIR` into a second declared output. The project's own install rule
   lays out headers, libraries and `.pc` files exactly where a downstream
   configure looks. Nothing in that tree ships; it exists only to be an
   input to a dependent's conversion (`ConvertedProjectInfo.install`).
2. **Configure with `--prefix=/usr/local`, not the tree's own path.** The
   `.pc` file then says `prefix=/usr/local`, which is wrong on its face and
   exactly right in practice: `PKG_CONFIG_SYSROOT_DIR=<staged tree>` makes
   pkg-config prepend the staged tree to every `-I` and `-L` it emits, and
   `PKG_CONFIG_LIBDIR=<staged tree>/usr/local/lib/pkgconfig` makes it look
   only there. A prefix baked as the dependency's absolute output path would
   be wrong by the time the tree is an input to a different action in a
   different sandbox; the fixed prefix makes the tree relocatable. The
   translator's `dependencies::INSTALL_PREFIX` is the one home for the value.
3. **Several dependencies merge into ONE sysroot**, because
   `PKG_CONFIG_SYSROOT_DIR` holds one path. The merge records which module
   installed each file, and refuses two modules installing one path.
4. **The link line then names the library by an absolute path into the
   sysroot** — `-L<sysroot>/usr/local/lib -lgreet` from pkg-config, or the
   `.so` path outright from libtool — and that path is what resolves it to
   the module that built it. By path, never by name: `-lgreet` alone says
   nothing about who provides it.
5. **Every path in configure's environment must be absolute.** The first
   Bazel run failed with `configure step failed:` and an EMPTY message: the
   sysroot path was execroot-relative, configure runs from the build
   directory, so `PKG_CONFIG_LIBDIR` pointed at nothing and pkg-config
   reported "not found" to a log nobody printed. By hand, with absolute
   paths, the same conversion succeeded — which is the tell for this class:
   works outside Bazel, fails inside, empty error.
6. **The dependency's `.so` has to be staged beside the dependent's ground
   truth**, since the comparison runs the ground-truth binary long after the
   sysroot is gone and its `DT_NEEDED` names `libgreet.so.1`. Staged by NAME
   (`libgreet.so*`), not by resolving the symlink chain: the sysroot merge
   copied each link as a real file, so the chain's names no longer share an
   inode and the inode-based stager found one of three.

What does NOT need doing: a `--with-greet=DIR` flag, an rpath, or any host
package. The environment above is enough for `PKG_CHECK_MODULES` and for
plain `AC_CHECK_LIB` (`CPPFLAGS`/`LDFLAGS` carry the sysroot's include and
lib directories for the latter).

Where it is pinned: `dependencies.rs` unit tests for the resolution,
`autotools.rs` tests for both directions of the link line (edge with the
dependency, escalation without), and fixtures `autotools/011` + `012` for the
whole path through Bazel and the unpacked workspace.
