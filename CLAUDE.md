# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Flexo is a caching proxy for `pacman`, the Arch Linux package manager. It sits between Arch clients
and the official mirrors: it picks low-latency mirrors automatically, caches downloaded packages, and
shares a single upstream connection across multiple clients downloading the same file simultaneously.

It is a single Rust binary that listens on TCP (default port 7878) and speaks HTTP by hand (no async
runtime, no web framework). Concurrency is one OS thread per client connection plus background
worker threads for downloads. Fetching uses `curl`; serving cached files uses `sendfile` via `libc`.

## Repository layout

The Cargo project is **not** at the repo root — it lives in the `flexo/` subdirectory. Always run
cargo commands from there.

```
flexo/            <- the Rust crate (run cargo here)
  src/
  conf/flexo.toml <- annotated example config
test/             <- docker-compose integration tests + helper crates (tcp-proxy-delay, integration-test-client)
.github/workflows/rust.yml
mirror_selection.md  <- design notes on how mirrors are scored/selected
```

## Build, test, run

All cargo commands run from `flexo/`:

```bash
cargo build
cargo test                              # unit + in-crate tests
cargo test test_filesize_exceeds_sendfile_count   # single test by name
cargo run                               # reads /etc/flexo/flexo.toml or FLEXO_* env vars
```

CI (`.github/workflows/rust.yml`) just runs `cargo build` and `cargo test` with working-directory
`./flexo` on push to `master`/`dev` and PRs to `master`.

### Integration tests (Docker)

End-to-end tests spin up mock mirrors (fast/slow/stalling/redirecting/no-content-length) plus Flexo
servers and a client, and assert behavior under adverse mirror conditions. Run from
`test/docker-test-local/`:

```bash
./docker-compose          # builds tarballs of the source, then runs docker-compose up
```

The script tars `flexo/src` + `Cargo.toml`/`Cargo.lock` into the test images, so the integration
tests always build from the current working-tree source. The run exits with the flexo-client's exit
code. Each `mirror-*-mock` and `flexo-server-*` directory under `docker-test-local/` is one scenario.

## Architecture

### Generic scheduling core (`src/lib.rs`)

`lib.rs` is a **provider-agnostic job-scheduling framework** expressed entirely through traits — it
contains no Arch/pacman/HTTP specifics. The key abstractions:

- `Job` — a unit of work with a large associated-type bundle (`Order`, `Provider`, `Channel`,
  score type `S`, properties `PR`, etc.).
- `Provider` — a source that can serve jobs (here: a mirror). Has an `initial_score` (latency-test
  derived, known before use) and accrues a dynamic score from real success/failure.
- `Order` — a request for a specific artifact; knows how to open/reuse a `Channel`.
- `Channel` — a reusable connection (enables persistent connections across downloads).
- `JobContext` — the orchestrator. `try_schedule()` is the entry point: it deduplicates in-progress
  orders, picks the best provider (`ProviderGuards` / `provider_guards.rs` manage per-provider
  locking and choice), reuses or opens channels, and retries on failure (`NUM_MAX_ATTEMPTS`,
  `TIMEOUT_ALL_RETRIES`). It returns a `ScheduleOutcome`: `Cached`, `Scheduled`, `AlreadyInProgress`,
  `Uncacheable`, or `Unavailable`.

When changing scheduling/retry/provider-selection logic, this is the file — but note that a lot of
the retry policy lives here while the per-request HTTP response handling lives in `main.rs`, and the
two are coupled by timeouts (e.g. `TIMEOUT_ALL_RETRIES` in lib.rs must stay below
`TIMEOUT_RECEIVE_CONTENT_LENGTH` in main.rs). There are TODOs in `main.rs` noting that retry-to-
another-mirror logic can't currently be triggered from the request handler because it lives in lib.rs.

### Arch-specific implementation (`src/mirror_flexo.rs`)

Concrete impls of the `lib.rs` traits for downloading Arch packages: `DownloadJob`, `DownloadProvider`
(a mirror), `DownloadOrder`, `DownloadChannel`, plus the curl `Handler` (`DownloadState`) that streams
bytes to disk and reports progress. Also: client HTTP request parsing (`Request`, `read_client_header`,
range/`resume_from` handling), latency-test scoring (`MirrorResults`, `rated_providers*`), and cache
inspection (`inspect_and_initialize_cache`).

### The growing-file mechanism

This is the core trick that lets concurrent clients share one upstream download. While a package is
being downloaded, it is written to a file that grows over time, and a sidecar **`.cfs` file** ("complete
file size") records the expected final size. Clients are served from this *growing* file:
`serve_from_growing_file` / `serve_growing_file_loop` in `main.rs` stream bytes as they arrive, blocking
when they reach the current end-of-file and resuming when more data is written, until the `.cfs`-recorded
size is reached. `GROWING_FILE_STALL_TIMEOUT` bounds how long the file may stop growing before giving up.
A second client requesting an `AlreadyInProgress` order joins by reading the same growing file rather
than opening a new upstream connection.

### TCP server & request routing (`src/main.rs`)

`main()` loads config, initializes/cleans the cache, runs initial mirror latency tests, then accepts
connections (one thread per client via `serve_client`). `serve_request` routes:
- special paths: `status` (200), `metrics` (JSON of per-provider success/failure), `reset-metrics`
  (POST), path validation (`permitted_path`/`valid_path` → 403/400);
- everything else becomes a `DownloadOrder` → `try_schedule` → serve from cache, from a growing file,
  or via redirect (for uncacheable / custom-repo content).

Cache purging (`purge_cache`, keeps `num_versions_retain` versions via `paccache`/scruffy;
`purge_cfs_files`, `purge_uncacheable_files`) also lives here.

### Config (`src/mirror_config.rs`)

Config comes from `/etc/flexo/flexo.toml` **or** `FLEXO_`-prefixed environment variables (Docker
usage). Every TOML key has an env-var equivalent (e.g. `listen_ip_address` ↔ `FLEXO_LISTEN_IP_ADDRESS`;
nested keys like `mirrors_auto.allowed_countries` ↔ `FLEXO_MIRRORS_AUTO_ALLOWED_COUNTRIES`). The
annotated reference config is `flexo/conf/flexo.toml`. `custom_repo` entries route `/custom_repo/<name>/...`
paths to non-official mirrors (e.g. ArchLinuxARM, AUR-adjacent repos).

### Mirror fetching/caching (`src/mirror_fetch.rs`, `src/mirror_cache.rs`)

`mirror_fetch.rs` pulls the official mirror list and runs latency tests; `mirror_cache.rs` persists the
chosen/rated providers (e.g. `latency_test_results.json` under the state dir) so subsequent startups are
fast. `latency_tests_refresh_required` / `get_country_filter` decide when to re-test. See
`mirror_selection.md` for the scoring rationale.

## Conventions worth knowing

- Edition 2018, no async. Threads + crossbeam channels for concurrency; `Arc<Mutex<...>>` for shared
  state (`JobContext` is wrapped in one and locked per request — keep critical sections short).
- HTTP responses are written by hand in `http_headers.rs` + the `serve_*_header` helpers in `main.rs`.
- `#[cfg(test)]` toggles constants to make tests fast (e.g. `MAX_SENDFILE_COUNT` shrinks to 128).
- A panic in any thread exits the whole process (a panic hook calls `std::process::exit(1)`), so a
  panic is a hard crash, not a silently dead worker thread.
