FROM curlimages/curl:7.70.0

USER root

RUN apk add --no-cache openssl ca-certificates

RUN ln -s /lib/ld-musl-x86_64.so.1  /lib/ld64.so.1

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
    FLEXO_MIRROR_SELECTION_METHOD="predefined" \
    FLEXO_MIRRORS_PREDEFINED="['http://mirror-low-bandwidth-mock', 'http://mirror-fast-mock']" \
    FLEXO_MIRRORS_BLACKLIST=[] \
    FLEXO_LOW_SPEED_TIME_SECS=1 \
    FLEXO_LOW_SPEED_LIMIT=1048576

ENV RUST_BACKTRACE="full" \
    RUST_LOG="debug"

COPY flexo /usr/bin/flexo

ENTRYPOINT /usr/bin/flexo
