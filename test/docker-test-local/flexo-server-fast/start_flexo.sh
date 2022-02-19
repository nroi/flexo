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

# Notice that mirror-fast-mock has a file with the same name of size of 100
# bytes: We use this file to test Flexo's behavior when the file is locally
# cached, but only partially.
truncate -s 50 /tmp/var/cache/flexo/pkg/partially-cached
echo '100' > /tmp/var/cache/flexo/pkg/.partially-cached.cfs

exec /usr/bin/flexo
