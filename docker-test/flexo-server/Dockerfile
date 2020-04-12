FROM curlimages/curl

USER root

EXPOSE 7878

RUN apk add --no-cache openssl ca-certificates

RUN ln -s /lib/ld-musl-x86_64.so.1  /lib/ld64.so.1

RUN mkdir -p /var/cache/flexo && \
    mkdir /etc/flexo && \
    mkdir -p /var/cache/flexo/community/os/x86_64 && \
    mkdir -p /var/cache/flexo/community-staging/os/x86_64 && \
    mkdir -p /var/cache/flexo/community-testing/os/x86_64 && \
    mkdir -p /var/cache/flexo/core/os/x86_64 && \
    mkdir -p /var/cache/flexo/extra/os/x86_64 && \
    mkdir -p /var/cache/flexo/gnome-unstable/os/x86_64 && \
    mkdir -p /var/cache/flexo/kde-unstable/os/x86_64 && \
    mkdir -p /var/cache/flexo/multilib/os/x86_64 && \
    mkdir -p /var/cache/flexo/multilib-testing/os/x86_64 && \
    mkdir -p /var/cache/flexo/staging/os/x86_64 && \
    mkdir -p /var/cache/flexo/testing/os/x86_64

ENV FLEXO_CACHE_DIRECTORY="/var/cache/flexo" \
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
    FLEXO_MIRRORS_AUTO_TIMEOUT=350

COPY flexo /usr/bin/flexo

ENTRYPOINT /usr/bin/flexo
