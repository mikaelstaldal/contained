//! # Docker Client
//!
//! `docker_client` contains functions to call the Docker daemon.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::io::ErrorKind::UnexpectedEof;
use std::os::unix::net::UnixStream;
use std::thread;

use atoi::{FromRadix10Checked, FromRadix16Checked};
use byteorder::{BigEndian, ByteOrder};
use http::{header, HeaderName, Method, Request, StatusCode};
use httparse::Response;
use httparse::Status::Complete;
use serde_json::json;
use serde_json::Value;

use StreamType::{Stderr, Stdin, Stdout};

use crate::docker_client::DockerError::{ErrorResponse, HttpError, InvalidJson, InvalidResponse, InvalidStream, NetworkError};

const DOCKER_SOCK: &str = "/var/run/docker.sock";
const APPLICATION_JSON: &str = "application/json";
const BUFFER_SIZE: usize = 1024;

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
    #[error("Invalid stream: {0}")]
    InvalidStream(u8),
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

pub struct Tty {
    height: u16,
    width: u16,
}

impl Tty {
    pub fn new(height: u16, width: u16) -> Self {
        Self {
            height,
            width,
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
                        working_dir: &str,
                        tty: &Option<Tty>) -> Result<String, DockerError> {
    let mut entrypoint = arguments.to_vec();
    entrypoint.insert(0, program.to_string());
    let (status, maybe_body) = body_request(Method::POST, "/containers/create",
                                            json!({
                                  "Image": "empty",
                                  "Entrypoint": entrypoint,
                                  "User": user,
                                  "AttachStdin": true,
                                  "AttachStdout": true,
                                  "AttachStderr": true,
                                  "OpenStdin": true,
                                  "Tty": tty.is_some(),
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
                                      "ConsoleSize": tty.as_ref().map(|t| [t.height, t.width])
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

/// Waits for a Docker container to exit, and return the exit code.
pub fn wait_container(id: &str) -> Result<u8, DockerError> {
    let (status, maybe_body) = empty_request(Method::POST, &format!("/containers/{id}/wait?condition=next-exit"))?;
    match maybe_body {
        Some(body) if status.is_success() => {
            let status_code = body["StatusCode"].as_u64().ok_or(InvalidResponse(status.as_u16(), body.to_string()))?;
            Ok(status_code.try_into().map_err(|_| InvalidResponse(status.as_u16(), format!("container status code >255: {}", status_code)))?)
        }
        Some(body) => Err(make_error_response(status, body, "Container wait failed")),
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

/// Attach to a Docker container and stream the output.
pub fn attach_container(id: &str) -> Result<(), DockerError> {
    let req = Request::builder()
        .method(Method::POST)
        .uri(&format!("/containers/{id}/attach?stream=true&stdin=true&stdout=true&stderr=true"))
        .header(header::HOST, "localhost")
        .header(header::UPGRADE, "tcp")
        .header(header::CONNECTION, "Upgrade")
        .body(None)
        .expect("failed to build request");

    let mut stream = UnixStream::connect(DOCKER_SOCK)?;

    send_request(req, &mut stream)?;
    let (buffer, bytes_read, header_size, stream, is_multiplexed) = read_response(stream)?;

    let write_stream = stream.try_clone()?;

    if is_multiplexed {
        handle_multiplexed(buffer, bytes_read, header_size, stream, write_stream)?;
    } else {
        handle_raw(buffer, bytes_read, header_size, stream, write_stream)?;
    }

    Ok(())
}

fn read_response(mut stream: UnixStream) -> Result<([u8; BUFFER_SIZE], usize, usize, UnixStream, bool), DockerError> {
    let mut buffer = [0; BUFFER_SIZE];
    let mut bytes_read: usize = 0;
    loop {
        bytes_read += stream.read(&mut buffer[bytes_read..])?;
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut response = Response::new(&mut headers);
        if let Complete(header_size) = response.parse(&buffer)? {
            let status_code: StatusCode = StatusCode::from_u16(response.code.ok_or(HttpError(httparse::Error::Status))?)
                .map_err(|_| HttpError(httparse::Error::Status))?;
            if !status_code.is_informational() {
                return Err(InvalidResponse(status_code.as_u16(), response.reason.map_or("".to_string(), |s| s.to_string())));
            }

            let content_type = get_header_value(&mut response, header::CONTENT_TYPE).unwrap_or(&[]).to_vec();
            let is_multiplexed = if content_type == b"application/vnd.docker.multiplexed-stream" {
                true
            } else if content_type == b"application/vnd.docker.raw-stream" {
                false
            } else {
                return Err(InvalidResponse(status_code.as_u16(),
                                           format!("Unrecognized content-type from attach: {}",
                                                   String::from_utf8(content_type).expect("UTF-8"))));
            };
            return Ok((buffer, bytes_read, header_size, stream, is_multiplexed));
        }
    };
}

fn handle_raw(buffer: [u8; BUFFER_SIZE], bytes_read: usize, header_size: usize, stream: UnixStream, write_stream: UnixStream) -> Result<(), DockerError> {
    thread::Builder::new().name("read".to_string()).spawn(move || {
        read_raw_data(buffer, header_size, bytes_read, stream).unwrap();
    })?;
    thread::Builder::new().name("write".to_string()).spawn(move || {
        write_data(write_stream).unwrap();
    })?;
    Ok(())
}

fn read_raw_data(mut buffer: [u8; BUFFER_SIZE], header_size: usize, bytes_read: usize, mut stream: UnixStream) -> Result<(), DockerError> {
    let mut stdout = io::stdout();

    stdout.write_all(&buffer[header_size..bytes_read])?;
    stdout.flush()?;

    copy_stream(&mut stream, &mut stdout, &mut buffer)
}

fn write_data(mut stream: UnixStream) -> Result<(), DockerError> {
    let mut buffer = [0; BUFFER_SIZE];
    copy_stream(&mut io::stdin(), &mut stream, &mut buffer)
}

fn copy_stream<R, W>(from: &mut R, to: &mut W, buffer: &mut [u8]) -> Result<(), DockerError>
    where
        R: Read,
        W: Write,
{
    let mut bytes_read: usize;
    loop {
        bytes_read = from.read(buffer)?;
        if bytes_read < 1 {
            return Ok(());
        }
        to.write_all(&buffer[..bytes_read])?;
        to.flush()?;
    }
}

fn handle_multiplexed(buffer: [u8; BUFFER_SIZE], bytes_read: usize, header_size: usize, stream: UnixStream, write_stream: UnixStream) -> Result<(), DockerError> {
    thread::Builder::new().name("read".to_string()).spawn(move || {
        read_multiplexed_data(buffer, header_size, bytes_read, stream).unwrap();
    })?;
    thread::Builder::new().name("write".to_string()).spawn(move || {
        write_data(write_stream).unwrap();
    })?;
    Ok(())
}

enum StreamType {
    Stdin,
    Stdout,
    Stderr,
}

fn read_multiplexed_data(buffer: [u8; BUFFER_SIZE], header_size: usize, bytes_read: usize, stream: UnixStream) -> Result<(), DockerError> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    let stream = &mut buffer[header_size..bytes_read].chain(stream);

    loop {
        let mut frame_header = [0; 8];
        match stream.read_exact(&mut frame_header) {
            Ok(_) => {}
            Err(e) if e.kind() == UnexpectedEof => return Ok(()),
            Err(e) => return Err(NetworkError(e))
        }
        let (stream_type, size) = read_frame_header(&frame_header)?;
        let mut frame = vec![0; size as usize];
        stream.read_exact(&mut frame)?;

        match stream_type {
            Stdin | Stdout => {
                stdout.write_all(&frame)?;
                stdout.flush()?;
            }
            Stderr => {
                stderr.write_all(&frame)?;
                stderr.flush()?;
            }
        };
    }
}

fn read_frame_header(buffer: &[u8]) -> Result<(StreamType, u32), DockerError> {
    let stream_type = match buffer[0] {
        0 => Stdin,
        1 => Stdout,
        2 => Stderr,
        _ => return Err(InvalidStream(buffer[0]))
    };
    let size = BigEndian::read_u32(&buffer[4..]);
    Ok((stream_type, size))
}

/// Make a request to the Docker daemon without a body.
fn empty_request(method: Method, url: &str) -> Result<(StatusCode, Option<Value>), DockerError> {
    let req = Request::builder()
        .method(method)
        .uri(url)
        .header(header::HOST, "localhost")
        .header(header::ACCEPT, APPLICATION_JSON)
        .body(None)
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
        .body(Some(raw_body))
        .expect("failed to build request");

    make_request(req)
}

fn make_request(req: Request<Option<Vec<u8>>>) -> Result<(StatusCode, Option<Value>), DockerError> {
    let mut stream = UnixStream::connect(DOCKER_SOCK)?;

    send_request(req, &mut stream)?;

    let mut buffer = [0; BUFFER_SIZE];
    let mut bytes_read: usize = 0;
    loop {
        bytes_read += stream.read(&mut buffer[bytes_read..])?;
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut response = Response::new(&mut headers);
        if let Complete(header_size) = response.parse(&buffer)? {
            let status_code: StatusCode = StatusCode::from_u16(response.code.ok_or(HttpError(httparse::Error::Status))?)
                .map_err(|_| HttpError(httparse::Error::Status))?;
            let content_type = get_header_value(&mut response, header::CONTENT_TYPE).unwrap_or(&[]).to_vec();
            let transfer_encoding = get_header_value(&mut response, header::TRANSFER_ENCODING).unwrap_or(&[]);
            let content_length = match get_header_value(&mut response, header::CONTENT_LENGTH) {
                Some(v) => usize::from_radix_10_checked(v).0.ok_or(HttpError(httparse::Error::HeaderValue))?,
                None => 0
            };

            let body = if content_length > 0 {
                if content_length > (bytes_read - header_size) {
                    stream.read_exact(&mut buffer[bytes_read..(content_length - header_size)])?;
                }
                Some(&buffer[header_size..(header_size + content_length)])
            } else if transfer_encoding.eq_ignore_ascii_case("chunked".as_bytes()) {
                let chunk_bytes_read = stream.read(&mut buffer[bytes_read..])?;

                let mut chunk_size_end: usize = header_size;
                loop {
                    if buffer[chunk_size_end] == b'\r' || chunk_size_end > (bytes_read + chunk_bytes_read) {
                        break;
                    }
                    chunk_size_end += 1;
                }
                let chunk_size = usize::from_radix_16_checked(&buffer[header_size..chunk_size_end]).0.ok_or(HttpError(httparse::Error::Token))?;
                Some(&buffer[(chunk_size_end + 2)..(chunk_size_end + 2 + chunk_size)])
            } else {
                None
            };

            return match body {
                Some(body) => {
                    if content_type.eq_ignore_ascii_case(APPLICATION_JSON.as_bytes()) {
                        let json = Some(serde_json::from_slice(body).map_err(|err|
                            InvalidJson(status_code.into(), String::from_utf8(body.to_vec()).unwrap_or(String::from("")), err)
                        )?);
                        Ok((status_code, json))
                    } else {
                        Err(InvalidResponse(status_code.as_u16(), String::from_utf8(body.to_vec()).unwrap_or(String::from(""))))
                    }
                }
                None => Ok((status_code, None))
            };
        }
    };
}

fn send_request(req: Request<Option<Vec<u8>>>, stream: &mut UnixStream) -> Result<(), DockerError> {
    stream.write_all(&*format!("{} {} HTTP/1.1\r\n", req.method().as_str(), req.uri().to_string()).into_bytes())?;
    for (name, value) in req.headers() {
        stream.write_all(name.as_str().as_bytes())?;
        stream.write_all(": ".as_bytes())?;
        stream.write_all(value.as_bytes())?;
        stream.write_all("\r\n".as_bytes())?;
    }
    stream.write_all("\r\n".as_bytes())?;
    if let Some(body) = req.body() {
        stream.write_all(body)?;
    }
    stream.flush()?;
    Ok(())
}

fn get_header_value<'headers, 'buf>(response: &mut Response<'headers, 'buf>, header_name: HeaderName) -> Option<&'headers [u8]> {
    response.headers.into_iter()
        .find(|h| h.name.eq_ignore_ascii_case(header_name.as_str()))
        .map(|h| h.value)
}

fn make_error_response(status: StatusCode, body: Value, fallback_error_message: &str) -> DockerError {
    ErrorResponse(status.as_u16(), body["message"].as_str().unwrap_or(fallback_error_message).to_string())
}
