---
title: Saved connections
description: Give connections names, and attach read-only mode, SSH tunnels, start-up SQL and password commands to them.
---

A saved connection is a name for a target, stored in your config. Open it with `quarry NAME`, or
pick it in the TUI's connection manager.

## Saving one

The quickest way is `--save` on a command that connects:

```sh
quarry postgres://me@db.example.com/app --save app
quarry app          # next time
```

Or in the TUI: <kbd>Ctrl</kbd>+<kbd>O</kbd>, fill in the form on the right with a **Name**, and leave
**Save** set to yes.

List what you have with `quarry --list` (or `-l`):

```
$ quarry --list
app    postgres://me@db.example.com/app
local  mysql://root@127.0.0.1/shop (read-only)
```

## Writing them by hand

Saved connections live in `~/.config/quarry/config.toml`, one table per name:

```toml
[connections.prod]
url = "postgres://deploy@db.internal:5432/app?sslmode=verify-full"
password_command = "pass show db/prod"
ssh = "deploy@bastion.example.com:22"
readonly = true
color = "red"
init_commands = ["SET search_path TO app, public", "SET statement_timeout = '30s'"]
```

| Key | Meaning |
|---|---|
| `url` | Any target quarry understands: a URL (with query parameters), or a SQLite file |
| `password_command` | A shell command whose output is the password. See [Passwords](/advanced/passwords/#password_command). |
| `ssh` | Tunnel through SSH: `[user@]host[:port]`. See [SSH tunnels](/advanced/tls-ssh/#ssh-tunnels). |
| `readonly` | `true` opens the connection in [read-only mode](/guides/safety/#read-only-mode) |
| `color` | Tag colour in the TUI, e.g. `"red"` for production: the connection's name in the explorer and its label in the status bar are drawn in it. `"#rrggbb"` or a colour name. |
| `init_commands` | SQL to run right after connecting, in order |

Flags on the command line still apply on top: `quarry prod -d other_db` connects to `other_db`, and
`--init-command` adds to `init_commands`.

## What `--save` stores

- The target as you typed it, if it contains `:` or `.` (a URL or a file name). Otherwise, the URL
  quarry built from your flags.
- `readonly` and `--ssh`.

**Passwords are never saved.** If the URL contains one (`me:secret@…` or `?password=`), quarry
removes it and says so; give the password a home in a
[`password_command`](/advanced/passwords/#password_command), `~/.pgpass` or `~/.my.cnf`. `--save` also
doesn't store `--ssh-key`, TLS flags or `--init-command`. Add those by hand if you need them.

If you write a password into a `url` by hand, keep the file private (`chmod 600`); quarry warns at
start-up when it isn't.

Saving rewrites `config.toml`, so comments you added are lost. The previous file is kept as
`config.toml.bak`.

## Deleting

Delete the `[connections.NAME]` table from the config, or press <kbd>d</kbd> on it in the TUI's
connection manager. The TUI doesn't ask for confirmation, but the previous config is kept as
`config.toml.bak`.

## Using them

- `quarry NAME` in a terminal. Add `--tui` to open the TUI.
- `\c NAME` in the REPL, to switch to it without restarting.
- The TUI's connection manager (<kbd>Ctrl</kbd>+<kbd>O</kbd>), where read-only connections are
  marked `ʀ`.
