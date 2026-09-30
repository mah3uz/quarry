---
title: Environment variables
description: Every environment variable quarry reads.
---

## quarry

| Variable | Meaning |
|---|---|
| `QUARRY_CONFIG_DIR` | Use this directory for `config.toml`, `favorites.toml` and `themes/` |
| `QUARRY_DATA_DIR` | Use this directory for history, the query log, credentials and TUI state |
| `XDG_CONFIG_HOME` | Config goes in `$XDG_CONFIG_HOME/quarry` (absolute paths only; default `~/.config`) |
| `XDG_DATA_HOME` | Data goes in `$XDG_DATA_HOME/quarry` (absolute paths only; default `~/.local/share`) |

## PostgreSQL

Used for anything not given in the target or flags.

| Variable | Meaning |
|---|---|
| `PGHOST` | Host, or a socket directory if it starts with `/` |
| `PGPORT` | Port |
| `PGUSER` | User |
| `PGDATABASE` | Database |
| `PGPASSWORD` | Password |
| `PGPASSFILE` | Password file instead of `~/.pgpass` |

## MySQL / MariaDB

| Variable | Meaning |
|---|---|
| `MYSQL_HOST` | Host |
| `MYSQL_TCP_PORT` | Port |
| `MYSQL_UNIX_PORT` | Socket path |
| `MYSQL_PWD` | Password |

Other defaults come from `~/.my.cnf`. See [Passwords and secrets](/advanced/passwords/#mycnf).

## Language models

| Variable | Meaning |
|---|---|
| `ANTHROPIC_API_KEY` | Anthropic API key, used before a saved one |
| `ANTHROPIC_AUTH_TOKEN` | Anthropic bearer token, used if no API key is set |
| `ANTHROPIC_BASE_URL` | Send Anthropic requests to this URL (e.g. a gateway) |

`OPENAI_API_KEY` is intentionally **not** read. See [API keys](/guides/ai/#api-keys).

## Terminal and tools

| Variable | Meaning |
|---|---|
| `NO_COLOR` | Any non-empty value turns colour off |
| `COLORTERM` | `truecolor` or `24bit` enables 24-bit colour |
| `TERM` | Used to detect colour support. `dumb` turns colour off. |
| `PAGER` | Pager, when the config doesn't set one |
| `LESS` | If unset, quarry sets it to `-SRXF` for the pager |
| `VISUAL`, `EDITOR` | Editor for `\e`, in that order (default `vi`) |
| `USER`, `USERNAME` | Default user name |
