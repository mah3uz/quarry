---
title: Connecting to a database
description: URLs, SQLite files, flags, defaults and switching databases.
---

You tell quarry where to connect with one argument, the **target**. It can be a URL, a SQLite file,
the name of a saved connection, or just a database name with flags.

```sh
quarry postgres://me@db.example.com/app   # a URL
quarry data.db                             # a SQLite file
quarry prod                                # a saved connection
quarry app me                              # pgcli style: database, then user
quarry                                     # no target: the TUI's connection manager
```

## URLs

```
scheme://user:password@host:port/database?param=value
```

| Database | Schemes |
|---|---|
| PostgreSQL | `postgres://`, `postgresql://`, `pg://`, `pgsql://` |
| MySQL / MariaDB | `mysql://`, `mariadb://`, `mysqlx://` |
| SQLite | `sqlite:` or `file:` (see [SQLite](#sqlite) below) |

Every part except the scheme is optional:

| Part | Default |
|---|---|
| user | `$USER` (then `$USERNAME`, then `root`) |
| host | `localhost` |
| port | 5432 for PostgreSQL, 3306 for MySQL |
| database | PostgreSQL: the user name. MySQL: none (pick one later with `use`) |

Special characters in the user or password must be percent-encoded: `p@ss` becomes `p%40ss`.

### URL parameters

| Parameter | Meaning |
|---|---|
| `sslmode` (or `ssl_mode`, `ssl`) | TLS mode: `disable`, `prefer`, `require`, `verify-ca`, `verify-full` |
| `sslrootcert` (or `ssl_ca`, `sslca`) | CA certificate file |
| `sslcert`, `sslkey` (or `ssl_cert`, `ssl_key`) | Client certificate and key |
| `host` | Host name, or a socket path if it starts with `/` |
| `socket` (or `unix_socket`) | Unix socket path |
| `port`, `user`, `password`, `dbname` (or `database`) | The same as the URL parts |
| `connect_timeout` | Seconds to wait for the connection (default 15) |
| `readonly` (or `read_only`) | `1`, `true`, `yes` or `on` opens the connection read-only |
| `ssh` | Tunnel through SSH: `[user@]host[:port]` |
| `application_name`, `options` | Passed to PostgreSQL |
| `charset` | MySQL character set (runs `SET NAMES`) |

```sh
quarry "postgres://me@db.example.com/app?sslmode=verify-full&application_name=quarry"
```

### Unix sockets

Put the socket path in the host position (percent-encoded), or use the `socket` parameter or `-S`:

```sh
quarry "postgres://me@%2Fvar%2Frun%2Fpostgresql/app"
quarry mysql://root@localhost/shop?socket=/run/mysqld/mysqld.sock
quarry -S /run/mysqld/mysqld.sock -u root shop
```

## SQLite

A target is treated as a SQLite database when it:

- is `:memory:`,
- ends in `.db`, `.sqlite`, `.sqlite3`, `.db3`, `.s3db` or `.sl3`, or
- is an existing file that starts with the SQLite header.

```sh
quarry data.db                     # relative path; created if it doesn't exist
quarry ~/notes/notes.sqlite
quarry :memory:                    # a throwaway in-memory database
quarry sqlite:///abs/path/app.db   # URL form, absolute path
quarry "sqlite:app.db?mode=ro"     # read-only
```

A file with a SQLite extension is **created** if it doesn't exist, unless you open it read-only.

## Flags

Instead of a URL you can give the parts as flags. pgcli-style and mycli-style short flags both work:

| Flag | Meaning |
|---|---|
| `-h`, `--host` | Host |
| `-p`, `-P`, `--port` | Port |
| `-u`, `-U`, `--user` | User |
| `-d`, `-D`, `--database` | Database |
| `-S`, `--socket` | Unix socket |
| `--backend` | `postgres`, `mysql` or `sqlite` |

```sh
quarry -h db.example.com -u me -d app
quarry --backend mysql -h 127.0.0.1 -u root -d shop
```

Flags override the same part of a URL or saved connection. Without `--backend`, quarry assumes
PostgreSQL unless the port is 3306 or the socket path contains `mysql`.

:::note
`-h` means **host**, as in psql and mysql. For help, use `--help`.
:::

### pgcli style

If the target isn't a URL, file or saved connection name, quarry treats it as a database name and
an optional second argument as the user:

```sh
quarry app me     # database "app", user "me", on localhost
```

## Defaults from the environment

For fields you haven't set, quarry reads the same environment variables as the standard clients:

- **PostgreSQL:** `PGHOST`, `PGPORT`, `PGUSER`, `PGDATABASE`, `PGPASSWORD`, and `~/.pgpass`
  (or `$PGPASSFILE`).
- **MySQL:** `MYSQL_HOST`, `MYSQL_TCP_PORT`, `MYSQL_UNIX_PORT`, `MYSQL_PWD`, and the `[client]` and
  `[mysql]` sections of `/etc/my.cnf`, `/etc/mysql/my.cnf` and `~/.my.cnf`.

Anything you give explicitly always wins. [Passwords and secrets](/advanced/passwords/) explains the
full order.

## Passwords

quarry looks for a password in the URL, the environment, `~/.pgpass` or `~/.my.cnf`, and a saved
connection's `password_command`. If the server still rejects the login and you're at a terminal, it
asks you (up to three times).

- `-W` / `--password` always asks before connecting.
- `-w` / `--no-password` never asks, which is useful in scripts.

## Switching databases

In the REPL, `\c` (or `\connect`, `use`) switches without restarting:

```
\c other_db                           # same server, another database
use other_db                          # MySQL style
\c prod                               # a saved connection
\c postgres://me@other-host/app       # a different server
\c                                    # show where you are connected
```

A saved connection name wins over a database with the same name. On SQLite, `\c file.db` (or
`.open file.db`) opens another file.

## Starting without a target

Run `quarry` with no target and no connection flags and the TUI opens on its connection manager,
where you can pick a saved connection or fill in a new one. In a script (with `-e`, `-f` or piped
input), a missing target is an error instead.

## Related

- [Saved connections](/advanced/saved-connections/)
- [TLS and SSH tunnels](/advanced/tls-ssh/)
- [Command-line options](/reference/cli/)
