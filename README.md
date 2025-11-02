# Various tools for sandboxing programs in Linux

* These tools are made for Linux and will most likely not work on other operating systems.


## contained

Run a program in a container without having to build a specific image for it, without using a daemon.

### Prerequisites

Requires [Podman](https://podman.io/) to be installed and setup for rootless and daemonless operation,
and the `podman` command to be in `PATH`.

### Setup

Build an empty image with:
```shell
buildah build -t empty .
```

## run-image

Convenience tools to run a Podman/Docker/OCI image with Podman without using a daemon, 
alternative to `podman run`.

### Prerequisites

Requires [Podman](https://podman.io/) to be installed and setup for rootless and daemonless operation,
and the `podman` command to be in `PATH`.


## contained-d

Run a program in a container without having to build a specific image for it, via Docker daemon.

### Prerequisites

Requires Docker daemon, or a compatible daemon (e.g. Podman API service), set `DOCKER_HOST` environment variable 
unless using standard Docker daemon.

### Setup

Build an empty image with:
```shell
docker build -t empty .
```

### run-image-d

Convenience tools to run a Podman/Docker/OCI image with Podman, via Docker daemon,
alternative to `docker run`.

### Prerequisites

Requires Docker daemon, or a compatible daemon (e.g. Podman API service), set `DOCKER_HOST` environment variable 
unless using standard Docker daemon.
