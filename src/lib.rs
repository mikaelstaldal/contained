//! # Contained
//!
//! Run a program in a Docker container.

use anyhow::Context;
use hyper::{Method, StatusCode};
use serde_json::json;
use docker_client::DockerError::{self, ErrorResponse, InvalidResponse};

pub fn run(program: String, arguments: &[String]) -> Result<String, anyhow::Error> {
    let id = create_container(program, arguments).context("Unable to create container")?;
    start_container(&id).context("Unable to start container")?;
    Ok(id)
}

/// Creates a Docker container.
fn create_container(program: String, arguments: &[String]) -> Result<String, DockerError> {
    let mut entrypoint = arguments.to_vec();
    entrypoint.insert(0, program);
    let (status, body) = docker_client::body_request(Method::POST, "/containers/create",
                                             json!({
                                  "Image": "empty",
                                  "Entrypoint": entrypoint,
                                  "HostConfig": {
                                      "NetworkMode": "none"
                                  },
                              }))?;
    if status == StatusCode::CREATED {
        let id = body["Id"].as_str().ok_or(InvalidResponse(status.as_u16(), body.to_string()))?;
        Ok(id.to_string())
    } else {
        Err(ErrorResponse(status.as_u16(), body["message"].as_str().unwrap_or("Container creation failed").to_string()).into())
    }
}

/// Starts a Docker container.
fn start_container(id: &str) -> Result<(), DockerError> {
    let (status, body) = docker_client::empty_request(Method::POST, &format!("/containers/{id}/start"))?;
    if status.is_success() {
        Ok(())
    } else {
        Err(ErrorResponse(status.as_u16(), body["message"].as_str().unwrap_or("Container start failed").to_string()).into())
    }
}

mod docker_client;
