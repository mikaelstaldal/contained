//! # Contained
//!
//! Run a program in a Docker container.

use std::process::ExitCode;
use clap::Parser;
use contained::run;

#[derive(Parser)]
struct Cli {
    /// The program to run
    program: std::path::PathBuf,

    /// Arguments to the programs
    arguments: Vec<String>,

    /// Network mode
    #[arg(long, default_value = "none")]
    network: String,

    /// Current dir writable
    #[arg(long)]
    current_dir_writable: bool
}

fn main() -> Result<ExitCode, anyhow::Error> {
    let args = Cli::parse();
    let (_, status_code) = run(args.program, &args.arguments, &args.network, args.current_dir_writable)?;
    Ok(ExitCode::from(status_code))
}
