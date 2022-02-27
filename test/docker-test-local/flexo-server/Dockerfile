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
    FLEXO_CONNECT_TIMEOUT=3000 \
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

COPY --from=flexo-base-integration-test /tmp/build_output/flexo /usr/bin/flexo

COPY start_flexo.sh /usr/bin

ENTRYPOINT /usr/bin/start_flexo.sh
