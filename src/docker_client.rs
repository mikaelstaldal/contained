//! # Docker Client
//!
//! `docker_client` contains functions to call the Docker daemon.

use futures::{FutureExt, TryFutureExt};
use hyper::{Body, Client, Method, Request, StatusCode};
use hyperlocal::{UnixClientExt, Uri};
use serde_json::Value;
use tokio::runtime::Runtime;
use crate::docker_client::DockerError::InvalidJson;

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

/// Make a request to the Docker daemon without a body.
pub fn empty_request(method: Method, url: &str) -> Result<(StatusCode, Value), DockerError> {
    let req = Request::builder()
        .uri::<Uri>(Uri::new(DOCKER_SOCK, url).into())
        .header("Accept", "application/json")
        .method(method)
        .body(Body::empty())
        .expect("failed to build request");

    make_request(req)
}

/// Make a request to the Docker daemon with a body.
pub fn body_request(method: Method, url: &str, body: Value) -> Result<(StatusCode, Value), DockerError> {
    let req = Request::builder()
        .uri::<Uri>(Uri::new(DOCKER_SOCK, url).into())
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .method(method)
        .body(Body::from(serde_json::to_vec(&body).expect("JSON serialize")))
        .expect("failed to build request");

    make_request(req)
}

fn make_request(req: Request<Body>) -> Result<(StatusCode, Value), DockerError> {
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
            let json = serde_json::from_slice(&body.to_vec()).map_err(|err|
                InvalidJson(status_code.into(), String::from_utf8(body).unwrap_or(String::from("")), err)
            )?;
            Ok((status_code, json))
        }
        Err(e) => Err(e.into())
    }
}
