FROM rust:1.43.1-buster as build

WORKDIR /tmp

COPY flexo.tar.gz /tmp

RUN tar xf flexo.tar.gz && \
    cd /tmp/flexo && \
    cargo build --release

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
    FLEXO_LOW_SPEED_LIMIT=128000 \
    FLEXO_LOW_SPEED_TIME_SECS=3 \
    FLEXO_MIRRORLIST_FALLBACK_FILE="/var/cache/flexo/state/mirrorlist" \
    FLEXO_PORT=7878 \
    FLEXO_MIRROR_SELECTION_METHOD="auto" \
    FLEXO_MIRRORS_PREDEFINED=[] \
    FLEXO_MIRRORS_BLACKLIST=[] \
    FLEXO_MIRRORS_AUTO_HTTPS_REQUIRED=true \
    FLEXO_MIRRORS_AUTO_IPV4=true \
    FLEXO_MIRRORS_AUTO_IPV6=true \
    FLEXO_MIRRORS_AUTO_MAX_SCORE=2.5 \
    FLEXO_MIRRORS_AUTO_NUM_MIRRORS=8 \
    FLEXO_MIRRORS_AUTO_MIRRORS_RANDOM_OR_SORT="sort" \
    FLEXO_MIRRORS_AUTO_TIMEOUT=350 \
    FLEXO_MIRRORS_AUTO_MAX_SPEED_LIMIT=102400

ENV RUST_BACKTRACE="full" \
    RUST_LOG="debug"

COPY --from=build /tmp/flexo/target/release/flexo /usr/bin/flexo

ENTRYPOINT /usr/bin/flexo
