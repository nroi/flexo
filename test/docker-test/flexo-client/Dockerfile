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

COPY 'packages/curl-7.70.0-1-x86_64.pkg.tar.zst' \
     'packages/diffutils-3.7-3-x86_64.pkg.tar.xz' \
     'packages/parallel-20191122-1-any.pkg.tar.xz' \
     'packages/tcpdump-4.9.3-1-x86_64.pkg.tar.xz' \
     'packages/wget-1.20.3-2-x86_64.pkg.tar.xz' /root/

RUN pacman -U --noconfirm \
    'root/curl-7.70.0-1-x86_64.pkg.tar.zst' \
    'root/diffutils-3.7-3-x86_64.pkg.tar.xz' \
    'root/parallel-20191122-1-any.pkg.tar.xz' \
    'root/tcpdump-4.9.3-1-x86_64.pkg.tar.xz' \
    'root/wget-1.20.3-2-x86_64.pkg.tar.xz'

ENTRYPOINT ["/root/flexo_test_wrap_output"]

