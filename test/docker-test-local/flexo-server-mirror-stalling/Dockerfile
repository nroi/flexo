# syntax=docker/dockerfile:experimental
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
    FLEXO_CONNECT_TIMEOUT=3000 \
    FLEXO_MIRRORS_PREDEFINED="['http://mirror-stalling-mock', 'http://mirror-fast-mock']" \
    FLEXO_MIRRORS_BLACKLIST=[] \
    FLEXO_LOW_SPEED_TIME_SECS=1 \
    FLEXO_LOW_SPEED_LIMIT=1048576

ENV RUST_BACKTRACE="full" \
    RUST_LOG="debug"

COPY --from=flexo-base-integration-test /tmp/build_output/flexo /usr/bin/flexo

COPY start_flexo.sh /usr/bin

ENTRYPOINT /usr/bin/start_flexo.sh
