# Run a program in a Docker container for sandboxing

## Prerequisites

* This program is made for Linux, and will probably not work on other operating systems.
* Requires a Docker daemon (but does not use the `docker` command).

## Setup

Build an empty Docker image with:
```shell
docker build -t empty .
```
