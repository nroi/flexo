FROM archlinux

RUN echo 'Server = http://flexo-server:7878/$repo/os/$arch' > /etc/pacman.d/mirrorlist

COPY ./flexo_test /root/flexo_test

ENTRYPOINT ["/root/flexo_test"]
