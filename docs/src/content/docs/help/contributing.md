---
title: Contributing
description: Build quarry, run its tests, and work on the artwork and this documentation site.
---

## Build and test

quarry is a Rust project. Common tasks are in the `justfile` at the repository root; run
[`just`](https://github.com/casey/just) to list them:

| Recipe | Does |
|---|---|
| `just run …` / `just tui …` | Run quarry from source, e.g. `just run demo.db` |
| `just check` | Clippy (warnings are errors), then the tests |
| `just test-slow` | The slow timing tests, in release mode |
| `just install` | Install the binary into `~/.cargo/bin` |
| `just release` | Build and package `target/dist/quarry-<version>-<target>.tar.gz` with a checksum |
| `just art` | Regenerate the logo and banner |
| `just docs` / `just docs-build` | Preview or build this site |

Or use Cargo directly, from the repository root:

```sh
cargo build            # debug build
cargo test             # unit and integration tests
cargo clippy --all-targets
```

The integration tests for PostgreSQL and MySQL need running servers. Point quarry at them with
environment variables; tests for a server that can't be reached are skipped:

```sh
QUARRY_TEST_PG=postgres://postgres@127.0.0.1/postgres \
QUARRY_TEST_MYSQL=mysql://root@127.0.0.1 \
cargo test
```

The tests create their own `quarry_test_*` databases and drop them afterwards.

## Project layout

| Path | Contents |
|---|---|
| `src/cli.rs`, `src/main.rs` | Command-line options and start-up |
| `src/conn/` | Connection URLs, password files, SSH tunnels |
| `src/db/` | The PostgreSQL, MySQL and SQLite drivers, and the schema catalog |
| `src/repl/` | The REPL: prompt, key bindings, session |
| `src/tui/` | The TUI |
| `src/complete/` | Context-aware completion |
| `src/special/` | Special commands |
| `src/sql/` | Statement splitting, classification, formatting |
| `src/output/` | Table and machine output formats, pager |
| `src/llm/` | `\llm` providers and the `--setup-llm` wizard |
| `src/theme.rs` | Built-in themes and custom theme loading |
| `tests/` | Integration tests |
| `docs/` | This site |

## The logo and banner

`logo.svg` and `banner.svg` in the repository root are generated. Change the mascot in
`scripts/gen_art.py`, then run:

```sh
just art        # or: python3 scripts/gen_art.py
```

The animations leave every shape at its resting pose, so renderers without SVG animation still draw
the static artwork.

## This documentation site

The site lives in `docs/` and is built with [Astro Starlight](https://starlight.astro.build). Pages
are Markdown files in `docs/src/content/docs/`; the sidebar is defined in `docs/astro.config.mjs`.

```sh
just docs         # live preview at http://localhost:4321
just docs-build   # static site in docs/dist
```

When you change quarry's behaviour, update the matching page here in the same change. The reference
pages ([options](/reference/cli/), [commands](/reference/special-commands/),
[keys](/reference/keys/), [config](/reference/config/)) are the ones most likely to need it.

The terminal screenshots on the home page and the TUI guide are real captures (`tmux capture-pane -e -p`)
stored in `docs/src/captures/`, and rendered to HTML at build time.
