//! # Contained
//!
//! Run a program in a Docker container.

use std::error::Error;
use hyper::{Method, StatusCode};
use serde_json::{json, Value};

/// Create a Docker container.
pub fn create_container(program: String, arguments: &[String]) -> Result<(StatusCode, Value), Box<dyn Error>> {
    let mut entrypoint = arguments.to_vec();
    entrypoint.insert(0, program);
    let result = docker_client::body_request(Method::POST, "/containers/create",
                                             json!({
                                  "Image": "empty",
                                  "Entrypoint": entrypoint,
                                  "HostConfig": {
                                      "NetworkMode": "none"
                                  },
                              }))?;
    Ok(result)
}

pub mod docker_client;
