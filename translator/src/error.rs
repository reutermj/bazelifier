//! The frontend's error type.
//!
//! Its own module so the frontend's parts can share it without any of them
//! importing another. `cmake_api` drives the conversion and `ctest` /
//! `configure_file` are extracted, dependency-free modules it calls into —
//! but every one of them can fail on I/O or a malformed reply, so the error
//! has to live somewhere all three can reach. Homing it in `cmake_api` made
//! `ctest` import from the module that calls it, which reads as a cycle even
//! though it never was one.
//!
//! Variants are named for what the user has to fix, not for the Rust
//! operation that failed: `ConfigureFailed` carries CMake's own stderr
//! because that text, not ours, is what tells someone what went wrong in
//! their project. See `main.rs::report` for how it reaches the terminal.

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    ConfigureFailed {
        stderr: String,
    },
    BuildFailed {
        stderr: String,
    },
    NoProject,
    /// No frontend recognised the source directory.
    ///
    /// Distinct from `NoProject`, which means a CMake reply arrived and was
    /// empty. Here nothing has run yet and the tool may not be CMake at all,
    /// so a message naming a codemodel would send the reader somewhere the
    /// problem is not.
    NoFrontendDetected {
        source_dir: String,
    },
    SourceDirOutsideDeliverableRoot {
        source_dir: String,
        deliverable_root: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Json(e) => write!(f, "failed to parse CMake File API JSON: {e}"),
            Error::ConfigureFailed { stderr } => {
                write!(f, "configure step failed:\n{stderr}")
            }
            Error::BuildFailed { stderr } => {
                write!(f, "build step failed:\n{stderr}")
            }
            Error::NoProject => write!(f, "codemodel reply contains no project()"),
            Error::NoFrontendDetected { source_dir } => write!(
                f,
                "no supported build system found in {source_dir}: expected a \
                 CMakeLists.txt (CMake) or configure.ac (Autotools) at its \
                 root. Pass --frontend to choose explicitly."
            ),
            Error::SourceDirOutsideDeliverableRoot {
                source_dir,
                deliverable_root,
            } => write!(
                f,
                "the project directory ({source_dir}) is not inside the declared \
                 deliverable root ({deliverable_root}); the deliverable root must contain \
                 the project being converted"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}
