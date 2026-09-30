---
title: Installation
description: Build and install quarry from source with Cargo.
---

quarry is built from source with Cargo, Rust's package manager.

## Requirements

- **A recent stable Rust toolchain.** Install it with [rustup](https://rustup.rs) if you don't have
  it. quarry uses the Rust 2024 edition, so update an older toolchain with `rustup update`.
- **A C compiler** (`cc`, `gcc` or `clang`). SQLite is compiled into quarry from source during the
  build.
- **Linux or macOS.** quarry runs the pager, shell commands and password commands through `sh`, so
  Windows is not supported yet.

At run time there are no libraries to install: TLS uses rustls and SQLite is bundled. A few features
use programs you probably have already:

| Program | Used for |
|---|---|
| `ssh` | SSH tunnels (`--ssh`) |
| `less` | Paging long results (or whatever `$PAGER` names) |
| `$VISUAL` / `$EDITOR` | `\e`, editing a query in your editor (defaults to `vi`) |

## Install

From a checkout of the quarry repository:

```sh
cargo install --path .
```

Cargo builds an optimised binary and puts it in `~/.cargo/bin`, which rustup adds to your `PATH`.
Check that it worked:

```sh
quarry --version
```

## Update

Pull the latest changes and run the same command again. Cargo replaces the old binary:

```sh
git pull
cargo install --path .
```

## Uninstall

```sh
cargo uninstall quarry
```

This removes the binary but leaves your settings and history. Delete those yourself if you want a
clean slate:

```sh
rm -rf ~/.config/quarry ~/.local/share/quarry
```

## Where quarry keeps its files

The first time it runs, quarry creates a commented configuration file at
`~/.config/quarry/config.toml`. History and other data go to `~/.local/share/quarry/`. Both follow
`$XDG_CONFIG_HOME` and `$XDG_DATA_HOME` when those are set, on macOS too. See
[Files and directories](/reference/files/) for the full list.

Next: take the [Quick start](/start/quick-start/).
