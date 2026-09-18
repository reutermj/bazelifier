//! Converted modules a project depends on, as the translator sees them at
//! conversion time.
//!
//! A project that links a library it does not build has to be converted
//! AGAINST that library's conversion: its real configure has to find the
//! library's headers and `.so` on this machine, or no ground truth is ever
//! produced. The source is the dependency's own ground-truth build,
//! `make install`ed into a scratch tree by its conversion action and handed
//! to this one as an input (`--dependency <module tree>:<install tree>`).
//! Nothing from that tree ships: it exists so the dependent's build can run,
//! and so a link input whose path lies inside it can be resolved — by PATH,
//! which the input states, never by library name — to the module that built
//! it. See docs/architecture/build-verification.md, "Depending on another
//! converted module".
//!
//! Frontend-agnostic on purpose: everything here keys on a path or a file
//! the translator wrote itself (`MODULE.bazel`, `TARGETS`), never on which
//! build system either side used.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::{ExternalDependency, ModuleDependency};

/// One converted dependency: what it is called and what it built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub module: ModuleDependency,
    /// Library targets the dependency's module exposes, by the file stem
    /// they were built as (`libgreet`), from its `TARGETS` manifest.
    pub libraries: Vec<Library>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// File stem of the built artifact: `libgreet` for `libgreet.la`,
    /// `libgreet.so` or `libgreet.a`. The consumer's link line names the
    /// library by this stem (`-lgreet`), whatever extension it resolved to.
    pub stem: String,
    /// The Bazel target name inside the dependency's module.
    pub target: String,
    pub shared: bool,
}

/// Every dependency, plus the one directory their install trees were merged
/// into so a single `PKG_CONFIG_SYSROOT_DIR` covers all of them.
#[derive(Debug, Default)]
pub struct Dependencies {
    dependencies: Vec<Dependency>,
    /// Where the merged install trees live. `None` when there are no
    /// dependencies, so callers can tell "nothing to point configure at"
    /// from "an empty sysroot".
    sysroot: Option<PathBuf>,
    /// Sysroot-relative path of every installed file, to the index in
    /// `dependencies` of the module that installed it. Recorded while
    /// merging, because after the merge the files are indistinguishable.
    origin: HashMap<PathBuf, usize>,
}

/// The install prefix every dependency's ground-truth build is installed
/// under, inside its scratch tree. Fixed rather than the tree's own absolute
/// path so the `.pc` files a dependency installs stay path-free
/// (`prefix=/usr/local`) and `PKG_CONFIG_SYSROOT_DIR` does the relocation —
/// an absolute prefix baked at the dependency's conversion would be wrong
/// by the time the tree is staged into the dependent's action.
pub const INSTALL_PREFIX: &str = "/usr/local";

impl Dependencies {
    /// Loads each `<module tree>:<install tree>` spec and merges the install
    /// trees into `sysroot_dir`. The module tree supplies the name, version
    /// and library targets; the install tree supplies what the dependent's
    /// build will find.
    pub fn load(specs: &[String], sysroot_dir: &Path) -> Result<Self, Error> {
        if specs.is_empty() {
            return Ok(Self::default());
        }
        let mut deps = Self {
            sysroot: Some(sysroot_dir.to_path_buf()),
            ..Self::default()
        };
        std::fs::create_dir_all(sysroot_dir)?;
        for spec in specs {
            let (module_tree, install_tree) = spec
                .split_once(':')
                .ok_or_else(|| Error::Spec { spec: spec.clone() })?;
            let module_tree = Path::new(module_tree);
            let dependency = Dependency {
                module: read_module(module_tree)?,
                libraries: read_libraries(module_tree)?,
            };
            let index = deps.dependencies.len();
            deps.dependencies.push(dependency);
            merge_tree(
                Path::new(install_tree),
                sysroot_dir,
                Path::new(""),
                &mut |relative| {
                    // Two dependencies installing one path is a real
                    // conflict (two modules claiming `include/config.h`),
                    // not something to resolve by order.
                    if let Some(prior) = deps.origin.insert(relative.to_path_buf(), index) {
                        return Err(Error::Conflict {
                            path: relative.display().to_string(),
                            first: deps.dependencies[prior].module.name.clone(),
                            second: deps.dependencies[index].module.name.clone(),
                        });
                    }
                    Ok(())
                },
            )?;
        }
        Ok(deps)
    }

    /// Environment that makes a configure script find the dependencies:
    /// pkg-config relocated into the sysroot, and the include/lib
    /// directories for checks that do not go through pkg-config
    /// (`AC_CHECK_LIB`, `--with-foo=DIR`-less projects).
    pub fn configure_env(&self) -> Vec<(String, String)> {
        let Some(sysroot) = &self.sysroot else {
            return Vec::new();
        };
        let prefix = sysroot.join(INSTALL_PREFIX.trim_start_matches('/'));
        vec![
            (
                "PKG_CONFIG_LIBDIR".to_string(),
                prefix.join("lib/pkgconfig").display().to_string(),
            ),
            (
                "PKG_CONFIG_SYSROOT_DIR".to_string(),
                sysroot.display().to_string(),
            ),
            (
                "CPPFLAGS".to_string(),
                format!("-I{}", prefix.join("include").display()),
            ),
            (
                "LDFLAGS".to_string(),
                format!("-L{}", prefix.join("lib").display()),
            ),
        ]
    }

    /// The modules the dependent ends up depending on, in spec order.
    pub fn modules(&self) -> Vec<ModuleDependency> {
        self.dependencies.iter().map(|d| d.module.clone()).collect()
    }

    /// Resolves a library file the link line named by absolute path.
    pub fn resolve_path(&self, path: &Path) -> Option<ExternalDependency> {
        let sysroot = self.sysroot.as_deref()?;
        let relative = path.strip_prefix(sysroot).ok()?;
        let index = *self.origin.get(relative)?;
        let stem = library_stem(path)?;
        self.library(index, &stem)
    }

    /// Resolves a `-l<name>` against the `-L` directories the same link line
    /// carried, the way the linker would: the first directory holding
    /// `lib<name>.so` or `lib<name>.a` wins. `None` when no directory inside
    /// the sysroot has it — a system library, or a genuinely unconverted
    /// one; the caller decides which.
    pub fn resolve_name(&self, name: &str, search_dirs: &[PathBuf]) -> Option<ExternalDependency> {
        for dir in search_dirs {
            for candidate in [format!("lib{name}.so"), format!("lib{name}.a")] {
                let path = dir.join(candidate);
                if let Some(found) = self.resolve_path(&path) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// The real shared-library file behind an external dependency, for
    /// staging beside the dependent's ground-truth binaries: the binary's
    /// `DT_NEEDED` names it, and the sysroot is gone by the time the
    /// comparison runs.
    pub fn shared_library_path(&self, dep: &ExternalDependency) -> Option<PathBuf> {
        let sysroot = self.sysroot.as_deref()?;
        let index = self
            .dependencies
            .iter()
            .position(|d| d.module.name == dep.module)?;
        let library = self.dependencies[index]
            .libraries
            .iter()
            .find(|l| l.target == dep.target && l.shared)?;
        self.origin
            .iter()
            .filter(|(_, i)| **i == index)
            .map(|(rel, _)| rel)
            .find(|rel| {
                rel.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == format!("{}.so", library.stem))
            })
            .map(|rel| sysroot.join(rel))
    }

    fn library(&self, index: usize, stem: &str) -> Option<ExternalDependency> {
        let dependency = &self.dependencies[index];
        let library = dependency.libraries.iter().find(|l| l.stem == stem)?;
        Some(ExternalDependency {
            module: dependency.module.name.clone(),
            target: library.target.clone(),
            shared: library.shared,
        })
    }
}

/// `libgreet` from `libgreet.so`, `libgreet.so.1.0.0`, `libgreet.a` or
/// `libgreet.la`: the name before the first extension. `None` for a path
/// with no file name.
pub fn library_stem(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    Some(
        name.split_once('.')
            .map_or(name, |(stem, _)| stem)
            .to_string(),
    )
}

/// `module(name = "...", version = "...")` from a module tree the translator
/// wrote. Read back out of the file rather than passed as a flag because the
/// Bazel rule that stages the tree cannot read its contents at analysis
/// time — and codegen renders the block in exactly one form
/// (`render_module_bazel`), which is the same reliance
/// `validation_workspace.bzl` already makes.
fn read_module(module_tree: &Path) -> Result<ModuleDependency, Error> {
    let path = module_tree.join("MODULE.bazel");
    let text = std::fs::read_to_string(&path).map_err(|e| Error::ModuleTree {
        path: path.display().to_string(),
        source: e,
    })?;
    let field = |key: &str| -> Option<String> {
        text.lines()
            .map(str::trim)
            .find_map(|line| line.strip_prefix(&format!("{key} = \"")))
            .and_then(|rest| rest.split('"').next())
            .map(str::to_string)
    };
    let name = field("name").ok_or_else(|| Error::ModuleTree {
        path: path.display().to_string(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "no `name = \"...\"` line in the module() block",
        ),
    })?;
    Ok(ModuleDependency {
        name,
        version: field("version"),
    })
}

/// The `library <target> <stem> <shared|static>` lines of a module's
/// `TARGETS` manifest — see `main::write_targets_manifest`.
fn read_libraries(module_tree: &Path) -> Result<Vec<Library>, Error> {
    let path = module_tree.join("TARGETS");
    let text = std::fs::read_to_string(&path).map_err(|e| Error::ModuleTree {
        path: path.display().to_string(),
        source: e,
    })?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            match (fields.next(), fields.next(), fields.next(), fields.next()) {
                (Some("library"), Some(target), Some(stem), Some(linkage)) => Some(Library {
                    stem: stem.to_string(),
                    target: target.to_string(),
                    shared: linkage == "shared",
                }),
                _ => None,
            }
        })
        .collect())
}

/// Copies `src` into `dst`, calling `on_file` with each file's path
/// relative to the sysroot root before writing it. Symlinks are followed:
/// libtool installs `libgreet.so -> libgreet.so.1 -> libgreet.so.1.0.0`, and
/// a link into a tree Bazel will discard is worse than a copy.
fn merge_tree(
    src: &Path,
    dst: &Path,
    relative: &Path,
    on_file: &mut dyn FnMut(&Path) -> Result<(), Error>,
) -> Result<(), Error> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let child_relative = relative.join(entry.file_name());
        let child_src = entry.path();
        let child_dst = dst.join(&child_relative);
        // `metadata` follows symlinks, so a linked directory recurses and a
        // linked file copies its content.
        if std::fs::metadata(&child_src)?.is_dir() {
            std::fs::create_dir_all(&child_dst)?;
            merge_tree(&child_src, dst, &child_relative, on_file)?;
        } else {
            on_file(&child_relative)?;
            if let Some(parent) = child_dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&child_src, &child_dst)?;
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Spec {
        spec: String,
    },
    ModuleTree {
        path: String,
        source: std::io::Error,
    },
    Conflict {
        path: String,
        first: String,
        second: String,
    },
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Spec { spec } => write!(
                f,
                "--dependency {spec:?}: expected <converted module tree>:<install tree>"
            ),
            Error::ModuleTree { path, source } => {
                write!(f, "cannot read the converted dependency's {path}: {source}")
            }
            Error::Conflict {
                path,
                first,
                second,
            } => write!(
                f,
                "dependencies {first} and {second} both install {path}; two modules \
                 cannot own one installed file"
            ),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bzlf_deps_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A converted dependency as its conversion leaves it: the module tree
    /// with the two files this module reads, and an install tree laid out
    /// the way `make install DESTDIR=` lays one out.
    fn fake_dependency(root: &Path, name: &str, version: &str) -> (PathBuf, PathBuf) {
        let module = root.join(format!("{name}_module"));
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(
            module.join("MODULE.bazel"),
            format!("module(\n    name = \"{name}\",\n    version = \"{version}\",\n)\n"),
        )
        .unwrap();
        std::fs::write(
            module.join("TARGETS"),
            format!("binary {name}-demo\nlibrary lib{name}_la lib{name} shared\n"),
        )
        .unwrap();
        let install = root.join(format!("{name}_install"));
        let lib = install.join("usr/local/lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(install.join("usr/local/include")).unwrap();
        std::fs::write(lib.join(format!("lib{name}.so.1.0.0")), "elf").unwrap();
        std::os::unix::fs::symlink(
            format!("lib{name}.so.1.0.0"),
            lib.join(format!("lib{name}.so")),
        )
        .unwrap();
        std::fs::write(install.join(format!("usr/local/include/{name}.h")), "").unwrap();
        (module, install)
    }

    #[test]
    fn a_link_input_inside_the_sysroot_resolves_to_the_module_that_installed_it() {
        let root = scratch("resolve");
        let (module, install) = fake_dependency(&root, "greet", "1.2");
        let sysroot = root.join("sysroot");
        let deps = Dependencies::load(
            &[format!("{}:{}", module.display(), install.display())],
            &sysroot,
        )
        .unwrap();

        let libdir = sysroot.join("usr/local/lib");
        let expected = ExternalDependency {
            module: "greet".to_string(),
            target: "libgreet_la".to_string(),
            shared: true,
        };
        assert_eq!(
            deps.resolve_name("greet", &[libdir.clone()]),
            Some(expected.clone()),
            "-L<sysroot lib> -lgreet is the shape pkg-config produces"
        );
        assert_eq!(
            deps.resolve_path(&libdir.join("libgreet.so")),
            Some(expected),
            "libtool spells the same input as an absolute path"
        );
        assert_eq!(
            deps.resolve_name("greet", &[PathBuf::from("/usr/lib")]),
            None,
            "a directory outside the sysroot is not ours to resolve, even for a name we know"
        );
        assert_eq!(
            deps.resolve_name("curl", &[libdir]),
            None,
            "a name nothing installed stays unresolved — that is the escalation's input"
        );
        assert_eq!(
            deps.modules(),
            vec![ModuleDependency {
                name: "greet".to_string(),
                version: Some("1.2".to_string())
            }]
        );
        assert!(
            sysroot.join("usr/local/lib/libgreet.so").is_file()
                && !sysroot.join("usr/local/lib/libgreet.so").is_symlink(),
            "the merged tree carries real files, not links into a tree Bazel discards"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn two_dependencies_installing_one_path_is_an_error_not_an_order() {
        let root = scratch("conflict");
        let (m1, i1) = fake_dependency(&root, "greet", "1.0");
        let (m2, i2) = fake_dependency(&root, "shout", "1.0");
        // Make shout also install greet's header.
        std::fs::write(i2.join("usr/local/include/greet.h"), "").unwrap();
        let err = Dependencies::load(
            &[
                format!("{}:{}", m1.display(), i1.display()),
                format!("{}:{}", m2.display(), i2.display()),
            ],
            &root.join("sysroot"),
        )
        .err()
        .expect("must refuse");
        let text = err.to_string();
        assert!(
            text.contains("greet") && text.contains("shout") && text.contains("greet.h"),
            "{text}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn no_dependencies_means_no_sysroot_and_no_environment() {
        let deps = Dependencies::load(&[], Path::new("/nonexistent")).unwrap();
        assert!(deps.modules().is_empty());
        assert!(deps.configure_env().is_empty());
        assert_eq!(deps.resolve_name("m", &[PathBuf::from("/usr/lib")]), None);
    }

    #[test]
    fn library_stem_drops_every_extension() {
        for (path, stem) in [
            ("lib/libgreet.so", "libgreet"),
            ("lib/libgreet.so.1.0.0", "libgreet"),
            ("libgreet.a", "libgreet"),
            ("libgreet.la", "libgreet"),
        ] {
            assert_eq!(
                library_stem(Path::new(path)).as_deref(),
                Some(stem),
                "{path}"
            );
        }
    }
}
