FROM archlinux

RUN pacman -Sy --noprogress --noconfirm tcpdump coreutils wget parallel
RUN echo 'Server = http://flexo-server:7878/$repo/os/$arch' > /etc/pacman.d/mirrorlist

# Keep the container running so that we can start an interactive shell session.
ENTRYPOINT ["tail", "-f", "/dev/null"]
