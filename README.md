# Flexo

 <img src="flexo_beard.svg" width="120">

Flexo is a central cache for pacman, the package manager of Arch Linux.

## Why should I use it?

* If you're bothered by slow mirrors: Flexo runs a performance test during startup to make it
less likely that you end up with a slow, outdated or unreliable mirror. It can also transparently
switch from one mirror to another if a mirror turns out to be too slow.
* If you have multiple machines running ArchLinux, and you don't want each machine to download
and store the packages: You can just set flexo as your new ArchLinux mirror so that no file needs
to be downloaded more than once.
* If you run ArchLinux inside docker, you may be annoyed when packages have to be downloaded and installed on the container even though they have already been downloaded on the host: Just install Flexo on the host and run this command on the docker container:
    ````
    echo 'Server = http://172.17.0.1:7878/$repo/os/$arch' > /etc/pacman.d/mirrorlist
    ````
  so that packages that have already been downloaded will be fetched from the cache.
## Installation
A package for Arch Linux is available on [AUR](https://aur.archlinux.org/packages/flexo-git/).
Alternatively, you can use the [docker image](https://hub.docker.com/r/nroi/flexo) instead.
Flexo needs to be installed on a single machine (the server) so that it can be accessed by
multiple clients.
Once you have installed flexo on the server, start and enable the systemd service:
```
systemctl start flexo.service
systemctl enable flexo.service
```
Next, set the new mirror in `/etc/pacman.d/mirrorlist` on all clients.
In most cases, the server that runs flexo will also be a client that uses flexo, so
add the following entry to the top of your mirrorlist:
```bash
Server = http://localhost:7878/$repo/os/$arch
```
If you have additional ArchLinux clients in your LAN, then modify their mirrorlist as well.
Instead of referring to localhost, use the appropriate IP address or hostname:
```bash
Server = http://<FLEXO_SERVER_IP_ADDRESS>:7878/$repo/os/$arch
```

## Features

* Concurrent downloads: You can have multiple clients downloading files from flexo without one client having to wait.
* Efficient bandwidth sharing for concurrent downloads: Flexo does not require a new connection to the remote mirror
  when the same file is downloaded by multiple clients. For instance, suppose a client starts downloading a given file.
  After 5 seconds have elapsed, 100MB have been downloaded. Now, a second client requests the same file. The second
  client will receive the first 100MB immediately from the local file system. Then, both clients continue to download
  the file, while only a single connection to the remote mirror exists. This means that your bandwidth is not split for
  the two clients, both clients will be able to download the file with the full download speed provided by your ISP.
* Persistent connections: This is especially useful when many small files are downloaded, since no new TLS negotiation
  is required for each file.

## Configuration

The AUR package will install the configuration file in `/etc/flexo/flexo.toml`.
It includes many comments and should be self explanatory (open an issue in case you disagree).
In most cases, you will want to leave all settings untouched, with two exceptions:

1. The setting `low_speed_limit` is commented by default, which means that flexo will *not* attempt
to switch to a faster mirror if a download is extremely slow. To make use of this feature,
uncomment the setting and enter an appropriate value.

2. The setting `allowed_countries` is set to the empty list by default, which means that at the first start and at
   regular intervals, Flexo will run latency tests on all official mirrors from all continents. Add the ISO code
   of your own country (and perhaps a few neighboring countries) to improve the startup time of Flexo.

## Troubleshooting

If Flexo does not start at all or crashes, check the logs first:
```bash
journalctl --unit=flexo
```
If that does not help you, please open an issue that includes:
1. An excerpt of that log, if Flexo has crashed or did not start.
2. Your installation method (docker or AUR).
3. The version you are using (either the output of `pacman -Qi flexo`, or the tag if you are using Docker).
4. Your settings, if you have changed them
   (either the `/etc/flexo/flexo.toml` file, or the environment variables if you use Docker).
5. If the issue is related to the mirror selection, it might also help if you include the country you are located in
and the `/var/cache/flexo/state/latency_test_results.json` file, if it exists.

For issues related to the mirror selection, also see [this page](./mirror_selection.md) for more details.

## Attributes & Design Goals
* Lightweight: Flexo is a single binary with less than 3 MB and a low memory footprint.
* Robust: Flexo includes certain functions to allow it to switch from one mirror to another if a
mirror is too slow or causes other issues.
* Simple: Users should not require more than a few minutes to set up flexo and understand what it does.

## Cleaning the package cache

`paccache` from [pacman-contrib](https://www.archlinux.org/packages/?name=pacman-contrib) can be used to purge old
packages. Install it if you haven't done so already:
```bash
sudo pacman -S pacman-contrib
```

Packages are stored in the directory specified by the `cache_directory` variable in `/etc/flexo/flexo.toml`. By default,
it's `/var/cache/flexo`. Use `paccache` to clean up the subdirectories of this directory. For instance, the following
will delete all packages except for the three most recent versions:

```bash
paccache -r -k3  $(find /var/cache/flexo/pkg -type d -name x86_64 -printf "-c %p ")
```

## Contribute
If you know rust, feel free to dive into the code base and send a PR. Smaller improvements
to make the code base cleaner, more idiomatic or efficient are always welcome. Before submitting
larger changes, including new features or design changes, you should first open an issue to see
if that feature is desired and if it fits into the design goals of flexo.

## Development

Details about design decisions and the terminology used in the code
are described [here](flexo/terminology.md).

The following packages are required to build and test flexo:

```bash
pacman -S rustup docker docker-compose curl
```

The [./docker-compose](test/docker-test-local/docker-compose) script may require you to be able to use docker
without root privileges. Add your user to the docker group to do so. You may want to read
the [wiki](https://wiki.archlinux.org/index.php/Docker) for more details on on the security
implications of doing so.

```
gpasswd -a <user> docker
```

Furthermore, Docker BuildKit is required to run the integration tests inside docker, so make sure you have enabled it.
One way to enable it is to modify your `~/.docker/config.json` to include the following:
```json
{
    "experimental": "enabled"
}
```

Make sure to restart the docker daemon after modifying this file.

We have two types of test cases:
1. Tests written in Rust: [integration_tests.rs](flexo/tests/integration_test.rs). These tests run quickly
and they are fully deterministic (afaik). You can run them with `cargo`:
    ```
   cd flexo
   cargo test
    ```
2. end-to-end tests using Docker.
We try to avoid flaky test cases, but we cannot guarantee that all our docker test cases are deterministic,
since their outcome depends on various factors outside of our control (e.g. how the scheduler runs OS processes,
how TCP packets are assembled by the kernel's TCP stack, etc.).  
As a result, a failing end-to-end
test may indicate that a new bug was introduced, but it might also have been bad luck or a badly written test case.

In order to run the Docker test cases, run the shell script to set up everything:

```
cd test/docker-test-local
./docker-compose
```

Most of the output is relevant only if you need to investigate failing test cases. The outcome
of all test cases is shown towards the end:

```
flexo-client_1  | Test summary:
flexo-client_1  | flexo-test-install                       [SUCCESS]
flexo-client_1  | flexo-test-install-cached                [SUCCESS]
flexo-client_1  | flexo-test-download-cached-concurrently  [SUCCESS]
flexo-client_1  | flexo-test-download-concurrently         [SUCCESS]
```
