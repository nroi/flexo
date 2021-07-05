# Docker e2e Tests

Notice that the setup for the docker e2e tests can be time-consuming. Furthermore, at least 32 GB RAM are required
to run the tests. Therefore, running the docker e2e tests before submitting a PR is **optional**. Just run
`cargo test` before submitting a PR.

The following packages are required to run the docker tests:

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

2. end-to-end tests using Docker.
   We try to avoid flaky test cases, but we cannot guarantee that all our Docker test cases are deterministic,
   since their outcome depends on various factors outside our control (e.g. how the scheduler runs OS processes,
   how the kernel's TCP stack assembles TCP packets, etc.).
   As a result, a failing end-to-end test may indicate that a new bug was introduced, but it might also have been bad luck
   or a badly written test case.

Also notice that the Docker test cases currently require at least 32 GB of RAM, because all files are downloaded
in a tmpfs.

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
