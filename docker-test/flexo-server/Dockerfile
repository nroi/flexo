FROM archlinux

EXPOSE 7878

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
    mkdir -p /var/cache/flexo/testing/os/x86_64 && \
    echo 'Server = http://localhost:7878/$repo/os/$arch' > /etc/pacman.d/mirrorlist


COPY flexo.toml /etc/flexo/
COPY flexo /usr/bin/flexo

ENTRYPOINT /usr/bin/flexo
