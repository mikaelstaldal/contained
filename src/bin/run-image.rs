//! # run-image
//!
//! Run a Podman image

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

    /// Mount the current directory
    #[arg(long, conflicts_with = "current_dir_writable")]
    current_dir: bool,

    /// Mount the current directory writable
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

    /// Working directory
    #[arg(short, long)]
    workdir: Option<String>,

    /// Run GUI X11 application
    #[arg(short = 'X')]
    x11: bool,
}

fn main() -> Result<ExitCode, anyhow::Error> {
    let cli = Cli::parse();
    contained::run_image(
        &cli.image,
        &cli.arguments,
        cli.entrypoint,
        &cli.network,
        cli.current_dir || cli.current_dir_writable,
        cli.current_dir_writable,
        &cli.mount,
        &cli.mount_writable,
        &cli.env,
        cli.workdir,
        cli.x11,
    )?;
    Ok(ExitCode::SUCCESS)
}
