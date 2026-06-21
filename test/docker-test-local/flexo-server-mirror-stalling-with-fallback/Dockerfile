# syntax=docker/dockerfile:experimental
FROM debian:bookworm-slim

EXPOSE 7878

RUN apt-get update && \
    apt-get install -y curl

RUN mkdir /etc/flexo

# Counterpart to flexo-server-mirror-stalling-no-fallback: same primary mirror
# (mirror-stalling-after-header-mock, which sends the response header then never sends the body)
# and the same growing-file stall detection (FLEXO_GROWING_FILE_STALL_TIMEOUT_SECS=3), but with
# mirror-fast-mock as a fallback. This verifies that fallover to the fast mirror works and the
# client receives a successful HTTP 200 response.
#
# FLEXO_LOW_SPEED_TIME_SECS / FLEXO_LOW_SPEED_LIMIT are required for fallback to actually
# trigger. serve_from_growing_file sends the HTTP 200 header to the client before streaming the
# body. When GROWING_FILE_STALL_TIMEOUT_SECS fires it merely closes the client connection —
# it does not signal the download thread, so curl keeps waiting on the stalled mirror and
# try_until_success never reaches the fallback. Only a LOW_SPEED timeout makes curl abort the
# blocked transfer early enough for the retry to start writing data before the client-side stall
# fires.
ENV FLEXO_CACHE_DIRECTORY="/tmp/var/cache/flexo/pkg" \
    FLEXO_MIRRORLIST_FALLBACK_FILE="/tmp/var/cache/flexo/state/mirrorlist" \
    FLEXO_MIRRORLIST_LATENCY_TEST_RESULTS_FILE="/tmp/var/cache/flexo/state/latency_test_results.json" \
    FLEXO_PORT=7878 \
    FLEXO_LISTEN_IP_ADDRESS="0.0.0.0" \
    FLEXO_MIRROR_SELECTION_METHOD="predefined" \
    FLEXO_CONNECT_TIMEOUT=5000 \
    FLEXO_MIRRORS_PREDEFINED="['http://mirror-stalling-after-header-mock', 'http://mirror-fast-mock']" \
    FLEXO_MIRRORS_BLACKLIST=[]

ENV RUST_BACKTRACE="full" \
    RUST_LOG="debug"

COPY --from=flexo-base-integration-test /tmp/build_output/flexo /usr/bin/flexo

COPY start_flexo.sh /usr/bin

ENTRYPOINT /usr/bin/start_flexo.sh
