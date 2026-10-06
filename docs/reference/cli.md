---
title: 'Command-line options'
description: 'Every quarry command-line flag.'
---

# Command-line options

```
quarry [OPTIONS] [TARGET] [EXTRA]
```

`TARGET` is a URL, a saved connection name, a SQLite file, or a database name. See
[Connecting](/guides/connecting) for how it's interpreted. `EXTRA` is a database name when `TARGET`
is a URL without one, or the user in the pgcli-style `quarry dbname user`.

## Connection

| Option | Short | Meaning |
|---|---|---|
| `--host HOST` | `-h` | Host |
| `--port PORT` | `-p`, `-P` | Port |
| `--user USER` | `-u`, `-U` | User. Also `--username`. |
| `--database DB` | `-d`, `-D` | Database. Also `--dbname`. |
| `--socket PATH` | `-S` | Unix socket |
| `--backend NAME` | | `postgres` (`pg`, `postgresql`), `mysql` (`mariadb`, `my`) or `sqlite` (`sqlite3`, `lite`), for connecting with flags only |
| `--password` | `-W` | Always ask for a password before connecting |
| `--no-password` | `-w` | Never ask for a password |
| `--readonly` | `-r` | Refuse statements that change data or schema. See [read-only mode](/guides/safety#read-only-mode). |
| `--init-command SQL` | | Run SQL right after connecting. Repeatable. |

## TLS and SSH

| Option | Meaning |
|---|---|
| `--ssl-mode MODE` | `disable`, `prefer` (default), `require`, `verify-ca`, `verify-full` |
| `--ssl-ca FILE` | CA certificate to trust (PEM) |
| `--ssl-cert FILE` | Client certificate (PEM) |
| `--ssl-key FILE` | Client key (PEM) |
| `--ssh [USER@]HOST[:PORT]` | Tunnel through SSH |
| `--ssh-key FILE` | SSH private key |

See [TLS and SSH tunnels](/advanced/tls-ssh).

## Running SQL

| Option | Short | Meaning |
|---|---|---|
| `--execute SQL` | `-e` | Run SQL (or one special command) and exit. Repeatable. |
| `--file FILE` | `-f` | Run the SQL in a file and exit |
| `--format FORMAT` | `-F` | Output format: `rounded`, `psql`, `csv`, `tsv`, `json`, `markdown`, … See [Output formats](/reference/output-formats). |
| `--continue-on-error` | | Keep going after a failing statement |

See [Scripts, exports and pipes](/guides/scripting).

## Interface

| Option | Short | Meaning |
|---|---|---|
| `--tui` | `-T` | Open the full-screen TUI. Ignored in scripts. |
| `--theme NAME` | | Colour theme for this run |
| `--prompt FORMAT` | | Prompt format for this session. See [the prompt](/advanced/customising#the-prompt). |
| `--icons SET` | | `auto`, `nerd`, `unicode` or `ascii` for this run. See [icons](/advanced/customising#icons). |
| `--no-color` | | No colour |
| `--less-chatty` | | Skip the banner and the goodbye message |
| `--row-limit N` | | Ask before showing more than `N` rows. `0` never asks. |

## Configuration and connections

| Option | Short | Meaning |
|---|---|---|
| `--config FILE` | | Use this config file instead of `~/.config/quarry/config.toml`. Favourites and themes are then read from next to it. |
| `--list` | `-l` | List saved connections and exit |
| `--save NAME` | | Save this connection under `NAME`. See [Saved connections](/advanced/saved-connections). |
| `--default-config` | | Print the default config file, every option commented, to start customizing from. See [Configuration file](/reference/config). |
| `--setup-llm` | | Choose how `\llm` reaches a model, test it and save it. See [Asking a model for SQL](/guides/ai). |
| `--completions SHELL` | | Print the tab-completion script for `bash`, `zsh`, `fish`, `elvish` or `powershell`. See [Shell completion](/start/installation#shell-completion). |

## Help

| Option | Short | Meaning |
|---|---|---|
| `--help` | | Show help. (`-h` is the host.) |
| `--version` | `-V` | Show the version |

## Exit status

| Status | When |
|---|---|
| `0` | Everything ran |
| `1` | A statement or a special command failed, or quarry couldn't connect |
| `2` | The command-line options were invalid |
