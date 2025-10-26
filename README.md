# Various tools for sandboxing programs in Linux

* These tools are made for Linux and will most likely not work on other operating systems.

## contained

Runs a program in a Podman container.

### Prerequisites

* Requires Podman to be installed and setup for rootless and daemonless operation.

### Setup

Build an empty image with:
```shell
buildah build -t empty .
```

## run-image

Convenience tools to run a Podman/Docker/OCI image with Podman, alternative to `podman run`.

### Prerequisites

* Requires Podman to be installed and setup for rootless and daemonless operation.
