//! # contained
//!
//! Various tools for sandboxing programs in Linux.

#![cfg(target_os = "linux")]

use anyhow::{anyhow, Context};
use std::env::current_dir;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::{env, fs, io, slice, thread};

use serde_json::Value;
use termion::raw::IntoRawMode;
use termion::terminal_size;
use users::{get_effective_gid, get_effective_uid};

use crate::docker_client::{Bind, DockerClient, Tmpfs, Tty};

mod docker_client;

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
    "USER",
];

const SYSTEM_MOUNTS: [&str; 8] = [
    "/bin", "/etc", "/lib", "/lib32", "/lib64", "/libx32", "/sbin", "/usr",
];

const USER_MOUNTS: [&str; 2] = ["/etc/passwd", "/etc/group"];

const X11_SOCKET: &str = "/tmp/.X11-unix";

pub fn contained_via_daemon(
    image: &str,
    program: &Path,
    arguments: &[String],
    network: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
    x11: bool,
) -> Result<(String, u8), anyhow::Error> {
    let client = DockerClient::new()?;

    let user = format!("{}:{}", get_effective_uid(), get_effective_gid());

    let is_tty =
        io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal();
    let tty = if is_tty {
        let (width, height) = terminal_size()?;
        Some(Tty::new(height, width))
    } else {
        None
    };

    let body = contained_body(
        &client,
        image,
        program,
        arguments,
        network,
        &user,
        mount_current_dir,
        mount_current_dir_writable,
        mount_readonly,
        mount_writable,
        extra_env,
        workdir,
        x11,
        &tty,
    )?;
    let id = client
        .create_container(body)
        .context("Unable to create container")?;

    run_container_with_tty(&client, tty, &id)
}

fn contained_body(
    docker_client: &DockerClient,
    image: &str,
    program: &Path,
    arguments: &[String],
    network: &str,
    user: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
    x11: bool,
    tty: &Option<Tty>,
) -> Result<Value, anyhow::Error> {
    let program = resolve_program(program)?;
    let program_dir = program.parent().ok_or(anyhow!("Invalid path"))?;
    let current_dir = current_dir()?;
    let current_dir_bind_option = [if mount_current_dir_writable {
        "rw"
    } else {
        "ro"
    }];

    let mut binds = Vec::new();
    let working_dir: &Path;
    let program_dir_str = program_dir
        .to_str()
        .ok_or(anyhow!("Program name is not valid Unicode"))?;
    if mount_current_dir {
        if program_dir != current_dir {
            binds.push(Bind::new(program_dir_str, program_dir_str, &["ro"]));
        }
        let current_dir_str = current_dir
            .to_str()
            .ok_or(anyhow!("Current dir is not valid Unicode"))?;
        binds.push(Bind::new(
            current_dir_str,
            current_dir_str,
            &current_dir_bind_option,
        ));
        working_dir = workdir.as_deref().unwrap_or(&*current_dir);
    } else {
        binds.push(Bind::new(program_dir_str, program_dir_str, &["ro"]));
        working_dir = workdir.as_deref().unwrap_or("/".as_ref());
    }
    for path in SYSTEM_MOUNTS {
        if Path::new(path).exists() {
            binds.push(Bind::new(path, path, &["ro"]));
        }
    }
    for path in mount_readonly {
        let path_str = path.to_str().ok_or(anyhow!("Path is not valid Unicode"))?;
        binds.push(Bind::new(path_str, path_str, &["ro"]));
    }
    for path in mount_writable {
        let path_str = path.to_str().ok_or(anyhow!("Path is not valid Unicode"))?;
        binds.push(Bind::new(path_str, path_str, &["rw"]));
    }

    let mut tmpfs = Vec::new();
    tmpfs.push(Tmpfs::new("/tmp", &["rw", "exec"]));
    tmpfs.push(Tmpfs::new("/var/tmp", &["rw", "exec"]));
    tmpfs.push(Tmpfs::new("/run", &["rw", "noexec"]));
    tmpfs.push(Tmpfs::new("/var/run", &["rw", "noexec"]));

    let absolute_working_dir = fs::canonicalize(working_dir)?;
    let absolute_working_dir_str = absolute_working_dir
        .to_str()
        .ok_or(anyhow!("Working directory name is not valid Unicode"))?;

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

    let mut entrypoint = arguments.to_vec();
    entrypoint.insert(
        0,
        program
            .to_str()
            .ok_or(anyhow!("Program name is not valid Unicode"))?
            .to_string(),
    );

    let body = docker_client.create_container_body(
        image,
        &None,
        &Some(&entrypoint),
        network,
        &user,
        &env,
        &binds,
        &tmpfs,
        true,
        absolute_working_dir_str,
        &tty,
    );
    Ok(body)
}

pub fn contained_via_command(
    image: &str,
    program: &Path,
    arguments: &[String],
    network: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
    x11: bool,
) -> Result<(), anyhow::Error> {
    let mut command = contained_cmd(
        image,
        program,
        arguments,
        network,
        mount_current_dir,
        mount_current_dir_writable,
        mount_readonly,
        mount_writable,
        extra_env,
        workdir,
        x11,
    )?;

    let error = command.exec();
    // If we reach this point, exec failed
    Err(anyhow::Error::new(error).context("Failed to exec"))
}

fn contained_cmd(
    image: &str,
    program: &Path,
    arguments: &[String],
    network: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
    x11: bool,
) -> Result<Command, anyhow::Error> {
    let mut cmd = podman_cmd(
        network,
        mount_current_dir,
        mount_current_dir_writable,
        mount_readonly,
        mount_writable,
        extra_env,
        workdir,
        x11,
    )?;

    cmd.arg("--read-only");

    let program = resolve_program(program)?;
    let program_dir = program
        .parent()
        .ok_or(anyhow!("Invalid path"))?
        .to_path_buf();

    if mount_current_dir {
        let current_dir = current_dir()?;

        if program_dir != current_dir {
            let mut program_dir_arg = OsString::from("type=bind,source=");
            program_dir_arg.push(program_dir.clone());
            program_dir_arg.push(",target=");
            program_dir_arg.push(program_dir.clone());
            program_dir_arg.push(",readonly");
            cmd.arg("--mount").arg(program_dir_arg);
        }
    } else {
        let mut program_dir_arg = OsString::from("type=bind,source=");
        program_dir_arg.push(program_dir.clone());
        program_dir_arg.push(",target=");
        program_dir_arg.push(program_dir.clone());
        program_dir_arg.push(",readonly");
        cmd.arg("--mount").arg(program_dir_arg);
    }

    for path in SYSTEM_MOUNTS {
        if Path::new(path).exists() {
            cmd.arg("--mount")
                .arg(format!("type=bind,source={path},target={path},readonly"));
        }
    }

    cmd.arg("--tmpfs=/tmp:rw,exec");
    cmd.arg("--tmpfs=/var/tmp:rw,exec");
    cmd.arg("--tmpfs=/run:rw,noexec");
    cmd.arg("--tmpfs=/var/run:rw,noexec");

    for e in ENV {
        cmd.arg("-e").arg(e);
    }

    cmd.arg("--entrypoint").arg(program);

    cmd.arg(image);

    for arg in arguments {
        cmd.arg(arg);
    }

    Ok(cmd)
}

pub fn run_image_via_daemon(
    image: &str,
    arguments: &[String],
    entrypoint: Option<String>,
    network: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
    x11: bool,
) -> Result<(String, u8), anyhow::Error> {
    let client = DockerClient::new()?;

    let user = format!("{}:{}", get_effective_uid(), get_effective_gid());

    let is_tty =
        io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal();
    let tty = if is_tty {
        let (width, height) = terminal_size()?;
        Some(Tty::new(height, width))
    } else {
        None
    };

    let body = run_image_body(
        &client,
        image,
        arguments,
        entrypoint,
        network,
        &user,
        mount_current_dir,
        mount_current_dir_writable,
        mount_readonly,
        mount_writable,
        extra_env,
        workdir,
        x11,
        &tty,
    )?;
    let id = client
        .create_container(body)
        .context("Unable to create container")?;

    run_container_with_tty(&client, tty, &id)
}

fn run_image_body(
    client: &DockerClient,
    image: &str,
    arguments: &[String],
    entrypoint: Option<String>,
    network: &str,
    user: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
    x11: bool,
    tty: &Option<Tty>,
) -> Result<Value, anyhow::Error> {
    let current_dir = current_dir()?;
    let current_dir_bind_option = [if mount_current_dir_writable {
        "rw"
    } else {
        "ro"
    }];
    let mut binds = Vec::new();
    let working_dir: &Path;
    if mount_current_dir {
        let current_dir_str = current_dir
            .to_str()
            .ok_or(anyhow!("Current dir is not valid Unicode"))?;
        binds.push(Bind::new(
            current_dir_str,
            current_dir_str,
            &current_dir_bind_option,
        ));
        working_dir = workdir.as_deref().unwrap_or(&*current_dir);
    } else {
        working_dir = workdir.as_deref().unwrap_or("/".as_ref());
    }
    for path in USER_MOUNTS {
        if Path::new(path).exists() {
            binds.push(Bind::new(path, path, &["ro"]));
        }
    }
    for path in mount_readonly {
        let path_str = path.to_str().ok_or(anyhow!("Path is not valid Unicode"))?;
        binds.push(Bind::new(path_str, path_str, &["ro"]));
    }
    for path in mount_writable {
        let path_str = path.to_str().ok_or(anyhow!("Path is not valid Unicode"))?;
        binds.push(Bind::new(path_str, path_str, &["rw"]));
    }

    let mut env = Vec::new();
    for e in extra_env {
        env.push(e.to_string());
    }

    if x11 {
        env.push("DISPLAY".to_string());
        binds.push(Bind::new(X11_SOCKET, X11_SOCKET, &[]));
    }

    let absolute_working_dir = fs::canonicalize(working_dir)?;
    let absolute_working_dir_str = absolute_working_dir
        .to_str()
        .ok_or(anyhow!("Working directory name is not valid Unicode"))?;

    Ok(client.create_container_body(
        image,
        &Some(arguments),
        &entrypoint.as_ref().map(|e| slice::from_ref(e)),
        network,
        &user,
        &env,
        &binds,
        &[],
        false,
        absolute_working_dir_str,
        &tty,
    ))
}

fn run_container_with_tty(
    client: &DockerClient,
    tty: Option<Tty>,
    id: &str,
) -> Result<(String, u8), anyhow::Error> {
    if tty.is_some() {
        let stdout = io::stdout().into_raw_mode()?; // set stdout in raw mode so we can do TTY
        let result = run_container(client, &id, true);
        drop(stdout); // restore terminal mode
        result
    } else {
        run_container(client, &id, false)
    }
}

fn run_container(
    client: &DockerClient,
    id: &str,
    is_tty: bool,
) -> Result<(String, u8), anyhow::Error> {
    client
        .attach_container(&id, is_tty)
        .context("Unable to attach container")?;

    let id_copy = id.to_string();
    let (tx, wait_rx) = mpsc::channel();
    thread::Builder::new()
        .name("wait".to_string())
        .spawn(move || {
            tx.send(DockerClient::new().unwrap().wait_container(&id_copy))
                .expect("Unable to send wait result");
        })?;

    client
        .start_container(&id)
        .context("Unable to start container")?;

    let result = wait_rx
        .recv()?
        .context("Unable to wait for container")
        .map(|status_code| (id.to_string(), status_code))?;

    client
        .remove_container(&id)
        .context("Unable to remove container")?;

    Ok(result)
}

pub fn run_image_via_command(
    image: &str,
    arguments: &[String],
    entrypoint: Option<String>,
    network: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
    x11: bool,
) -> Result<(), anyhow::Error> {
    let mut command = run_image_cmd(
        image,
        arguments,
        entrypoint,
        network,
        mount_current_dir,
        mount_current_dir_writable,
        mount_readonly,
        mount_writable,
        extra_env,
        workdir,
        x11,
    )?;

    let error = command.exec();
    // If we reach this point, exec failed
    Err(anyhow::Error::new(error).context("Failed to exec"))
}

fn run_image_cmd(
    image: &str,
    arguments: &[String],
    entrypoint: Option<String>,
    network: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
    x11: bool,
) -> Result<Command, anyhow::Error> {
    let mut cmd = podman_cmd(
        network,
        mount_current_dir,
        mount_current_dir_writable,
        mount_readonly,
        mount_writable,
        extra_env,
        workdir,
        x11,
    )?;

    if let Some(entrypoint) = entrypoint {
        cmd.arg("--entrypoint").arg(entrypoint);
    }

    cmd.arg(image);

    for arg in arguments {
        cmd.arg(arg);
    }

    Ok(cmd)
}

fn podman_cmd(
    network: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
    x11: bool,
) -> Result<Command, anyhow::Error> {
    let mut cmd = Command::new("podman");
    cmd.arg("run")
        .arg("--userns=keep-id")
        .arg("--cap-drop")
        .arg("ALL")
        .arg("--security-opt")
        .arg("no-new-privileges=true")
        .arg("--rm")
        .arg("--interactive");

    let is_tty =
        io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal();

    if is_tty {
        cmd.arg("--tty");
    }

    cmd.arg(format!("--network={network}"));

    if mount_current_dir {
        let current_dir = current_dir()?;

        let home_dir = PathBuf::from(env::var_os("HOME").ok_or(anyhow!("HOME not set"))?);
        if (current_dir == home_dir) || (home_dir.starts_with(current_dir.as_path())) {
            return Err(anyhow!(
                "Cannot run from home directory or its parent directories"
            ));
        }

        let mut current_dir_arg = OsString::from("type=bind,source=");
        current_dir_arg.push(current_dir.clone());
        current_dir_arg.push(",target=");
        current_dir_arg.push(current_dir.clone());
        if !mount_current_dir_writable {
            current_dir_arg.push(",readonly");
        }
        cmd.arg("--mount").arg(current_dir_arg);

        cmd.arg("--workdir");
        if let Some(workdir) = workdir {
            cmd.arg(workdir);
        } else {
            cmd.arg(current_dir);
        }
    } else {
        if let Some(workdir) = workdir {
            cmd.arg("--workdir").arg(workdir);
        }
    }

    for path in mount_readonly {
        let path = fs::canonicalize(path).context(format!("Mount point {:?} not found", path))?;
        let mut mount_arg = OsString::from("type=bind,source=");
        mount_arg.push(path.clone());
        mount_arg.push(",target=");
        mount_arg.push(path);
        mount_arg.push(",readonly");
        cmd.arg("--mount").arg(mount_arg);
    }
    for path in mount_writable {
        let path = fs::canonicalize(path).context(format!("Mount point {:?} not found", path))?;
        let mut mount_arg = OsString::from("type=bind,source=");
        mount_arg.push(path.clone());
        mount_arg.push(",target=");
        mount_arg.push(path);
        cmd.arg("--mount").arg(mount_arg);
    }

    if x11 {
        cmd.arg("-e").arg("DISPLAY");
        cmd.arg("--mount")
            .arg(format!("type=bind,source={X11_SOCKET},target={X11_SOCKET}"));
    }

    for e in extra_env {
        cmd.arg("-e").arg(e);
    }

    Ok(cmd)
}

pub fn wrapped(
    program: &Path,
    arguments: &[String],
    network: bool,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
) -> Result<(), anyhow::Error> {
    let mut command = bwrap_cmd(
        program,
        arguments,
        network,
        mount_current_dir,
        mount_current_dir_writable,
        mount_readonly,
        mount_writable,
        extra_env,
        workdir,
    )?;

    let error = command.exec();
    // If we reach this point, exec failed
    Err(anyhow::Error::new(error).context("Failed to exec bwrap"))
}

fn bwrap_cmd(
    program: &Path,
    arguments: &[String],
    network: bool,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[PathBuf],
    mount_writable: &[PathBuf],
    extra_env: &[String],
    workdir: Option<PathBuf>,
) -> Result<Command, anyhow::Error> {
    let mut cmd = Command::new("bwrap");
    cmd
        .arg("--ro-bind")
        .arg("/usr/bin")
        .arg("/usr/bin")
        .arg("--ro-bind")
        .arg("/usr/sbin")
        .arg("/usr/sbin")
        .arg("--ro-bind")
        .arg("/usr/lib")
        .arg("/usr/lib")
        .arg("--ro-bind")
        .arg("/usr/lib64")
        .arg("/usr/lib64")
        .arg("--ro-bind")
        .arg("/usr/share")
        .arg("/usr/share")
        .arg("--symlink")
        .arg("/usr/lib")
        .arg("/lib")
        .arg("--symlink")
        .arg("/usr/lib64")
        .arg("/lib64")
        .arg("--symlink")
        .arg("/usr/bin")
        .arg("/bin")
        .arg("--symlink")
        .arg("/usr/sbin")
        .arg("/sbin")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev");

    let program = resolve_program(program)?;
    let program_dir = program
        .parent()
        .ok_or(anyhow!("Invalid path"))?
        .to_path_buf();

    if mount_current_dir {
        let current_dir = current_dir()?;

        let home_dir = PathBuf::from(env::var_os("HOME").ok_or(anyhow!("HOME not set"))?);
        if (current_dir == home_dir) || (home_dir.starts_with(current_dir.as_path())) {
            return Err(anyhow!(
                "Cannot run from home directory or its parent directories"
            ));
        }

        if mount_current_dir_writable {
            cmd.arg("--bind");
        } else {
            cmd.arg("--ro-bind");
        }
        cmd.arg(current_dir.clone()).arg(current_dir.clone());

        cmd.arg("--chdir");
        if let Some(workdir) = workdir {
            cmd.arg(workdir);
        } else {
            cmd.arg(current_dir.clone());
        }

        if program_dir != current_dir {
            cmd.arg("--ro-bind").arg(program_dir.clone()).arg(program_dir.clone());
        }
    } else {
        if let Some(workdir) = workdir {
            cmd.arg("--chdir").arg(workdir);
        }

        cmd.arg("--ro-bind").arg(program_dir.clone()).arg(program_dir);
    }

    for path in mount_readonly {
        let path = fs::canonicalize(path).context(format!("Mount point {:?} not found", path))?;
        cmd.arg("--ro-bind").arg(path.clone()).arg(path);
    }
    for path in mount_writable {
        let path = fs::canonicalize(path).context(format!("Mount point {:?} not found", path))?;
        cmd.arg("--bind").arg(path.clone()).arg(path);
    }

    for e in extra_env {
        cmd.arg("--setenv").arg(e);
    }

    cmd.arg("--new-session");
    cmd.arg("--unshare-all");
    if network {
        cmd.arg("--share-net");
    }

    cmd.arg(program);

    for arg in arguments {
        cmd.arg(arg);
    }

    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error;

    #[test]
    fn test_run_body() -> Result<(), Box<dyn error::Error>> {
        let client = DockerClient::new()?;

        let image = "test_image";
        let program = Path::new("/usr/bin/ls");
        let arguments = ["arg1".to_string(), "arg2".to_string()];
        let network = "host";
        let user = "1000:1000";
        let mount_current_dir = true;
        let mount_current_dir_writable = false;
        let mount_readonly = [PathBuf::from("/readonly1"), PathBuf::from("/readonly2")];
        let mount_writable = [PathBuf::from("/writable1")];
        let extra_env = ["MY_ENV=123".to_string()];
        let workdir = None;
        let x11 = false;
        let tty = None;

        let body = contained_body(
            &client,
            image,
            program,
            &arguments,
            network,
            user,
            mount_current_dir,
            mount_current_dir_writable,
            &mount_readonly,
            &mount_writable,
            &extra_env,
            workdir,
            x11,
            &tty,
        )?;

        println!(
            "{}",
            serde_json::to_string_pretty(&body).expect("JSON serialize")
        );

        assert_eq!(body["Image"].as_str(), Some(image));
        assert_eq!(
            body["Entrypoint"].as_array().unwrap(),
            &["/usr/bin/ls", "arg1", "arg2",]
        );

        // Check for bind mounts
        let binds = body["HostConfig"]["Binds"]
            .as_array()
            .expect("Expected Binds to be an array");
        for path in &mount_readonly {
            let bind_str = format!("{}:{}:ro", path.to_str().unwrap(), path.to_str().unwrap());
            assert!(
                binds.iter().any(|b| b.as_str() == Some(&bind_str)),
                "Expected bind mount for {:?} in readonly",
                path
            );
        }
        for path in &mount_writable {
            let bind_str = format!("{}:{}:rw", path.to_str().unwrap(), path.to_str().unwrap());
            assert!(
                binds.iter().any(|b| b.as_str() == Some(&bind_str)),
                "Expected bind mount for {:?} in writable",
                path
            );
        }

        // Check for environment variables
        let env_vars = body["Env"]
            .as_array()
            .expect("Expected Env to be an array")
            .iter()
            .map(|e| e.as_str().unwrap())
            .collect::<Vec<&str>>();
        for var in &extra_env {
            assert!(
                env_vars.contains(&var.as_str()),
                "Expected environment variable '{}' in container body",
                var
            );
        }

        Ok(())
    }

    #[test]
    fn test_run_image_body() -> Result<(), Box<dyn error::Error>> {
        let client = DockerClient::new()?;

        let image = "test_image";
        let arguments = ["arg1".to_string(), "arg2".to_string()];
        let entrypoint = Some("test_entrypoint".to_string());
        let network = "host";
        let user = "1000:1000";
        let mount_current_dir = true;
        let mount_current_dir_writable = false;
        let mount_readonly = [PathBuf::from("/readonly1"), PathBuf::from("/readonly2")];
        let mount_writable = [PathBuf::from("/writable1")];
        let extra_env = ["MY_ENV=123".to_string()];
        let workdir = None;
        let x11 = false;
        let tty = None;

        let body = run_image_body(
            &client,
            image,
            &arguments,
            entrypoint.clone(),
            network,
            user,
            mount_current_dir,
            mount_current_dir_writable,
            &mount_readonly,
            &mount_writable,
            &extra_env,
            workdir,
            x11,
            &tty,
        )?;

        println!(
            "{}",
            serde_json::to_string_pretty(&body).expect("JSON serialize")
        );

        assert_eq!(body["Image"].as_str(), Some(image));
        assert_eq!(
            body["Entrypoint"]
                .as_array()
                .unwrap()
                .get(0)
                .unwrap()
                .as_str(),
            entrypoint.as_deref(),
        );

        // Check for bind mounts
        let binds = body["HostConfig"]["Binds"]
            .as_array()
            .expect("Expected Binds to be an array");
        for path in &mount_readonly {
            let bind_str = format!("{}:{}:ro", path.to_str().unwrap(), path.to_str().unwrap());
            assert!(
                binds.iter().any(|b| b.as_str() == Some(&bind_str)),
                "Expected bind mount for {:?} in readonly",
                path
            );
        }
        for path in &mount_writable {
            let bind_str = format!("{}:{}:rw", path.to_str().unwrap(), path.to_str().unwrap());
            assert!(
                binds.iter().any(|b| b.as_str() == Some(&bind_str)),
                "Expected bind mount for {:?} in writable",
                path
            );
        }

        // Check for environment variables
        let env_vars = body["Env"]
            .as_array()
            .expect("Expected Env to be an array")
            .iter()
            .map(|e| e.as_str().unwrap())
            .collect::<Vec<&str>>();
        for var in &extra_env {
            assert!(
                env_vars.contains(&var.as_str()),
                "Expected environment variable '{}' in container body",
                var
            );
        }

        Ok(())
    }

    #[test]
    fn test_run_cmd() -> Result<(), Box<dyn error::Error>> {
        let image = "test_image";
        let program = Path::new("/usr/bin/ls");
        let arguments = ["arg1".to_string(), "arg2".to_string()];
        let network = "host";
        let mount_current_dir = true;
        let mount_current_dir_writable = false;
        let mount_readonly = [PathBuf::from("/opt"), PathBuf::from("/run")];
        let mount_writable = [PathBuf::from("/var")];
        let extra_env = ["MY_ENV=123".to_string()];
        let workdir = None;
        let x11 = false;

        let cmd = contained_cmd(
            image,
            program,
            &arguments,
            network,
            mount_current_dir,
            mount_current_dir_writable,
            &mount_readonly,
            &mount_writable,
            &extra_env,
            workdir,
            x11,
        )?;

        let args: Vec<_> = cmd.get_args().map(|s| s.to_str().unwrap()).collect();

        for arg in args.iter() {
            println!("{:?}", arg);
        }

        assert!(args.contains(&"--network=host"));
        assert!(args.contains(&image));
        assert!(args.contains(&"--entrypoint"));
        assert!(args.contains(&"/usr/bin/ls"));
        assert!(args.contains(&"arg1"));
        assert!(args.contains(&"arg2"));

        // Check for bind mounts

        let path = "/bin";
        let bind = format!("type=bind,source={},target={},readonly", path, path);
        assert!(args.contains(&&*bind), "mount {bind} not found");

        let path_buf = current_dir().unwrap();
        let path = path_buf.to_str().unwrap();
        let bind = format!("type=bind,source={},target={},readonly", path, path);
        assert!(args.contains(&&*bind), "mount {bind} not found");

        for path in &mount_readonly {
            let bind = format!(
                "type=bind,source={},target={},readonly",
                path.to_str().unwrap(),
                path.to_str().unwrap()
            );
            assert!(args.contains(&&*bind), "mount {bind} not found");
        }
        for path in &mount_writable {
            let bind = format!(
                "type=bind,source={},target={}",
                path.to_str().unwrap(),
                path.to_str().unwrap()
            );
            assert!(args.contains(&&*bind), "mount {bind} not found");
        }

        // Check for environment variables
        for var in ENV {
            assert!(args.contains(&&*var), "env {var} not found");
        }
        for var in &extra_env {
            assert!(args.contains(&&**var), "env {var} not found");
        }

        Ok(())
    }

    #[test]
    fn test_run_image_cmd() -> Result<(), Box<dyn error::Error>> {
        let image = "test_image";
        let arguments = ["arg1".to_string(), "arg2".to_string()];
        let entrypoint = Some("test_entrypoint".to_string());
        let network = "host";
        let mount_current_dir = false;
        let mount_current_dir_writable = false;
        let mount_readonly = [PathBuf::from("/opt"), PathBuf::from("/run")];
        let mount_writable = [PathBuf::from("/var")];
        let extra_env = ["MY_ENV=123".to_string()];
        let workdir = None;
        let x11 = false;

        let cmd = run_image_cmd(
            image,
            &arguments,
            entrypoint.clone(),
            network,
            mount_current_dir,
            mount_current_dir_writable,
            &mount_readonly,
            &mount_writable,
            &extra_env,
            workdir,
            x11,
        )?;

        let args: Vec<_> = cmd.get_args().map(|s| s.to_str().unwrap()).collect();

        for arg in args.iter() {
            println!("{:?}", arg);
        }

        assert!(args.contains(&"--network=host"));
        assert!(args.contains(&image));
        assert!(args.contains(&"--entrypoint"));
        assert!(args.contains(&entrypoint.unwrap().as_str()));
        assert!(args.contains(&"arg1"));
        assert!(args.contains(&"arg2"));

        // Check for bind mounts
        for path in &mount_readonly {
            let bind = format!(
                "type=bind,source={},target={},readonly",
                path.to_str().unwrap(),
                path.to_str().unwrap()
            );
            assert!(args.contains(&&*bind), "mount {bind} not found");
        }
        for path in &mount_writable {
            let bind = format!(
                "type=bind,source={},target={}",
                path.to_str().unwrap(),
                path.to_str().unwrap()
            );
            assert!(args.contains(&&*bind), "mount {bind} not found");
        }

        // Check for environment variables
        for var in &extra_env {
            assert!(args.contains(&&**var), "env {var} not found");
        }

        Ok(())
    }
}

fn resolve_program(program: &Path) -> Result<PathBuf, anyhow::Error> {
    let program = if !program.is_absolute() && !program.to_str().map_or(false, |s| s.contains('/')) {
        find_in_path(program).ok_or_else(|| anyhow!("Program {:?} not found in PATH", program))?
    } else {
        fs::canonicalize(program).context(format!("Program {:?} not found", program))?
    };
    if is_executable(&program) {
        Ok(program)
    } else {
        Err(anyhow!("Program {:?} not executable", program))
    }
}

fn find_in_path(program: &Path) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths).find_map(|dir| {
            let full_path = dir.join(program);
            if full_path.is_file() {
                Some(full_path)
            } else {
                None
            }
        })
    })
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}
