FROM archlinux:20200407

RUN echo 'Server = http://flexo-server:7878/$repo/os/$arch' > /etc/pacman.d/mirrorlist

COPY pkg.tar /tmp

# make db files locally available. This basically has the same effect as running 'pacman -Sy'.
RUN tar -C /tmp -xf /tmp/pkg.tar && \
    cp /tmp/core/os/x86_64/core.db /var/lib/pacman/sync && \
    cp /tmp/extra/os/x86_64/extra.db /var/lib/pacman/sync && \
    cp /tmp/community/os/x86_64/community.db /var/lib/pacman/sync

COPY ./flexo_test_wrap_output /root/flexo_test_wrap_output
COPY ./flexo_test /root/flexo_test

COPY packages/libbsd-0.10.0-2-x86_64.pkg.tar.zst /root
COPY packages/openbsd-netcat-1.206_1-1-x86_64.pkg.tar.zst /root

# netcat is required for our test cases. libbsd is installed since it is
# required by netcat.
RUN pacman -U --noconfirm --noprogress \
    /root/openbsd-netcat-1.206_1-1-x86_64.pkg.tar.zst \
    /root/libbsd-0.10.0-2-x86_64.pkg.tar.zst

ENTRYPOINT ["/root/flexo_test_wrap_output"]
