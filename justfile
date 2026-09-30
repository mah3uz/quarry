# quarry development tasks. Run `just` to see them all.

set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments

version := `sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1`
target := `rustc -vV | sed -n 's/^host: //p'`

[private]
default:
    @just --list

# Run quarry from source, e.g. `just run demo.db -e 'select 1'`
[group('dev')]
run *args:
    cargo run -- "$@"

# Open the TUI from source, e.g. `just tui postgres://me@localhost/app`
[group('dev')]
tui *args:
    cargo run -- --tui "$@"

# Debug build
[group('dev')]
build:
    cargo build

# Run the tests (Postgres/MySQL tests use QUARRY_TEST_PG / QUARRY_TEST_MYSQL and skip if unreachable)
[group('check')]
test *args:
    cargo test "$@"

# Run the slow, ignored timing tests in release mode
[group('check')]
test-slow:
    cargo test --release -- --ignored

# Clippy with warnings as errors
[group('check')]
lint:
    cargo clippy --all-targets -- -D warnings

# Everything CI should run: lint, then tests
[group('check')]
check: lint test

# Install the quarry binary into ~/.cargo/bin
[group('install')]
install:
    cargo install --path . --locked

# Remove the installed binary (settings and history are kept)
[group('install')]
uninstall:
    cargo uninstall quarry

# Build an optimised binary and package it as target/dist/quarry-<version>-<target>.tar.gz
[group('release')]
release:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release --locked
    name="quarry-{{version}}-{{target}}"
    dist=target/dist
    rm -rf "$dist/$name" && mkdir -p "$dist/$name"
    cp target/release/quarry README.md "$dist/$name/"
    tar -C "$dist" -czf "$dist/$name.tar.gz" "$name"
    rm -rf "$dist/$name"
    if command -v sha256sum >/dev/null; then sum=(sha256sum); else sum=(shasum -a 256); fi
    (cd "$dist" && "${sum[@]}" "$name.tar.gz" > "$name.tar.gz.sha256")
    echo "$dist/$name.tar.gz"
    cat "$dist/$name.tar.gz.sha256"

# Regenerate logo.svg, banner.svg and the docs site's copies
[group('art')]
art:
    python3 scripts/gen_art.py

[private]
docs-deps:
    [ -d docs/node_modules ] || npm --prefix docs ci

# Preview the docs site with live reload at http://localhost:4321
[group('docs')]
docs: docs-deps
    npm --prefix docs run dev

# Build the docs site into docs/dist
[group('docs')]
docs-build: docs-deps
    npm --prefix docs run build

# Serve the built docs site
[group('docs')]
docs-preview: docs-build
    npm --prefix docs run preview

# Remove build output (Rust and docs)
[group('clean')]
clean:
    cargo clean
    rm -rf docs/dist docs/.astro
