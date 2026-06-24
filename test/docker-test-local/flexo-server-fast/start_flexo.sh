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

# Partial cache for flexo_test_resume_offset_not_leaked_across_channel_reuse (regression
# test for issue #93): 50 of 100 bytes are cached, so fetching this file makes flexo send
# a range request (resume_from=50) to the mirror. The test then fetches an uncached file
# over the same reused connection to ensure that leftover offset is not reused.
truncate -s 50 /tmp/var/cache/flexo/pkg/partially-cached-leak
echo '100' > /tmp/var/cache/flexo/pkg/.partially-cached-leak.cfs

exec /usr/bin/flexo
