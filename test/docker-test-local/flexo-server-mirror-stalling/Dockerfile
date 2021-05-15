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

RUN mkdir /etc/flexo

ENV FLEXO_CACHE_DIRECTORY="/tmp/var/cache/flexo/pkg" \
    FLEXO_MIRRORLIST_FALLBACK_FILE="/tmp/var/cache/flexo/state/mirrorlist" \
    FLEXO_MIRRORLIST_LATENCY_TEST_RESULTS_FILE="/tmp/var/cache/flexo/state/latency_test_results.json" \
    FLEXO_PORT=7878 \
    FLEXO_LISTEN_IP_ADDRESS="0.0.0.0" \
    FLEXO_MIRROR_SELECTION_METHOD="predefined" \
    FLEXO_MIRRORS_PREDEFINED="['http://mirror-stalling-mock', 'http://mirror-fast-mock']" \
    FLEXO_MIRRORS_BLACKLIST=[] \
    FLEXO_LOW_SPEED_TIME_SECS=1 \
    FLEXO_LOW_SPEED_LIMIT=1048576

ENV RUST_BACKTRACE="full" \
    RUST_LOG="debug"

COPY --from=build /tmp/build_output/flexo /usr/bin/flexo

COPY start_flexo.sh /usr/bin

ENTRYPOINT /usr/bin/start_flexo.sh
