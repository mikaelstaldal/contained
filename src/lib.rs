use std::error::Error;
use hyper::{Method, StatusCode};
use serde_json::{json, Value};

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

pub mod docker_client {
    use std::error::Error;
    use futures::{FutureExt, TryFutureExt};
    use hyper::{Body, Client, Method, Request, StatusCode};
    use hyperlocal::{UnixClientExt, Uri};
    use serde_json::Value;
    use tokio::runtime::Runtime;

    const DOCKER_SOCK: &str = "/var/run/docker.sock";

    pub fn empty_request(method: Method, url: &str) -> Result<(StatusCode, Value), Box<dyn Error>> {
        let req = Request::builder()
            .uri::<Uri>(Uri::new(DOCKER_SOCK, url).into())
            .header("Accept", "application/json")
            .method(method)
            .body(Body::empty())
            .expect("failed to build request");

        make_request(req)
    }

    pub fn body_request(method: Method, url: &str, body: Value) -> Result<(StatusCode, Value), Box<dyn Error>> {
        let req = Request::builder()
            .uri::<Uri>(Uri::new(DOCKER_SOCK, url).into())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .method(method)
            .body(Body::from(serde_json::to_vec(&body).expect("JSON serialize")))
            .expect("failed to build request");

        make_request(req)
    }

    fn make_request(req: Request<Body>) -> Result<(StatusCode, Value), Box<dyn Error>> {
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
                let json = serde_json::from_slice(&body.to_vec())?;
                Ok((status_code, json))
            }
            Err(e) => Err(Box::new(e))
        }
    }
}
