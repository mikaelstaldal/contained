//! # Contained
//!
//! Run a program in a Docker container.

use std::error::Error;
use hyper::{Method, StatusCode};
use serde_json::json;

pub fn run(program: String, arguments: &[String]) -> Result<String, Box<dyn Error>> {
    let id = create_container(program, arguments)?;
    start_container(&id)?;
    Ok(id)
}

/// Creates a Docker container.
fn create_container(program: String, arguments: &[String]) -> Result<String, Box<dyn Error>> {
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
        let id = body["Id"].as_str().expect("Container creation failed");
        Ok(id.to_string())
    } else {
        Err(Box::<dyn Error>::from(body["message"].as_str().unwrap_or(&format!("Container creation failed: {status}"))))
    }
}

/// Starts a Docker container.
fn start_container(id: &str) -> Result<(), Box<dyn Error>> {
    let (status, body) = docker_client::empty_request(Method::POST, &format!("/containers/{id}/start"))?;
    if status.is_success() {
        Ok(())
    } else {
        Err(Box::<dyn Error>::from(body["message"].as_str().unwrap_or(&format!("Container start failed: {status}"))))
    }
}

mod docker_client;
