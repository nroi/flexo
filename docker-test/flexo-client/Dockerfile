FROM archlinux

RUN pacman -Sy --noprogress --noconfirm coreutils tcpdump
RUN echo 'Server = http://flexo-server:7878/$repo/os/$arch' > /etc/pacman.d/mirrorlist

COPY ./flexo_test_wrap_output /root/flexo_test_wrap_output
COPY ./flexo_test /root/flexo_test

ENTRYPOINT ["/root/flexo_test_wrap_output"]
