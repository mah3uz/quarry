---
title: 'Passwords and secrets'
description: 'Where quarry finds passwords, in what order, and how to keep them out of your config.'
---

# Passwords and secrets

quarry looks for a password in several places, the same ones psql and mysql use, so the setup you
already have usually just works.

## Where passwords come from

**PostgreSQL**, first match wins:

1. The prompt, if you pass `-W` / `--password`
2. The URL: `postgres://me:secret@host/db` or `?password=`
3. `PGPASSWORD`
4. `~/.pgpass` (or the file named by `PGPASSFILE`)
5. The saved connection's `password_command`
6. A prompt, if the server rejects the login and you're at a terminal

**MySQL / MariaDB**, first match wins:

1. The prompt, if you pass `-W`
2. The URL
3. `MYSQL_PWD`
4. `password` in the `[client]` or `[mysql]` section of `~/.my.cnf`
5. The saved connection's `password_command`
6. A prompt, if the server rejects the login and you're at a terminal

SQLite has no passwords.

::: info
Environment variables and password files are checked **before** a saved connection's
`password_command`. If `PGPASSWORD` is set in your shell, it wins.
:::

## Prompting

- When the server rejects the login and quarry is attached to a terminal, it asks
  `Password for me@host:` up to three times.
- `-W` asks before the first attempt.
- `-w` / `--no-password` never asks. Use it in scripts and cron jobs so they fail instead of
  waiting.

## `~/.pgpass`

The standard PostgreSQL password file. One line per server:

```
# host:port:database:user:password
db.example.com:5432:app:me:s3cret
*:*:*:deploy:another-secret
```

- `*` matches anything, and the first matching line wins.
- A backslash escapes `:` or `\` inside a field. The password is the rest of the line, so it may
  contain colons.
- For socket connections, the host to match is `localhost`.
- The file must be private. quarry ignores it, with a warning, if it's readable by group or others:
  `chmod 600 ~/.pgpass`.

## `~/.my.cnf`

quarry reads `/etc/my.cnf`, `/etc/mysql/my.cnf` and `~/.my.cnf`, in that order, with later files
winning. From the `[client]` and `[mysql]` sections it takes `user`, `password`, `host`, `port`,
`socket` and `database`:

```ini
[client]
user = me
password = "s3cret"
host = 127.0.0.1
```

`!include` lines are ignored, and so is `~/.mylogin.cnf`.

## `password_command`

For saved connections, the best option: the password comes from your password manager and never
touches a file.

```toml
[connections.prod]
url = "postgres://deploy@db.internal/app"
password_command = "pass show db/prod"
```

The command runs with `sh -c`, and its output, minus the trailing newline, is the password. It only
runs when no other source has already supplied a password. If it exits with an error, connecting
fails. Some examples:

```toml
password_command = "op read op://Private/prod-db/password"        # 1Password
password_command = "security find-generic-password -s prod-db -w"  # macOS Keychain
password_command = "secret-tool lookup service prod-db"            # GNOME Keyring
```

## Keeping secrets out of history

quarry doesn't save statements containing `password`, `identified by`, `secret` or `encrypted`, or
a URL with a password in it such as `\c postgres://me:pw@host/db`, to its history or query log.

Connection URLs printed by quarry (`--list`, errors, `\c` messages) never include the password.
