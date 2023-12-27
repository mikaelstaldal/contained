//! # Contained
//!
//! Run a program in a Docker container.

use std::{fs, thread};
use std::env::current_dir;
use std::path::Path;
use std::sync::mpsc;

use anyhow::{anyhow, Context};
use users::{get_effective_gid, get_effective_uid};

use crate::docker_client::{attach_container, Bind, create_container, start_container, Tmpfs, wait_container};

const SYSTEM_MOUNTS: [&str; 8] = ["/bin", "/etc", "/lib", "/lib32", "/lib64", "/libx32", "/sbin", "/usr"];
const TMPFS_MOUNTS: [&str; 4] = ["/tmp", "/var/tmp", "/run", "/var/run"];

pub fn run(program: &Path, arguments: &[String], network: &str, mount_current_dir: bool, writable: bool) -> Result<(String, u8), anyhow::Error> {
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
        program.to_str().ok_or(anyhow!("Program name is not valid Unicode"))?,
        arguments,
        network,
        &user,
        &binds,
        &TMPFS_MOUNTS.map(|path| Tmpfs::new(path, &["rw", "noexec"])),
        true,
        working_dir)
        .context("Unable to create container")?;

    attach_container(&id).context("Unable to attach container")?;

    let id_copy = id.clone();
    let (tx, wait_rx) = mpsc::channel();
    thread::spawn(move || {
        tx.send(wait_container(&id_copy)).expect("Unable to send wait result");
    });

    start_container(&id).context("Unable to start container")?;

    wait_rx.recv()?.context("Unable to wait for container").map(|status_code| (id, status_code))
}

mod docker_client;
