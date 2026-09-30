---
title: 'Special commands'
description: 'Every backslash command, dot command and word command in the quarry REPL, with aliases and options.'
---

# Special commands

Special commands run as soon as you press <kbd>Enter</kbd>, with no `;` needed. Most start with `\`.
On SQLite the dot commands (`.tables`, `.schema`, …) work too, and a few MySQL-style words (`use`,
`source`, `status`, …) are also understood.

`\?` lists the commands available on the current database. `\? word` filters that list.

## Reading this page

- **Aliases** are other names for the same command. Any of them works.
- `[+]` means you can add `+` for more detail: `\dt+`.
- `[pattern]` narrows the list:
  - `*` and `?` are wildcards: `\dt user*`.
  - `schema.name` limits it to one schema, and `schema.*` lists everything in it.
  - Quote a name to match it exactly: `\d "MixedCase"`.
  - On PostgreSQL, unquoted names are folded to lower case.
  - Without a schema, only objects on your search path (PostgreSQL) or in the current database
    (MySQL) are listed.
- **Databases** says where a command is listed in `\?`. Elsewhere, it prints a short note saying the
  database doesn't support it.

## General

| Command | Aliases | Does |
|---|---|---|
| `\? [command]` | `help`, `\h`, `\help`, `.help` | List commands, or filter the list |
| `\q` | `quit`, `exit`, `\quit`, `.quit`, `.exit` | Quit |
| `\! command` | `system` | Run a shell command |
| `\echo text` | | Print text |
| `\e [file]` or `query \e` | `\edit` | Edit the last query, a given query, or a file in `$VISUAL` / `$EDITOR`. The text comes back to the prompt. |
| `\i file` | `source`, `\ir`, `\.`, `.read`, `\include` | Run a file, like `-f`: SQL, plus special commands on their own lines. Stops at the first error, and a `\q` in the file quits. |
| `\history [n]` | | Show the last `n` statements (default 20) |
| `\llm question` | `\ai` | Ask the configured model to write SQL. See [Asking a model for SQL](/guides/ai). |
| `\theme [name]` | | List themes, or switch to one |
| `\R [format]` | `prompt`, `\prompt` | Set the prompt format. With no format, go back to `auto`. |
| `\refresh` | `rehash`, `\#`, `\rehash` | Reload the table and column names used for completion |
| `\tui` | | Open the TUI on this connection |

## Describing the database

| Command | Aliases | Databases | Does |
|---|---|---|---|
| `\l[+] [pattern]` | `\list`, `.databases` | all | List databases. `+` adds size and more. |
| `\d[+] [pattern]` | `describe`, `desc`, `\describe` | all | Without a pattern, list all tables and views. With one, describe each match: columns, indexes, constraints, foreign keys, referencing tables and triggers. `+` adds sizes, comments and view definitions. |
| `\dt[+] [pattern]` | `.tables` | all | List tables. (`.tables` includes views.) |
| `\dv[+] [pattern]` | `.views` | all | List views |
| `\dm[+] [pattern]` | | PostgreSQL | List materialized views |
| `\dE[+] [pattern]` | | PostgreSQL | List foreign tables |
| `\di[+] [pattern]` | `.indexes`, `.indices` | all | List indexes; the pattern matches the index or table name |
| `\ds [pattern]` | | PostgreSQL, MySQL | List sequences (on MariaDB, sequence tables) |
| `\df[+] [pattern]` | | all | List functions and procedures |
| `\dn [pattern]` | | all | List schemas. On SQLite, attached databases. |
| `\du [pattern]` | `\dg` | PostgreSQL, MySQL | List roles or users |
| `\dT [pattern]` | | PostgreSQL | List data types |
| `\dx [pattern]` | | PostgreSQL, MySQL | List extensions (MySQL: plugins) |
| `\dp [pattern]` | `\z` | PostgreSQL, MySQL | List access privileges |
| `\sf[+] function` | | PostgreSQL, MySQL | Show a function's definition |
| `\sv[+] view` | | all | Show a view's definition |
| `.schema [pattern]` | `\schema` | all | Show `CREATE` statements |
| `\s` | `status`, `.status`, `\status` | all | Connection and server status: version, user, uptime, encodings, transaction, read-only, TLS |
| `\conninfo` | | all | Where and how you're connected |

`describe t` and `desc t` describe a table. `DESCRIBE SELECT …` and `DESC t column` go to the server
as SQL, as MySQL users expect.

## Output

| Command | Aliases | Does |
|---|---|---|
| `\x [on\|off\|auto]` | `\expanded` | Vertical output. Without an argument, cycles off → on → auto. |
| `\timing [on\|off]` | `\t` | Show or hide statement timing. On SQLite, `.timer on` also works. |
| `\T [format]` | `\tableformat`, `.mode` | Show or set the [output format](/reference/output-formats) |
| `\pager [command]` | `pager`, `\P` | Turn the pager on, optionally with a command |
| `\nopager` | `nopager` | Turn the pager off |
| `tee [-o] file` | `\tee`, `.output` | Also write every result to a file, appending (`-o` overwrites; `.output` always overwrites). `.output` alone or `.output stdout` stops. |
| `notee` | `\notee` | Stop `tee` |
| `\o [-o] file` | `\once`, `.once` | Write the next result to a file instead of the screen |
| `\\| command` | `\pipe_once` | Pipe the next result to a shell command |
| `query \clip` | | Copy the query (or the last one) to the clipboard |
| `\export format file query` | | Write a query's complete result to a file in any format |

## Running queries

| Command | Aliases | Databases | Does |
|---|---|---|---|
| `\watch [sec] [-c] [query]` or `query \watch [sec]` | `watch` | all | Re-run a query every `sec` seconds (default 2) until <kbd>Ctrl</kbd>+<kbd>C</kbd>. `-c` clears the screen each time. Without a query, watches the last one. |
| `\format [query]` | | all | Pretty-print a query (or the last one) and put it back at the prompt |
| `\explain [analyze] query` | | all | Show the plan as a tree with costs, rows and, with `analyze`, actual times |
| `delimiter string` | `\delimiter` | MySQL | Change the statement terminator, e.g. `delimiter //`. `delimiter ;` restores it. |
| `\readonly [on\|off]` | | all | Toggle [read-only mode](/guides/safety#read-only-mode) |
| `.load path` | `\load` | SQLite | Load a SQLite extension |

## Favourites

| Command | Aliases | Does |
|---|---|---|
| `\f [name [args…]]` | `\n` | List favourites, or run one |
| `\fs name query` | `\ns` | Save a favourite. `$1`…`$9`, `$*` and `${name}` are placeholders. |
| `\fd name` | `\nd` | Delete a favourite |

See [Favourite queries](/advanced/favorites).

## Connection

| Command | Aliases | Does |
|---|---|---|
| `\c [name \| database \| url]` | `\connect`, `use`, `\u`, `.open` | Switch to a saved connection, another database, or another server. Without an argument, show the current connection. |

- `\c prod` opens the saved connection `prod`, with its password command, tunnel and start-up SQL.
  A saved name wins over a database with the same name.
- `\c other_db` switches database on the same server (PostgreSQL reconnects; MySQL runs `USE`).
- `\c postgres://…` connects to a different server.
- On SQLite, `\c file.db` or `.open file.db` opens another file (creating it if needed).
- `use db` is not a command on SQLite; there it goes to the server as SQL.

## Statement terminators

These aren't commands, but they end a statement:

| Ending | Effect |
|---|---|
| `;` | Run the statement |
| `\G` | Run it and show the result vertically |
| `\g` | Run it (like `;`) |
