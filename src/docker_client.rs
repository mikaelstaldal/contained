//! # Docker Client
//!
//! `docker_client` contains functions to call the Docker daemon.

use std::collections::HashMap;
use futures::{FutureExt, TryFutureExt};
use hyper::{Body, Client, Method, Request, StatusCode};
use hyperlocal::{UnixClientExt, Uri};
use serde_json::json;
use serde_json::Value;
use tokio::runtime::Runtime;
use crate::docker_client::DockerError::InvalidJson;
use DockerError::{ErrorResponse, InvalidResponse};

const DOCKER_SOCK: &str = "/var/run/docker.sock";

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
                        binds: &[Bind],
                        network: &str,
                        readonly_rootfs: bool,
                        tmpfs: &[Tmpfs]) -> Result<String, DockerError> {
    let mut entrypoint = arguments.to_vec();
    entrypoint.insert(0, program.to_string());
    let (status, maybe_body) = body_request(Method::POST, "/containers/create",
                                            json!({
                                  "Image": "empty",
                                  "Entrypoint": entrypoint,
                                  "AttachStdout": true,
                                  "AttachStderr": true,
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
    let (status, maybe_body) = empty_request(Method::POST, &format!("/containers/{id}/start"))?;
    if status.is_success() {
        Ok(())
    } else {
        let body = maybe_body.ok_or(InvalidResponse(status.as_u16(), "".to_string()))?;
        Err(make_error_response(status, body, "Container start failed"))
    }
}

fn make_error_response(status: StatusCode, body: Value, fallback_error_message: &str) -> DockerError {
    ErrorResponse(status.as_u16(), body["message"].as_str().unwrap_or(fallback_error_message).to_string())
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
    let runtime = Runtime::new().unwrap();
    let response = runtime.block_on(
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
