.PHONY: all build test clean run fmt lint update

# Default target
all: build

# Build the main flexo binary
build:
	cd flexo && cargo build

# Run all tests (unit and integration)
test:
	cd flexo && cargo test

# Run the flexo server
run:
	cd flexo && cargo run

# Clean build artifacts for all components
clean:
	cd flexo && cargo clean
	cd test/integration-test-client && cargo clean
	cd test/tcp-proxy-delay && cargo clean

# Format code across the project
fmt:
	cd flexo && cargo fmt
	cd test/integration-test-client && cargo fmt
	cd test/tcp-proxy-delay && cargo fmt

# Run clippy for linting
lint:
	cd flexo && cargo clippy -- -D warnings
	cd test/integration-test-client && cargo clippy -- -D warnings
	cd test/tcp-proxy-delay && cargo clippy -- -D warnings

# Update rust toolchain and dependencies
update:
	rustup update
	cd flexo && cargo update
	cd test/integration-test-client && cargo update
	cd test/tcp-proxy-delay && cargo update

# Build integration test utilities
build-utils:
	cd test/integration-test-client && cargo build
	cd test/tcp-proxy-delay && cargo build
