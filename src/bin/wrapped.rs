//! # wrapped
//!
//! Run a program in a sandbox using bubblewrap.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

#[derive(Parser)]
#[command(version)]
struct Cli {
    /// The program to run
    program: PathBuf,

    /// Arguments to the programs
    arguments: Vec<String>,

    /// Enable network access
    #[arg(long)]
    network: bool,

    /// Mount the current directory
    #[arg(long, conflicts_with = "current_dir_writable")]
    current_dir: bool,

    /// Mount the current directory writable
    #[arg(long, conflicts_with = "current_dir")]
    current_dir_writable: bool,

    /// Mount additional directory read-only
    #[arg(long)]
    mount: Vec<PathBuf>,

    /// Mount additional directory writable
    #[arg(long)]
    mount_writable: Vec<PathBuf>,

    /// Pass environment variable
    #[arg(short, long)]
    env: Vec<String>,

    /// Working directory
    #[arg(short, long)]
    workdir: Option<PathBuf>,
}

fn main() -> Result<ExitCode, anyhow::Error> {
    let cli = Cli::parse();
    contained::wrapped(
        &cli.program,
        &cli.arguments,
        cli.network,
        cli.current_dir || cli.current_dir_writable,
        cli.current_dir_writable,
        &cli.mount,
        &cli.mount_writable,
        &cli.env,
        cli.workdir,
    )?;
    Ok(ExitCode::SUCCESS)
}
