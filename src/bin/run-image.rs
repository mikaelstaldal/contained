//! # run-image
//!
//! Run a docker image

use std::process::ExitCode;

use clap::Parser;

#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Image to run
    image: String,

    /// Arguments to the image
    arguments: Vec<String>,

    /// The entrypoint
    #[arg(long)]
    entrypoint: Option<String>,

    /// Network mode
    #[arg(long, default_value = "none")]
    network: String,

    /// Mount current directory writable
    #[arg(long)]
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
}

fn main() -> Result<ExitCode, anyhow::Error> {
    let cli = Cli::parse();
    let (_, status_code) = contained::run_image(
        &cli.image,
        &cli.arguments,
        cli.entrypoint,
        &cli.network,
        cli.current_dir_writable,
        &cli.mount,
        &cli.mount_writable,
        &cli.env,
    )?;
    Ok(ExitCode::from(status_code))
}
