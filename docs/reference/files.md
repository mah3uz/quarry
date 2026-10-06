---
title: 'Files and directories'
description: 'Where quarry keeps its configuration, history, logs, credentials and themes.'
---

# Files and directories

quarry uses two directories, following the XDG layout on Linux and macOS alike:

| Directory | Default | Override |
|---|---|---|
| Config | `~/.config/quarry` | `QUARRY_CONFIG_DIR`, or `$XDG_CONFIG_HOME/quarry` |
| Data | `~/.local/share/quarry` | `QUARRY_DATA_DIR`, or `$XDG_DATA_HOME/quarry` |

Keep the config directory in your dotfiles if you like: it holds settings, favourites and themes,
but no API keys.

## Config directory

| File | Contents |
|---|---|
| `config.toml` | Settings and saved connections. Created with comments on first run. See [Configuration file](/reference/config). |
| `config.toml.bak` | The previous `config.toml`, kept whenever quarry changes it |
| `favorites.toml` | [Favourite queries](/advanced/favorites) |
| `themes/` | Your [custom themes](/advanced/themes#your-own-theme): `*.toml`, `*.yaml`, `*.yml` |

With `--config FILE`, `favorites.toml` and `themes/` are read from the directory that holds `FILE`.

## Data directory

| File | Contents |
|---|---|
| `history.txt` | Statement history, shared by the REPL and the TUI |
| `quarry.log` | Every statement, when `log_queries = true` |
| `credentials.toml` | API keys saved by `--setup-llm` |
| `ui-state.toml` | The theme you last picked in the TUI, and each connection's open tabs with the text of its query tabs (unless `restore_tabs = false`) |

## Permissions

`config.toml`, `favorites.toml`, `history.txt`, `quarry.log`, `credentials.toml`, `ui-state.toml`, and
files you write with `\o`, `tee` and `\export`, are created readable only by you (mode 600).

## Starting fresh

```sh
rm -rf ~/.config/quarry ~/.local/share/quarry
```

quarry creates a new commented `config.toml` next time it runs.
