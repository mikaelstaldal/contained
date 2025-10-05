# Run a program in a Docker container for sandboxing

## Prerequisites

* This program is made for Linux and will probably not work on other operating systems.
* Requires a Docker compatible daemon, either Docker itself or Podman (but does not use the `docker` command).

## Setup

### Docker

Build an empty image with:
```shell
docker build -t empty .
```

### Podman

Build an empty image with:
```shell
buildah build -t empty .
```

Setup Docker compatible API service and set `DOCKER_HOST` environment variable as described 
[here](https://github.com/containers/podman/blob/main/docs/tutorials/socket_activation.md#socket-activation-of-the-api-service).
