# syntax=docker/dockerfile:experimental

FROM rust:1.54.0-buster as build

WORKDIR /tmp

RUN mkdir /tmp/build_output

COPY flexo.tar.gz /tmp

RUN tar xf flexo.tar.gz

WORKDIR /tmp/flexo

RUN --mount=type=cache,target=/usr/local/cargo,from=rust:1.54.0-buster,source=/usr/local/cargo \
    --mount=type=cache,target=/tmp/flexo/target \
    cargo build --release && \
    cp /tmp/flexo/target/release/flexo /tmp/build_output/
