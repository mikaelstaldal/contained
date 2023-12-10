//! # Docker Client
//!
//! `docker_client` contains functions to call the Docker daemon.

use std::collections::HashMap;
use std::io::{self, IsTerminal, Read, Write};
use std::time::Duration;
use futures::{FutureExt, StreamExt, TryFutureExt};
use futures::executor::block_on;
use hyper::{Body, Client, Method, Request, Response, StatusCode};
use hyper::upgrade::Upgraded;
use hyperlocal::{UnixClientExt, Uri};
use serde_json::json;
use serde_json::Value;
use termion::async_stdin;
use termion::raw::IntoRawMode;
use tokio::io::{AsyncWriteExt, split};
use tokio::spawn;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::codec::{BytesCodec, FramedRead};
use crate::docker_client::DockerError::InvalidJson;
use DockerError::{ErrorResponse, InvalidResponse};

const DOCKER_SOCK: &str = "/var/run/docker.sock";
const DOCKER_API_VERSION: &str = "v1.43";

#[derive(thiserror::Error, Debug)]
pub enum DockerError {
    #[error("Network error")]
    NetworkError(#[from] hyper::Error),
    #[error("Error from docker daemon: [{0}] {1}")]
    ErrorResponse(u16, String),
    #[error("Invalid response from Docker daemon: [{0}] {1}")]
    InvalidResponse(u16, String),
    #[error("Invalid JSON response from Docker daemon: [{0}] {1}")]
    InvalidJson(u16, String, #[source] serde_json::Error),
}

pub struct Bind<'a> {
    host_source: &'a str,
    container_dest: &'a str,
    options: &'a [&'a str],
}

impl<'a> Bind<'a> {
    pub fn new(host_source: &'a str, container_dest: &'a str, options: &'a [&'a str]) -> Self {
        Self {
            host_source,
            container_dest,
            options,
        }
    }
}

pub struct Tmpfs<'a> {
    container_dest: &'a str,
    options: &'a [&'a str],
}

impl<'a> Tmpfs<'a> {
    pub fn new(container_dest: &'a str, options: &'a [&'a str]) -> Self {
        Self {
            container_dest,
            options,
        }
    }
}

/// Creates a Docker container.
pub fn create_container(program: &str,
                        arguments: &[String],
                        network: &str,
                        user: &str,
                        binds: &[Bind],
                        tmpfs: &[Tmpfs],
                        readonly_rootfs: bool,
                        working_dir: &str) -> Result<String, DockerError> {
    let mut entrypoint = arguments.to_vec();
    entrypoint.insert(0, program.to_string());
    let is_a_tty = io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal();
    let (status, maybe_body) = body_request(Method::POST, &format!("/{DOCKER_API_VERSION}/containers/create"),
                                            json!({
                                  "Image": "empty",
                                  "Entrypoint": entrypoint,
                                  "User": user,
                                  "AttachStdin": true,
                                  "AttachStdout": true,
                                  "AttachStderr": true,
                                  "Tty": is_a_tty,
                                  "WorkingDir": working_dir,
                                  "HostConfig": {
                                      "NetworkMode": network,
                                      "Binds": binds.into_iter().map(|bind| format!("{}:{}{}",
                                                                 bind.host_source,
                                                                 bind.container_dest,
                                                                 if bind.options.len() > 0 {
                                                                    format!(":{}", bind.options.join(","))
                                                                 } else {
                                                                    "".to_string()
                                                                 }))
                                                    .collect::<Vec<String>>(),
                                      "ReadonlyRootfs": readonly_rootfs,
                                      "Tmpfs": tmpfs.into_iter().map(|tmp| (tmp.container_dest.to_string(), tmp.options.join(",")))
                                                    .collect::<HashMap<String, String>>(),
                                  },
                              }))?;
    let body = maybe_body.ok_or(InvalidResponse(status.as_u16(), "".to_string()))?;
    if status == StatusCode::CREATED {
        let id = body["Id"].as_str().ok_or(InvalidResponse(status.as_u16(), body.to_string()))?;
        Ok(id.to_string())
    } else {
        Err(make_error_response(status, body, "Container creation failed"))
    }
}

/// Starts a Docker container.
pub fn start_container(id: &str) -> Result<(), DockerError> {
    let (status, maybe_body) = empty_request(Method::POST, &format!("/{DOCKER_API_VERSION}/containers/{id}/start"))?;
    if status.is_success() {
        Ok(())
    } else {
        let body = maybe_body.ok_or(InvalidResponse(status.as_u16(), "".to_string()))?;
        Err(make_error_response(status, body, "Container start failed"))
    }
}

/// Attach to a Docker container and stream the output.
pub fn attach_container(id: &str) -> JoinHandle<()> {
    let method = Method::POST;
    let url = &format!("/{DOCKER_API_VERSION}/containers/{id}/attach?logs=true&stream=true&stdin=true&stdout=true&stderr=true");
    let req = Request::builder()
        .uri::<Uri>(Uri::new(DOCKER_SOCK, url))
        .method(method)
        .header("Upgrade", "tcp")
        .header("Connection", "Upgrade")
        .body(Body::empty())
        .expect("failed to build request");
    spawn(upgrade_request(req))
}

async fn upgrade_request(req: Request<Body>) {
    let client = Client::unix();
    let response = client.request(req).await.expect("Unable to make attach request");
    if response.status().is_informational() {
        let content_type = response.headers().get("content-type").unwrap().as_bytes();
        let multiplexed: bool;
        if content_type == b"application/vnd.docker.multiplexed-stream" {
            multiplexed = true;
        } else if content_type == b"application/vnd.docker.raw-stream" {
            multiplexed = false;
        } else {
            panic!("Unrecognized content-type from attach: {}", String::from_utf8(content_type.to_vec()).unwrap());
        }
        let upgraded = hyper::upgrade::on(response).await.unwrap();
        if multiplexed {
            handle_multiplexed_stream(upgraded).await;
        } else {
            handle_raw_stream(upgraded).await;
        }
    } else {
        panic!("{}", parse_error_response(response, "Unable to attach").await.unwrap_err());
    }
}

async fn handle_multiplexed_stream(upgraded: Upgraded) {
    // TODO handle multiplexed stream
    let (read, mut write) = split(upgraded);
    let mut read = FramedRead::new(read, BytesCodec::new());

    let mut stdout = io::stdout();

    while let Some(Ok(data)) = read.next().await {
        stdout.write_all(&*data).expect("Error writing to stdout");
        stdout.flush().expect("Error flushing stdout");
    }
}

async fn handle_raw_stream(upgraded: Upgraded) {
    let (read, mut write) = split(upgraded);
    let mut read = FramedRead::new(read, BytesCodec::new());

    spawn(async move {
        let mut stdin = async_stdin().bytes();
        loop {
            if let Some(Ok(byte)) = stdin.next() {
                write.write(&[byte]).await.ok();
            } else {
                sleep(Duration::from_nanos(10)).await;
            }
        }
    });

    let stdout = io::stdout();
    let mut stdout = stdout.into_raw_mode().expect("Cannot set stdout into raw mode"); // set stdout in raw mode so we can do TTY

    // TODO this loop never terminates
    while let Some(Ok(data)) = read.next().await {
        stdout.write_all(&*data).expect("Error writing to stdout");
        stdout.flush().expect("Error flushing stdout");
    }
}

/// Wait for a Docker container.
pub fn wait_container(id: &str) -> Result<u8, DockerError> {
    let (status, maybe_body) = empty_request(Method::POST, &format!("/{DOCKER_API_VERSION}/containers/{id}/wait"))?;
    let body = maybe_body.ok_or(InvalidResponse(status.as_u16(), "".to_string()))?;
    if status.is_success() {
        let status_code = body["StatusCode"].as_u64().ok_or(InvalidResponse(status.as_u16(), body.to_string()))?;
        Ok(status_code.try_into().expect(&format!("container status code >255: {}", status_code)))
    } else {
        Err(make_error_response(status, body, "Container wait failed"))
    }
}

/// Make a request to the Docker daemon without a body.
fn empty_request(method: Method, url: &str) -> Result<(StatusCode, Option<Value>), DockerError> {
    let req = Request::builder()
        .uri::<Uri>(Uri::new(DOCKER_SOCK, url))
        .header("Accept", "application/json")
        .method(method)
        .body(Body::empty())
        .expect("failed to build request");

    make_request(req)
}

/// Make a request to the Docker daemon with a body.
fn body_request(method: Method, url: &str, body: Value) -> Result<(StatusCode, Option<Value>), DockerError> {
    let req = Request::builder()
        .uri::<Uri>(Uri::new(DOCKER_SOCK, url))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .method(method)
        .body(Body::from(serde_json::to_vec(&body).expect("JSON serialize")))
        .expect("failed to build request");

    make_request(req)
}

fn make_request(req: Request<Body>) -> Result<(StatusCode, Option<Value>), DockerError> {
    let client = Client::unix();
    let response = block_on(
        client.request(req)
            .and_then(|response| {
                let status_code = response.status();
                hyper::body::to_bytes(response.into_body())
                    .map(move |body| {
                        let body = body?.to_vec();
                        Ok((status_code, body))
                    })
            }
            ));

    match response {
        Ok((status_code, body)) => {
            let raw_body: &[u8] = &body.to_vec();
            let json = if raw_body.len() > 0 {
                Some(serde_json::from_slice(raw_body).map_err(|err|
                    InvalidJson(status_code.into(), String::from_utf8(body).unwrap_or(String::from("")), err)
                )?)
            } else {
                None
            };
            Ok((status_code, json))
        }
        Err(e) => Err(e.into())
    }
}

async fn parse_error_response(response: Response<Body>, fallback_error_message: &str) -> Result<(), DockerError> {
    let status = response.status();
    let body = hyper::body::to_bytes(response.into_body()).await?;
    let raw_body = body.to_vec();
    let json = serde_json::from_slice(&raw_body).map_err(|err|
        InvalidJson(status.into(), String::from_utf8(raw_body).unwrap_or(String::from("")), err)
    )?;
    Err(make_error_response(status, json, fallback_error_message))
}

fn make_error_response(status: StatusCode, body: Value, fallback_error_message: &str) -> DockerError {
    ErrorResponse(status.as_u16(), body["message"].as_str().unwrap_or(fallback_error_message).to_string())
}
