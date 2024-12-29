//! # Contained
//!
//! Run a program in a Docker container.

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

    /// Image to use
    #[arg(long, default_value = "empty")]
    image: String,

    /// Network mode
    #[arg(long, default_value = "none")]
    network: String,

    /// Mount current directory
    #[arg(long, conflicts_with = "current_dir_writable")]
    current_dir: bool,

    /// Mount current directory writable
    #[arg(long, conflicts_with = "current_dir")]
    current_dir_writable: bool,

    /// Mount additional directory read-only
    #[arg(long)]
    mount: Vec<String>,

    /// Mount additional directory writable
    #[arg(long)]
    mount_writable: Vec<String>,

    /// Pass environment variable
    #[arg(short, long)]
    env: Vec<String>,

    /// Working directory
    #[arg(short, long)]
    workdir: Option<String>,

    /// Run GUI X11 application
    #[arg(short = 'X')]
    x11: bool,
}

fn main() -> Result<ExitCode, anyhow::Error> {
    let cli = Cli::parse();
    let (_, status_code) = contained::run(
        &cli.image,
        &cli.program,
        &cli.arguments,
        &cli.network,
        cli.current_dir || cli.current_dir_writable,
        cli.current_dir_writable,
        &cli.mount,
        &cli.mount_writable,
        &cli.env,
        cli.workdir,
        cli.x11,
    )?;
    Ok(ExitCode::from(status_code))
}
