# syntax=docker/dockerfile:experimental

FROM rust:1.46.0-buster as build

WORKDIR /tmp

RUN mkdir /tmp/build_output

COPY flexo.tar.gz /tmp

RUN tar xf flexo.tar.gz

WORKDIR /tmp/flexo

RUN --mount=type=cache,target=/usr/local/cargo,from=rust:1.46.0-buster,source=/usr/local/cargo \
    --mount=type=cache,target=/tmp/flexo/target \
    cargo build --release && \
    cp /tmp/flexo/target/release/flexo /tmp/build_output/

FROM debian:buster-slim

EXPOSE 7878

RUN apt-get update && \
    apt-get install -y curl

RUN mkdir -p /var/cache/flexo/pkg && \
    mkdir /var/cache/flexo/state && \
    mkdir /etc/flexo && \
    mkdir -p /var/cache/flexo/pkg/community/os/x86_64 && \
    mkdir -p /var/cache/flexo/pkg/community-staging/os/x86_64 && \
    mkdir -p /var/cache/flexo/pkg/community-testing/os/x86_64 && \
    mkdir -p /var/cache/flexo/pkg/core/os/x86_64 && \
    mkdir -p /var/cache/flexo/pkg/extra/os/x86_64 && \
    mkdir -p /var/cache/flexo/pkg/gnome-unstable/os/x86_64 && \
    mkdir -p /var/cache/flexo/pkg/kde-unstable/os/x86_64 && \
    mkdir -p /var/cache/flexo/pkg/multilib/os/x86_64 && \
    mkdir -p /var/cache/flexo/pkg/multilib-testing/os/x86_64 && \
    mkdir -p /var/cache/flexo/pkg/staging/os/x86_64 && \
    mkdir -p /var/cache/flexo/pkg/testing/os/x86_64

ENV FLEXO_CACHE_DIRECTORY="/var/cache/flexo/pkg" \
    FLEXO_MIRRORLIST_FALLBACK_FILE="/var/cache/flexo/state/mirrorlist" \
    FLEXO_PORT=7878 \
    FLEXO_MIRROR_SELECTION_METHOD="auto" \
    FLEXO_MIRRORS_PREDEFINED=[] \
    FLEXO_MIRRORS_BLACKLIST=[] \
    FLEXO_MIRRORS_AUTO_HTTPS_REQUIRED=false \
    FLEXO_MIRRORS_AUTO_IPV4=true \
    FLEXO_MIRRORS_AUTO_IPV6=true \
    FLEXO_MIRRORS_AUTO_MAX_SCORE=2.5 \
    FLEXO_MIRRORS_AUTO_NUM_MIRRORS=8 \
    FLEXO_MIRRORS_AUTO_MIRRORS_RANDOM_OR_SORT="sort" \
    FLEXO_MIRRORS_AUTO_TIMEOUT=350

ENV RUST_BACKTRACE="full" \
    RUST_LOG="debug"

# Fetch the json content from a local file instead of from a remote HTTPS endpoint.
# With a hand-crafted JSON file, this allows us to specify which mirrors can be selected by flexo and brings us
# greater flexibility for our test cases.
ENV FLEXO_MIRRORS_AUTO_MIRRORS_STATUS_JSON_ENDPOINT="file:///root/mirrors.json"
COPY mirrors.json /root

COPY --from=build /tmp/build_output/flexo /usr/bin/flexo

ENTRYPOINT /usr/bin/flexo
