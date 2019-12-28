#!/bin/bash

cd /tmp || exit 1
mkdir server
mkdir curl_ex_out

cd server || exit 1

dd if=/dev/zero of=zero1 bs=10M count=10
dd if=/dev/zero of=zero2 bs=10M count=10
dd if=/dev/zero of=zero3 bs=10M count=10
dd if=/dev/zero of=zero4 bs=10M count=10
dd if=/dev/zero of=zero5 bs=10M count=10

dd if=/dev/urandom of=random1 bs=10M count=10
dd if=/dev/urandom of=random2 bs=10M count=10
dd if=/dev/urandom of=random3 bs=10M count=10
dd if=/dev/urandom of=random4 bs=10M count=10
dd if=/dev/urandom of=random5 bs=10M count=10

systemd-run --user --unit server_8000 /usr/bin/npx http-server /tmp/server -p 8000 -a 127.0.0.1
systemd-run --user --unit server_8001 /usr/bin/npx http-server /tmp/server -p 8001 -a 127.0.0.1
systemd-run --user --unit server_8002 /usr/bin/npx http-server /tmp/server -p 8002 -a 127.0.0.1
