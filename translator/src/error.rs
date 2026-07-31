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
//! operation that failed: `CmakeConfigureFailed` carries CMake's own stderr
//! because that text, not ours, is what tells someone what went wrong in
//! their project. See `main.rs::report` for how it reaches the terminal.

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    CmakeConfigureFailed {
        stderr: String,
    },
    CmakeBuildFailed {
        stderr: String,
    },
    NoProject,
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
            Error::CmakeConfigureFailed { stderr } => {
                write!(f, "cmake configure failed:\n{stderr}")
            }
            Error::CmakeBuildFailed { stderr } => {
                write!(f, "cmake build failed:\n{stderr}")
            }
            Error::NoProject => write!(f, "codemodel reply contains no project()"),
            Error::SourceDirOutsideDeliverableRoot {
                source_dir,
                deliverable_root,
            } => write!(
                f,
                "the CMake project directory ({source_dir}) is not inside the declared \
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
