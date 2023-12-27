//! # Docker Client
//!
//! `docker_client` contains functions to call the Docker daemon.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use atoi::atoi;
use http::{header, Method, Request, StatusCode};
use httparse::Status::{Complete, Partial};
use serde_json::json;
use serde_json::Value;

use crate::docker_client::DockerError::{ErrorResponse, HttpError, InvalidJson, InvalidResponse};

const DOCKER_SOCK: &str = "/var/run/docker.sock";
const APPLICATION_JSON: &str = "application/json";

#[derive(thiserror::Error, Debug)]
pub enum DockerError {
    #[error("Network error")]
    NetworkError(#[from] io::Error),
    #[error("HTTP error")]
    HttpError(#[from] httparse::Error),
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
    let (status, maybe_body) = body_request(Method::POST, "/containers/create",
                                            json!({
                                  "Image": "empty",
                                  "Entrypoint": entrypoint,
                                  "User": user,
                                  "AttachStdout": true,
                                  "AttachStderr": true,
                                  "Tty": true,
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
    match maybe_body {
        Some(body) if status == StatusCode::CREATED => {
            let id = body["Id"].as_str().ok_or(InvalidResponse(status.as_u16(), body.to_string()))?;
            Ok(id.to_string())
        }
        Some(body) => Err(make_error_response(status, body, "Container creation failed")),
        _ => Err(InvalidResponse(status.as_u16(), "".to_string()))
    }
}

/// Starts a Docker container.
pub fn start_container(id: &str) -> Result<(), DockerError> {
    let (status, maybe_body) = empty_request(Method::POST, &format!("/containers/{id}/start"))?;
    if status.is_success() {
        Ok(())
    } else {
        match maybe_body {
            Some(body) => Err(make_error_response(status, body, "Container start failed")),
            _ => Err(InvalidResponse(status.as_u16(), "".to_string()))
        }
    }
}
/*
/// Attach to a Docker container and stream the output.
pub fn attach_container(id: &str) {
    let method = Method::POST;
    let url = &format!("/containers/{id}/attach?logs=true&stream=true&stdout=true&stderr=true");
    let req = Request::builder()
        .uri(url)
        .method(method)
        .body(())
        .expect("failed to build request");
    // spawn(streaming_request(req));
}

fn streaming_request(req: Request<Body>) {
    let client = Client::unix();
    let mut response = client.request(req).await.expect("Unable to make attach request");
    if response.status().is_success() || response.status().is_informational() {
        handle_stream(&mut response).await;
    } else {
        panic!("{}", parse_error_response(response, "Unable to attach").await.unwrap_err());
    }
}

fn handle_stream(response: &mut Response<Body>) {
    while let Some(next) = response.data().await {
        let chunk = next.expect("Error reading from container");
        io::stdout().write_all(&chunk).expect("Error writing to stdout");
        io::stdout().flush().expect("Error flushing stdout");
    }
}

/// Wait for a Docker container.
pub fn wait_container(id: &str) -> Result<u8, DockerError> {
    let (status, maybe_body) = empty_request(Method::POST, &format!("/containers/{id}/wait"))?;
    let body = maybe_body.ok_or(InvalidResponse(status.as_u16(), "".to_string()))?;
    if status.is_success() {
        let status_code = body["StatusCode"].as_u64().ok_or(InvalidResponse(status.as_u16(), body.to_string()))?;
        Ok(status_code.try_into().expect(&format!("container status code >255: {}", status_code)))
    } else {
        Err(make_error_response(status, body, "Container wait failed"))
    }
}

*/

/// Make a request to the Docker daemon without a body.
fn empty_request(method: Method, url: &str) -> Result<(StatusCode, Option<Value>), DockerError> {
    let req = Request::builder()
        .method(method)
        .uri(url)
        .header(header::HOST, "localhost")
        .header(header::ACCEPT, APPLICATION_JSON)
        .body(Vec::new())
        .expect("failed to build request");

    make_request(req)
}

/// Make a request to the Docker daemon with a body.
fn body_request(method: Method, url: &str, body: Value) -> Result<(StatusCode, Option<Value>), DockerError> {
    let raw_body = serde_json::to_vec(&body).expect("JSON serialize");
    let req = Request::builder()
        .method(method)
        .uri(url)
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, APPLICATION_JSON)
        .header(header::CONTENT_LENGTH, raw_body.len().to_string())
        .header(header::ACCEPT, APPLICATION_JSON)
        .body(raw_body)
        .expect("failed to build request");

    make_request(req)
}

fn make_request(req: Request<Vec<u8>>) -> Result<(StatusCode, Option<Value>), DockerError> {
    let mut stream = UnixStream::connect(DOCKER_SOCK)?;

    stream.write_all(&*format!("{} {} HTTP/1.1\r\n", req.method().as_str(), req.uri().to_string()).into_bytes())?;
    for (name, value) in req.headers() {
        stream.write_all(name.as_str().as_bytes())?;
        stream.write_all(": ".as_bytes())?;
        stream.write_all(value.as_bytes())?;
        stream.write_all("\r\n".as_bytes())?;
    }
    stream.write_all("\r\n".as_bytes())?;
    if req.body().len() > 0 {
        stream.write_all(req.body())?;
    }
    stream.flush()?;

    let mut buffer = [0; 1024];
    let mut bytes_read: usize = 0;
    loop {
        bytes_read += stream.read(&mut buffer[bytes_read..])?;
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut response = httparse::Response::new(&mut headers);
        match response.parse(&buffer)? {
            Complete(header_size) => {
                let status_code: StatusCode = StatusCode::from_u16(response.code.ok_or(HttpError(httparse::Error::Status))?)
                    .map_err(|_| HttpError(httparse::Error::Status))?;
                let content_type = response.headers.into_iter()
                    .find(|h| h.name.eq_ignore_ascii_case(header::CONTENT_TYPE.as_str()))
                    .map(|h| h.value)
                    .unwrap_or(&[]);
                let content_length = match response.headers.into_iter()
                    .find(|h| h.name.eq_ignore_ascii_case(header::CONTENT_LENGTH.as_str()))
                    .map(|h| h.value) {
                    Some(v) => atoi::<usize>(v).ok_or(HttpError(httparse::Error::HeaderValue))?,
                    None => 0
                };

                return if content_length > 0 {
                    let body = if content_length > (bytes_read - header_size) {
                        let mut body_buffer = Vec::from(&buffer[header_size..bytes_read]);
                        body_buffer.resize(content_length, 0);
                        stream.read_exact(&mut body_buffer[(bytes_read - header_size)..])?;
                        body_buffer
                    } else {
                        (&buffer[header_size..(header_size + content_length)]).to_vec()
                    };

                    if content_type.eq_ignore_ascii_case(APPLICATION_JSON.as_bytes()) {
                        let json = Some(serde_json::from_slice(&*body).map_err(|err|
                            InvalidJson(status_code.into(), String::from_utf8(body).unwrap_or(String::from("")), err)
                        )?);
                        Ok((status_code, json))
                    } else {
                        Err(InvalidResponse(status_code.as_u16(), String::from_utf8(body).unwrap_or(String::from(""))))
                    }
                } else {
                    Ok((status_code, None))
                };
            }
            Partial => {}
        };
    };
}

/*
fn parse_error_response(response: Response<Body>, fallback_error_message: &str) -> Result<(), DockerError> {
    let status = response.status();
    let body = hyper::body::to_bytes(response.into_body()).await?;
    let raw_body = body.to_vec();
    let json = serde_json::from_slice(&raw_body).map_err(|err|
        InvalidJson(status.into(), String::from_utf8(raw_body).unwrap_or(String::from("")), err)
    )?;
    Err(make_error_response(status, json, fallback_error_message))
}
 */

fn make_error_response(status: StatusCode, body: Value, fallback_error_message: &str) -> DockerError {
    ErrorResponse(status.as_u16(), body["message"].as_str().unwrap_or(fallback_error_message).to_string())
}
