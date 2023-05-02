//! # Contained
//!
//! Run a program in a Docker container.

use std::path::{Path, PathBuf};
use anyhow::Context;
use crate::docker_client::{create_container, start_container, Bind};

const SYSTEM_MOUNTS: [&str; 8] = ["/bin", "/etc", "/lib", "/lib32", "/lib64", "/libx32", "/sbin", "/usr"];

pub fn run(program: PathBuf, arguments: &[String], network: &str) -> Result<String, anyhow::Error> {
    let program_dir = program.parent().expect("Invalid path").to_str().expect("Program name is not valid Unicode");
    let mut binds = vec![Bind::new(program_dir, program_dir, &["ro"])];
    for path in SYSTEM_MOUNTS {
        if Path::new(path).exists() {
            binds.push(Bind::new(path, path, &["ro"]));
        }
    }
    let id = create_container(
        program.to_str().expect("Program name is not valid Unicode"),
        arguments,
        &binds,
        network).context("Unable to create container")?;
    start_container(&id).context("Unable to start container")?;
    Ok(id)
}

mod docker_client;
