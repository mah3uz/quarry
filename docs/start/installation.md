---
title: 'Installation'
description: 'Install quarry from the AUR on Arch Linux, download the prebuilt Linux binary, or build it from source with Cargo.'
---

# Installation

| You're on | Easiest way |
|---|---|
| Arch Linux or a derivative | [The AUR](#arch-linux-aur) |
| Another Linux on x86_64 | [The prebuilt binary](#prebuilt-binary-linux-x86-64) |
| macOS, or anything else | [Cargo](#build-from-source) |

## Arch Linux (AUR)

Two packages install the same `quarry` command, with shell completion for bash, zsh and fish:

| Package | What it installs |
|---|---|
| `quarry-sql-bin` | The prebuilt binary from the GitHub release. Quick to install. |
| `quarry-sql` | Built from source on your machine. Needs Rust (`cargo`). |

With an AUR helper such as `paru` or `yay`:

```sh
paru -S quarry-sql-bin     # or: yay -S quarry-sql-bin
```

Or by hand:

```sh
git clone https://aur.archlinux.org/quarry-sql-bin.git
cd quarry-sql-bin
makepkg -si
```

The packages are called `quarry-sql` because the AUR's `quarry` is an unrelated game. Both provide
`/usr/bin/quarry`, so pacman won't install them side by side.

## Prebuilt binary (Linux x86_64)

Each [GitHub Release](https://github.com/mah3uz/quarry/releases) has a
`quarry-<version>-x86_64-unknown-linux-gnu.tar.gz` with the `quarry` binary, the README and the
licence:

```sh
curl -LO https://github.com/mah3uz/quarry/releases/latest/download/quarry-0.1.0-x86_64-unknown-linux-gnu.tar.gz
tar -xzf quarry-0.1.0-x86_64-unknown-linux-gnu.tar.gz
install -Dm755 quarry-0.1.0-x86_64-unknown-linux-gnu/quarry ~/.local/bin/quarry
```

Replace `0.1.0` with the version you're downloading. Each tarball has a `.sha256` file next to it
to check the download with `sha256sum -c`. The binary is built on a current Linux; if it complains
about the `GLIBC` version on an older system, build from source instead.

## Build from source

quarry is built with Cargo, Rust's package manager.

### Requirements

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

### Build and install

```sh
cargo install --git https://github.com/mah3uz/quarry
```

Or from a checkout, which is handy if you want to change something:

```sh
git clone https://github.com/mah3uz/quarry
cd quarry
cargo install --path .
```

Cargo builds an optimised binary and puts it in `~/.cargo/bin`, which rustup adds to your `PATH`.
Check that it worked:

```sh
quarry --version
```

## A Nerd Font

quarry's icons come from a [Nerd Font](https://www.nerdfonts.com). If your terminal doesn't use
one, install one (on Arch, `pacman -S ttf-jetbrains-mono-nerd`) and select it in the terminal's
settings, or set `icons = "unicode"` in the config. See [icons](/advanced/customising#icons).

## Shell completion

quarry completes its options in bash, zsh and fish, and it knows your setup: <kbd>Tab</kbd> after
`quarry` offers your saved connections (with their URLs), SQLite files and URL schemes; after
`--theme` your themes, including your own; after `--ssh` the hosts in `~/.ssh/config`; after
`--host`, `--user` or `--database` the values your saved connections use.

The AUR packages install it for you. Otherwise add one line to your shell's startup file:

| Shell | Add to | Line |
|---|---|---|
| bash | `~/.bashrc` | `source <(quarry --completions bash)` |
| zsh | `~/.zshrc` (after `compinit`) | `source <(quarry --completions zsh)` |
| fish | `~/.config/fish/config.fish` | `quarry --completions fish \| source` |

The script is small and asks `quarry` for candidates each time, so new saved connections and themes
appear without regenerating anything.

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
[Files and directories](/reference/files) for the full list.

Next: take the [Quick start](/start/quick-start).
