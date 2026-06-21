#!/bin/bash

mkdir -p /tmp/var/cache/flexo/pkg && \
mkdir -p /tmp/var/cache/flexo/state && \
mkdir -p /tmp/var/cache/flexo/pkg/community/os/x86_64 && \
mkdir -p /tmp/var/cache/flexo/pkg/community-staging/os/x86_64 && \
mkdir -p /tmp/var/cache/flexo/pkg/community-testing/os/x86_64 && \
mkdir -p /tmp/var/cache/flexo/pkg/core/os/x86_64 && \
mkdir -p /tmp/var/cache/flexo/pkg/extra/os/x86_64 && \
mkdir -p /tmp/var/cache/flexo/pkg/gnome-unstable/os/x86_64 && \
mkdir -p /tmp/var/cache/flexo/pkg/kde-unstable/os/x86_64 && \
mkdir -p /tmp/var/cache/flexo/pkg/multilib/os/x86_64 && \
mkdir -p /tmp/var/cache/flexo/pkg/multilib-testing/os/x86_64 && \
mkdir -p /tmp/var/cache/flexo/pkg/staging/os/x86_64 && \
mkdir -p /tmp/var/cache/flexo/pkg/testing/os/x86_64

exec /usr/bin/flexo
