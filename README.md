# Flexo

Successor of cpcache. Still a work in progress – Do not use unless you intend to waste your time.

## Long Term Goal
Provide the same functionality as cpcache, with the following advantages:
* More lightweight
* Easier to deploy
* More robust

The first two points are achieved by using Rust instead of Elixir. Robustness is achieved by using a
battletested and well known library: curl.

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

The [./docker-compose](docker-test/docker-compose) script may require you to be able to use docker
without root privileges. Add your user to the docker group to do so. You may want to read
the [wiki](https://wiki.archlinux.org/index.php/Docker) for more details on on the security
implications of doing so.
```
sudo gpasswd -a <user> docker
```

musl is required to compile the binary for alpine linux, which is used for our dockerized end-to-end tests:

```bash
rustup target add x86_64-unknown-linux-musl
```


Before submitting a PR, please make sure that all tests pass. We have two types
of test cases:
1. Tests written in Rust: [integration_tests.rs](flexo/tests/integration_test.rs). These tests run quickly
and they are fully deterministic (afaik). You can run them with `cargo`:
    ```
   cd flexo
   cargo test
    ```
2. end-to-end tests written in bash: [flexo_test](docker-test/flexo-client/flexo_test).
Keep in mind that our end-to-end tests are not deterministic: A failing end-to-end test may indicate that
a new bug was introduced, but it might also have been caused by a misbehaving remote mirror, or by a timeout
that did not occur in previous runs.
We use docker to set up the entire context required by our test-to-end tests:
    ```
    cd docker-test
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
