//! # Contained
//!
//! Run a program in a Docker container.

use std::process::ExitCode;

use clap::Parser;

use contained::run;

#[derive(Parser)]
#[command(version)]
struct Cli {
    /// The program to run
    program: std::path::PathBuf,

    /// Arguments to the programs
    arguments: Vec<String>,

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

    /// Run GUI X11 application
    #[arg(short = 'X')]
    x11: bool,
}

fn main() -> Result<ExitCode, anyhow::Error> {
    let args = Cli::parse();
    let (_, status_code) = run(&args.program, &args.arguments, &args.network,
                               args.current_dir || args.current_dir_writable, args.current_dir_writable,
                               &args.mount, &args.mount_writable, &args.env, args.x11)?;
    Ok(ExitCode::from(status_code))
}
