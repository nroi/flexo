# syntax=docker/dockerfile:experimental

FROM rust:1.54.0-buster as build

WORKDIR /tmp

RUN mkdir /tmp/build_output

COPY integration-test-client.tar.gz /tmp

RUN tar xf integration-test-client.tar.gz

WORKDIR /tmp/integration-test-client

RUN --mount=type=cache,target=/usr/local/cargo,from=rust:1.54.0-buster,source=/usr/local/cargo \
    --mount=type=cache,target=/tmp/integration-test-client/target \
    cargo build --release && \
    cp /tmp/integration-test-client/target/release/integration-test-client /tmp/build_output/

FROM archlinux:20200407

RUN echo 'Server = http://flexo-server:7878/$repo/os/$arch' > /etc/pacman.d/mirrorlist

COPY pkg.tar /tmp

# make db files locally available. This basically has the same effect as running 'pacman -Sy'.
RUN tar -C /tmp -xf /tmp/pkg.tar && \
    cp /tmp/core/os/x86_64/core.db /var/lib/pacman/sync && \
    cp /tmp/extra/os/x86_64/extra.db /var/lib/pacman/sync && \
    cp /tmp/community/os/x86_64/community.db /var/lib/pacman/sync

COPY ./flexo_test_wrap_output /root/flexo_test_wrap_output

COPY packages/libbsd-0.10.0-2-x86_64.pkg.tar.zst /root
COPY packages/openbsd-netcat-1.206_1-1-x86_64.pkg.tar.zst /root

# netcat is required for our test cases. libbsd is installed since it is
# required by netcat.
RUN pacman -U --noconfirm --noprogress \
    /root/openbsd-netcat-1.206_1-1-x86_64.pkg.tar.zst \
    /root/libbsd-0.10.0-2-x86_64.pkg.tar.zst

COPY --from=build /tmp/build_output/integration-test-client /usr/bin/integration-test-client

ENTRYPOINT ["/root/flexo_test_wrap_output"]
