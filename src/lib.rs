//! # Contained
//!
//! Run a program in a Docker container.

use std::{fs, io, thread};
use std::env::current_dir;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::mpsc;

use anyhow::{anyhow, Context};
use termion::raw::IntoRawMode;
use termion::terminal_size;
use users::{get_effective_gid, get_effective_uid};

use crate::docker_client::{attach_container, Bind, create_container, remove_container, start_container, Tmpfs, Tty, wait_container};

const ENV: [&str; 11] = [
    "LANG",
    "LC_ADDRESS",
    "LC_NAME",
    "LC_MONETARY",
    "LC_PAPER",
    "LC_IDENTIFICATION",
    "LC_TELEPHONE",
    "LC_MEASUREMENT",
    "LC_TIME",
    "LC_NUMERIC",
    "USER"
];

const SYSTEM_MOUNTS: [&str; 8] = ["/bin", "/etc", "/lib", "/lib32", "/lib64", "/libx32", "/sbin", "/usr"];
const TMPFS_MOUNTS: [&str; 4] = ["/tmp", "/var/tmp", "/run", "/var/run"];

const X11_SOCKET: &'static str = "/tmp/.X11-unix";

pub fn run(image: &str, program: &Path, arguments: &[String], network: &str, mount_current_dir: bool, mount_current_dir_writable: bool,
           mount_readonly: &[String], mount_writable: &[String], extra_env: &[String], workdir: Option<String>, x11: bool) -> Result<(String, u8), anyhow::Error> {
    let user = format!("{}:{}", get_effective_uid(), get_effective_gid());
    let program = fs::canonicalize(program)?;
    let program_dir = program.parent().ok_or(anyhow!("Invalid path"))?.to_str().ok_or(anyhow!("Program name is not valid Unicode"))?;
    let current_dir = current_dir()?;
    let current_dir_str = current_dir.to_str().ok_or(anyhow!("Current dir is not valid Unicode"))?;
    let current_dir_bind_option = [if mount_current_dir_writable { "rw" } else { "ro" }];

    let mut binds = Vec::new();
    let working_dir: &str;
    if mount_current_dir {
        if program_dir != current_dir_str {
            binds.push(Bind::new(program_dir, program_dir, &["ro"]));
        }
        binds.push(Bind::new(current_dir_str, current_dir_str, &current_dir_bind_option));
        working_dir = workdir.as_deref().unwrap_or(current_dir_str);
    } else {
        binds.push(Bind::new(program_dir, program_dir, &["ro"]));
        working_dir = workdir.as_deref().unwrap_or("/");
    }
    for path in SYSTEM_MOUNTS {
        if Path::new(path).exists() {
            binds.push(Bind::new(path, path, &["ro"]));
        }
    }
    for path in mount_readonly {
        binds.push(Bind::new(path, path, &["ro"]));
    }
    for path in mount_writable {
        binds.push(Bind::new(path, path, &["rw"]));
    }

    let absolute_working_dir = fs::canonicalize(working_dir)?;
    let absolute_working_dir_str = absolute_working_dir.to_str().ok_or(anyhow!("Working directory name is not valid Unicode"))?;

    let is_tty = io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal();
    let tty = if is_tty {
        let (width, height) = terminal_size()?;
        Some(Tty::new(height, width))
    } else {
        None
    };

    let mut env = Vec::new();
    for e in ENV {
        env.push(e.to_string());
    }
    for e in extra_env {
        env.push(e.to_string());
    }

    if x11 {
        env.push("DISPLAY".to_string());
        binds.push(Bind::new(X11_SOCKET, X11_SOCKET, &[]));
    }

    let id = create_container(
        image,
        program.to_str().ok_or(anyhow!("Program name is not valid Unicode"))?,
        arguments,
        network,
        &user,
        &env,
        &binds,
        &TMPFS_MOUNTS.map(|path| Tmpfs::new(path, &["rw", "noexec"])),
        true,
        absolute_working_dir_str,
        &tty)
        .context("Unable to create container")?;

    if tty.is_some() {
        let stdout = io::stdout().into_raw_mode()?; // set stdout in raw mode so we can do TTY
        let result = run_container(&id);
        drop(stdout); // restore terminal mode
        result
    } else {
        run_container(&id)
    }
}

fn run_container(id: &str) -> Result<(String, u8), anyhow::Error> {
    attach_container(&id).context("Unable to attach container")?;

    let id_copy = id.to_string();
    let (tx, wait_rx) = mpsc::channel();
    thread::Builder::new().name("wait".to_string()).spawn(move || {
        tx.send(wait_container(&id_copy)).expect("Unable to send wait result");
    })?;

    start_container(&id).context("Unable to start container")?;

    let result = wait_rx.recv()?.context("Unable to wait for container").map(|status_code| (id.to_string(), status_code))?;

    remove_container(&id).context("Unable to remove container")?;

    Ok(result)
}

mod docker_client;
