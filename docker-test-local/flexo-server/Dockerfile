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
    FLEXO_MIRROR_SELECTION_METHOD="auto" \
    FLEXO_MIRRORS_PREDEFINED=[] \
    FLEXO_MIRRORS_BLACKLIST=[] \
    FLEXO_MIRRORS_AUTO_HTTPS_REQUIRED=false \
    FLEXO_MIRRORS_AUTO_IPV4=true \
    FLEXO_MIRRORS_AUTO_IPV6=true \
    FLEXO_MIRRORS_AUTO_MAX_SCORE=2.5 \
    FLEXO_MIRRORS_AUTO_NUM_MIRRORS=8 \
    FLEXO_MIRRORS_AUTO_MIRRORS_RANDOM_OR_SORT="sort" \
    FLEXO_MIRRORS_AUTO_TIMEOUT=350 \
    FLEXO_MIRRORS_AUTO_LOW_SPEED_LIMIT=102400 \
    FLEXO_MIRRORS_AUTO_LOW_SPEED_TIME_SECS=1

ENV RUST_BACKTRACE="full" \
    RUST_LOG="debug"

# Fetch the json content from a local file instead of from a remote HTTPS endpoint.
# With a hand-crafted JSON file, this allows us to specify which mirrors can be selected by flexo and brings us
# greater flexibility for our test cases.
ENV FLEXO_MIRRORS_AUTO_MIRRORS_STATUS_JSON_ENDPOINT="file:///root/mirrors.json"
COPY mirrors.json /root

COPY flexo /usr/bin/flexo

ENTRYPOINT /usr/bin/flexo
