# Flexo <img src="flexo_beard.svg" width="50">

Flexo is a caching proxy for pacman, the package manager of Arch Linux.

## Why should I use it?

* If you're bothered by slow mirrors: Instead of manually maintaining a `mirrorlist`, Flexo automatically chooses
a low-latency mirror for you and switches to another mirror if the selected mirror turns out to be slow.
* If you have multiple machines running ArchLinux, and you don't want each machine to download
and store the packages: You can just set flexo as your new ArchLinux mirror so that no file needs
to be downloaded more often than once.
* If you run ArchLinux inside Docker, you may be annoyed when packages have to be downloaded and installed on the container even though they have already been downloaded on the host: Just install Flexo on the host and run this command on the Docker container:
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

* Concurrent downloads: You can have multiple clients downloading files from Flexo without one client having to wait.
* Efficient bandwidth sharing for concurrent downloads: Flexo does not require a new connection to the remote mirror
  when the same file is downloaded by multiple clients. For instance, suppose a client starts downloading a given file.
  After 5 seconds have elapsed, 100 MB have been downloaded. Now, a second client requests the same file. The second
  client will receive the first 100 MB immediately from the local file system. Then, both clients continue to download
  the file, while only a single connection to the remote mirror exists. This means that your bandwidth is not split for
  the two clients, both clients will be able to download the file with the full download speed provided by your ISP.
* Persistent connections: This is especially useful when many small files are downloaded, since no new TLS negotiation
  is required for each file.
* The package cache is cleaned automatically: No need to set up cron jobs or systemd timers to clean the cache
  regularly, Flexo will automatically ensure that only the 3 most recent versions of a package are kept in your cache
  (this parameter can be changed).

## Configuration

The AUR package will install the configuration file in `/etc/flexo/flexo.toml`.
It includes many comments and should be self-explanatory (open an issue in case you disagree).
In most cases, you will want to leave all settings unchanged, with two exceptions:

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
2. Your installation method (Docker or AUR).
3. The version you are using (either the output of `pacman -Qi flexo`, or the tag if you are using Docker).
4. Your settings, if you have changed them
   (either the `/etc/flexo/flexo.toml` file, or the environment variables if you use Docker).
5. If the issue is related to the mirror selection, it might also help if you include the country you are located in
and the `/var/cache/flexo/state/latency_test_results.json` file, if it exists.

For issues related to the mirror selection, also see [this page](./mirror_selection.md) for more details.

## Attributes & Design Goals
* Lightweight: Flexo is a single binary with less than 3 MB and a low memory footprint.
* Robust: As long as *most* mirrors work fine, Flexo should be able to handle the download process
  without the client noticing any issues or interruptions, even if remote mirrors are slow or connections
  are unexpectedly dropped.
* Simple: Users should not require more than a few minutes to set up flexo and understand what it does.


## Cleaning the package cache

The default configuration of Flexo will keep 3 versions of a package in cache: After a 4th version of a package has been
downloaded, the oldest version will be automatically removed. This setting can be changed with the `num_versions_retain`
parameter. See the [configuration example](./flexo/conf/flexo.toml) for more details.

If you use Docker, the default behavior can be changed with the `FLEXO_NUM_VERSIONS_RETAIN` environment variable.

If you want to disable this setting and never purge the cache, set the parameter to `0`.

## Using Unofficial User Repositories

If you are using [unofficial user repositories](https://wiki.archlinux.org/index.php/Unofficial_user_repositories)
and you want Flexo to cache packages from those repositories, both `pacman.conf` and `flexo.toml`
need to include the custom repository. For example, suppose that Flexo is running on localhost, port 7878,
and you want to add two custom repositories: archzfs and eschwartz. First, adapt your `/etc/pacman.conf` to include
both repositories. Notice that the path must start with `custom_repo/<repo-name>`:

```
[archzfs]
Server = http://localhost:7878/custom_repo/archzfs/$repo/$arch

[eschwartz]
Server = http://localhost:7878/custom_repo/eschwartz/~eschwartz/repo/$arch
```
Next, add the corresponding entries to your `/etc/flexo/flexo/toml` before the `[mirrors_auto]` section:

```toml
[[custom_repo]]
name = "archzfs"
url = "https://archzfs.com"

[[custom_repo]]
name = "eschwartz"
url = "https://pkgbuild.com"
```

Notice that the names (in this case `archzfs` and `eschwartz`) must match the path component right after
the `/custom_repo` in `pacman.conf`: So if your `pacman.conf` includes a repo with the path `/custom_repo/foo`,
then your `flexo.toml` must include a matching `[[custom_repo]]` entry with `name = "foo"`.

Alternatively, if you use Docker, set the environment variable instead of modifying the `flexo.toml` file:
```bash
FLEXO_CUSTOM_REPO="eschwartz@https://pkgbuild.com archzfs@https://archzfs.com"
```

## Contribute
If you know rust, feel free to dive into the code base and send a PR. Smaller improvements
to make the code base cleaner, more idiomatic or efficient are always welcome. Before submitting
larger changes, including new features or design changes, you should first open an issue to see
if that feature is desired and if it fits into the design goals of flexo.

## Development

Details about design decisions, and the terminology used in the code,
are described [here](flexo/terminology.md).

The following packages are required to build and test flexo:

```bash
pacman -S rustup docker docker-compose curl
```

The [./docker-compose](test/docker-test-local/docker-compose) script may require you to be able to use Docker
without root privileges. Add your user to the Docker group to do so. You may want to read
the [wiki](https://wiki.archlinux.org/index.php/Docker) for more details on the security
implications of doing so.

```
gpasswd -a <user> docker
```

Furthermore, Docker BuildKit is required to run the integration tests inside Docker, so make sure you have enabled it.
One way to enable it is to modify your `~/.docker/config.json` to include the following:
```json
{
    "experimental": "enabled"
}
```

Make sure to restart the Docker daemon after modifying this file.

We have two types of test cases:
1. Tests written in Rust: [integration_tests.rs](flexo/tests/integration_test.rs). These tests run quickly,
and they are fully deterministic (afaik). You can run them with `cargo`:
    ```
   cd flexo
   cargo test
    ```
2. end-to-end tests using Docker.
We try to avoid flaky test cases, but we cannot guarantee that all our Docker test cases are deterministic,
since their outcome depends on various factors outside our control (e.g. how the scheduler runs OS processes,
how TCP packets are assembled by the kernel's TCP stack, etc.).
As a result, a failing end-to-end test may indicate that a new bug was introduced, but it might also have been bad luck
or a badly written test case.

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
