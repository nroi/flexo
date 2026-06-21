# syntax=docker/dockerfile:experimental

FROM rust:1.86-slim-bookworm as build

RUN apt-get update && \
    apt-get install -y --no-install-recommends pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /tmp

RUN mkdir /tmp/build_output

COPY flexo.tar.gz /tmp

RUN tar xf flexo.tar.gz

WORKDIR /tmp/flexo

RUN --mount=type=cache,target=/usr/local/cargo,from=rust:1.86-slim-bookworm,source=/usr/local/cargo \
    --mount=type=cache,target=/tmp/flexo/target \
    cargo build --release && \
    cp /tmp/flexo/target/release/flexo /tmp/build_output/
