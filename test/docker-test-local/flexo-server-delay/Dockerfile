# syntax=docker/dockerfile:experimental

FROM rust:1.54.0-buster as build

WORKDIR /tmp

RUN mkdir /tmp/build_output

COPY tcp-proxy-delay.tar.gz /tmp

RUN tar xf tcp-proxy-delay.tar.gz

WORKDIR /tmp/tcp-proxy-delay

RUN --mount=type=cache,target=/usr/local/cargo,from=rust:1.54.0-buster,source=/usr/local/cargo \
    --mount=type=cache,target=/tmp/tcp-proxy-delay/target \
    cargo build --release && \
    cp /tmp/tcp-proxy-delay/target/release/tcp-proxy-delay /tmp/build_output/

FROM debian:buster-slim

EXPOSE 7878

ENV RUST_BACKTRACE="full" \
    RUST_LOG="debug"

ENV TCP_PROXY_DELAY_LISTEN_HOST=0.0.0.0 \
    TCP_PROXY_DELAY_LISTEN_PORT=7878 \
    TCP_PROXY_DELAY_CONNECT_TO_HOST=flexo-server-fast \
    TCP_PROXY_DELAY_CONNECT_TO_PORT=7878 \
    TCP_PROXY_DELAY_MILLISECS=100

COPY --from=build /tmp/build_output/tcp-proxy-delay /usr/bin/tcp-proxy-delay

ENTRYPOINT /usr/bin/tcp-proxy-delay
