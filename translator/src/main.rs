mod cmake_api;
mod codegen;
mod model;

use std::fs;
use std::path::PathBuf;

use clap::Parser;

/// Convert a CMake project into a Bazel BUILD file.
#[derive(Parser)]
struct Args {
    /// Path to the CMake project (directory containing CMakeLists.txt).
    source_dir: PathBuf,

    /// Directory to configure the CMake project in (a temp/scratch build dir).
    #[arg(long, default_value = "build")]
    build_dir: PathBuf,

    /// Path to write the generated BUILD.bazel file to.
    #[arg(long, default_value = "BUILD.bazel")]
    out: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let graph = cmake_api::discover(&args.source_dir, &args.build_dir)?;
    let rendered = codegen::render(&graph);
    fs::write(&args.out, rendered)?;

    Ok(())
}
