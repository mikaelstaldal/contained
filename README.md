# Run a program in a Podman container for sandboxing

## Prerequisites

* This program is made for Linux and will probably not work on other operating systems.
* Requires Podman to be installed and setup for rootless operation.

## Setup

Build an empty image with:
```shell
buildah build -t empty .
```
