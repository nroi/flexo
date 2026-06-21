# syntax=docker/dockerfile:experimental
FROM debian:bookworm-slim

EXPOSE 7878

RUN apt-get update && \
    apt-get install -y curl

RUN mkdir /etc/flexo

# Unlike flexo-server-mirror-stalling-after-header, this server has a SINGLE mirror (no fast
# fallback) and no low-speed timeout. The mirror sends the response header (with a Content-Length)
# and then never sends the body, so the cache file never grows and the download never completes or
# fails. This is the precise condition that made serve_from_growing_file loop forever in
# https://github.com/nroi/flexo/issues/111. It is used by the flexo_test_mirror_stalling_no_fallback
# regression test, which asserts that flexo gives up on the stalled transfer instead of hanging.
ENV FLEXO_CACHE_DIRECTORY="/tmp/var/cache/flexo/pkg" \
    FLEXO_MIRRORLIST_FALLBACK_FILE="/tmp/var/cache/flexo/state/mirrorlist" \
    FLEXO_MIRRORLIST_LATENCY_TEST_RESULTS_FILE="/tmp/var/cache/flexo/state/latency_test_results.json" \
    FLEXO_PORT=7878 \
    FLEXO_LISTEN_IP_ADDRESS="0.0.0.0" \
    FLEXO_MIRROR_SELECTION_METHOD="predefined" \
    FLEXO_CONNECT_TIMEOUT=3000 \
    FLEXO_GROWING_FILE_STALL_TIMEOUT_SECS=3 \
    FLEXO_MIRRORS_PREDEFINED="['http://mirror-stalling-after-header-mock']" \
    FLEXO_MIRRORS_BLACKLIST=[]

ENV RUST_BACKTRACE="full" \
    RUST_LOG="debug"

COPY --from=flexo-base-integration-test /tmp/build_output/flexo /usr/bin/flexo

COPY start_flexo.sh /usr/bin

ENTRYPOINT /usr/bin/start_flexo.sh
