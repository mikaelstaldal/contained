//! # contained
//!
//! Run Podman containers.

#![cfg(target_os = "linux")]

use anyhow::{anyhow, Context};
use std::env::current_dir;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs, io};

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

const X11_SOCKET: &str = "/tmp/.X11-unix";

pub fn run(
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
    let mut command = run_cmd(
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

fn run_cmd(
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

    let program = fs::canonicalize(program).context(format!("Program {:?} not found", program))?;
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

pub fn run_image(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::error;

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

        let cmd = run_cmd(
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
