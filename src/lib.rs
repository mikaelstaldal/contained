//! # contained
//!
//! Run Docker containers.

use std::env::current_dir;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::mpsc;
use std::{fs, io, slice, thread};

use anyhow::{anyhow, Context};
use serde_json::Value;
use termion::raw::IntoRawMode;
use termion::terminal_size;
use users::{get_effective_gid, get_effective_uid};

use crate::docker_client::{
    attach_container, create_container, create_container_body, remove_container, start_container,
    wait_container, Bind, Tmpfs, Tty,
};

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

const X11_SOCKET: &'static str = "/tmp/.X11-unix";

pub fn run(
    image: &str,
    program: &Path,
    arguments: &[String],
    network: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[String],
    mount_writable: &[String],
    extra_env: &[String],
    workdir: Option<String>,
    x11: bool,
) -> Result<(String, u8), anyhow::Error> {
    let user = format!("{}:{}", get_effective_uid(), get_effective_gid());

    let is_tty =
        io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal();
    let tty = if is_tty {
        let (width, height) = terminal_size()?;
        Some(Tty::new(height, width))
    } else {
        None
    };

    let body = run_body(
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
    let id = create_container(body).context("Unable to create container")?;

    run_container_with_tty(tty, &id)
}

fn run_body(
    image: &str,
    program: &Path,
    arguments: &[String],
    network: &str,
    user: &str,
    mount_current_dir: bool,
    mount_current_dir_writable: bool,
    mount_readonly: &[String],
    mount_writable: &[String],
    extra_env: &[String],
    workdir: Option<String>,
    x11: bool,
    tty: &Option<Tty>,
) -> Result<Value, anyhow::Error> {
    let program = fs::canonicalize(program)?;
    let program_dir = program
        .parent()
        .ok_or(anyhow!("Invalid path"))?
        .to_str()
        .ok_or(anyhow!("Program name is not valid Unicode"))?;
    let current_dir = current_dir()?;
    let current_dir_str = current_dir
        .to_str()
        .ok_or(anyhow!("Current dir is not valid Unicode"))?;
    let current_dir_bind_option = [if mount_current_dir_writable {
        "rw"
    } else {
        "ro"
    }];

    let mut binds = Vec::new();
    let working_dir: &str;
    if mount_current_dir {
        if program_dir != current_dir_str {
            binds.push(Bind::new(program_dir, program_dir, &["ro"]));
        }
        binds.push(Bind::new(
            current_dir_str,
            current_dir_str,
            &current_dir_bind_option,
        ));
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

    let body = create_container_body(
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

pub fn run_image(
    image: &str,
    arguments: &[String],
    entrypoint: Option<String>,
    network: &str,
    mount_current_dir_writable: bool,
    mount_readonly: &[String],
    mount_writable: &[String],
    extra_env: &[String],
) -> Result<(String, u8), anyhow::Error> {
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
        image,
        arguments,
        entrypoint,
        network,
        &user,
        mount_current_dir_writable,
        mount_readonly,
        mount_writable,
        extra_env,
        &tty,
    )?;
    let id = create_container(body).context("Unable to create container")?;

    run_container_with_tty(tty, &id)
}

fn run_image_body(
    image: &str,
    arguments: &[String],
    entrypoint: Option<String>,
    network: &str,
    user: &str,
    mount_current_dir_writable: bool,
    mount_readonly: &[String],
    mount_writable: &[String],
    extra_env: &[String],
    tty: &Option<Tty>,
) -> Result<Value, anyhow::Error> {
    let current_dir = current_dir()?;
    let current_dir_str = current_dir
        .to_str()
        .ok_or(anyhow!("Current dir is not valid Unicode"))?;
    let current_dir_bind_option = [if mount_current_dir_writable {
        "rw"
    } else {
        "ro"
    }];

    let mut binds = Vec::new();
    let working_dir: &str;
    binds.push(Bind::new(
        current_dir_str,
        current_dir_str,
        &current_dir_bind_option,
    ));
    working_dir = current_dir_str;
    for path in USER_MOUNTS {
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
    let absolute_working_dir_str = absolute_working_dir
        .to_str()
        .ok_or(anyhow!("Working directory name is not valid Unicode"))?;

    Ok(create_container_body(
        image,
        &Some(arguments),
        &entrypoint.as_ref().map(|e| slice::from_ref(e)),
        network,
        &user,
        &extra_env,
        &binds,
        &[],
        false,
        absolute_working_dir_str,
        &tty,
    ))
}

fn run_container_with_tty(tty: Option<Tty>, id: &str) -> Result<(String, u8), anyhow::Error> {
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
    thread::Builder::new()
        .name("wait".to_string())
        .spawn(move || {
            tx.send(wait_container(&id_copy))
                .expect("Unable to send wait result");
        })?;

    start_container(&id).context("Unable to start container")?;

    let result = wait_rx
        .recv()?
        .context("Unable to wait for container")
        .map(|status_code| (id.to_string(), status_code))?;

    remove_container(&id).context("Unable to remove container")?;

    Ok(result)
}

mod docker_client;

#[cfg(test)]
mod tests {
    use std::error;
    use super::*;

    #[test]
    fn test_run_body() -> Result<(), Box<dyn error::Error>> {
        let image = "test_image";
        let program = Path::new("/usr/bin/ls");
        let arguments = ["arg1".to_string(), "arg2".to_string()];
        let network = "host";
        let user = "1000:1000";
        let mount_current_dir = true;
        let mount_current_dir_writable = false;
        let mount_readonly = ["/readonly1".to_string(), "/readonly2".to_string()];
        let mount_writable = ["/writable1".to_string()];
        let extra_env = ["MY_ENV=123".to_string()];
        let workdir = None;
        let x11 = false;
        let tty = None;

        let body = run_body(
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
            &tty)?;

        println!("{}", serde_json::to_string_pretty(&body).expect("JSON serialize"));

        assert_eq!(body["Image"].as_str(), Some(image));
        assert_eq!(
            body["Entrypoint"]
                .as_array()
                .unwrap(),
            &[
                "/usr/bin/ls",
                "arg1",
                "arg2",
            ]
        );

        // Check for bind mounts
        let binds = body["HostConfig"]["Binds"]
            .as_array()
            .expect("Expected Binds to be an array");
        for path in &mount_readonly {
            let bind_str = format!("{}:{}:ro", path, path);
            assert!(
                binds.iter().any(|b| b.as_str() == Some(&bind_str)),
                "Expected bind mount for {} in readonly",
                path
            );
        }
        for path in &mount_writable {
            let bind_str = format!("{}:{}:rw", path, path);
            assert!(
                binds.iter().any(|b| b.as_str() == Some(&bind_str)),
                "Expected bind mount for {} in writable",
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
        let image = "test_image";
        let arguments = ["arg1".to_string(), "arg2".to_string()];
        let entrypoint = Some("test_entrypoint".to_string());
        let network = "host";
        let user = "1000:1000";
        let mount_current_dir_writable = false;
        let mount_readonly = ["/readonly1".to_string(), "/readonly2".to_string()];
        let mount_writable = ["/writable1".to_string()];
        let extra_env = ["MY_ENV=123".to_string()];
        let tty = None;

        let body = run_image_body(
            image,
            &arguments,
            entrypoint.clone(),
            network,
            user,
            mount_current_dir_writable,
            &mount_readonly,
            &mount_writable,
            &extra_env,
            &tty,
        )?;

        println!("{}", serde_json::to_string_pretty(&body).expect("JSON serialize"));

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
            let bind_str = format!("{}:{}:ro", path, path);
            assert!(
                binds.iter().any(|b| b.as_str() == Some(&bind_str)),
                "Expected bind mount for {} in readonly",
                path
            );
        }
        for path in &mount_writable {
            let bind_str = format!("{}:{}:rw", path, path);
            assert!(
                binds.iter().any(|b| b.as_str() == Some(&bind_str)),
                "Expected bind mount for {} in writable",
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
}
