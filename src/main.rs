//! # Contained
//!
//! Run a program in a Docker container.

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
    network: String
}

fn main() -> Result<(), anyhow::Error> {
    let args = Cli::parse();
    let id = run(args.program, &args.arguments, &args.network)?;
    println!("{id}");
    Ok(())
}
