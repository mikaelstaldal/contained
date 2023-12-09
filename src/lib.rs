//! # Contained
//!
//! Run a program in a Docker container.

use std::env::current_dir;
use std::fs;
use std::path::Path;
use anyhow::{anyhow, Context};
use tokio::runtime::Runtime;
use users::{get_effective_uid, get_effective_gid};
use crate::docker_client::{attach_container, create_container, start_container, wait_container, Bind, Tmpfs};

const SYSTEM_MOUNTS: [&str; 8] = ["/bin", "/etc", "/lib", "/lib32", "/lib64", "/libx32", "/sbin", "/usr"];
const TMPFS_MOUNTS: [&str; 4] = ["/tmp", "/var/tmp", "/run", "/var/run"];

pub fn run(program: &Path, arguments: &[String], network: &str, mount_current_dir: bool, writable: bool) -> Result<(String, u8), anyhow::Error> {
    let runtime = Runtime::new()?;

    let user = format!("{}:{}", get_effective_uid(), get_effective_gid());
    let program = fs::canonicalize(program)?;
    let program_dir = program.parent().ok_or(anyhow!("Invalid path"))?.to_str().ok_or(anyhow!("Program name is not valid Unicode"))?;
    let mut binds = vec![Bind::new(program_dir, program_dir, &["ro"])];
    let current_dir = current_dir()?;
    let current_dir_str = current_dir.to_str().ok_or(anyhow!("Current dir is not valid Unicode"))?;
    let current_dir_bind_option = [if writable { "rw" } else { "ro" }];
    let working_dir: &str;
    if mount_current_dir {
        binds.push(Bind::new(current_dir_str, current_dir_str, &current_dir_bind_option));
        working_dir = current_dir_str;
    } else {
        working_dir = "/";
    }
    for path in SYSTEM_MOUNTS {
        if Path::new(path).exists() {
            binds.push(Bind::new(path, path, &["ro"]));
        }
    }
    let id = create_container(
        &runtime,
        program.to_str().ok_or(anyhow!("Program name is not valid Unicode"))?,
        arguments,
        network,
        &user,
        &binds,
        &TMPFS_MOUNTS.map(|path| Tmpfs::new(path, &["rw", "noexec"])),
        true,
        working_dir)
        .context("Unable to create container")?;
    start_container(&runtime, &id).context("Unable to start container")?;
    attach_container(&runtime, &id);
    let status_code = wait_container(&runtime, &id).context("Unable to start container")?;
    Ok((id, status_code))
}

mod docker_client;
