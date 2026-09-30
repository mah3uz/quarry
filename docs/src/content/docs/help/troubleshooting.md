---
title: Troubleshooting
description: Fixes for common problems connecting, typing, displaying results and using the TUI.
---

## Connecting

**`cannot understand '…': expected postgres://, mysql://, sqlite: URL or a SQLite file path`**

quarry didn't recognise the target. Check the scheme (`postgres://`, `mysql://`), or, for a SQLite
file without a usual extension, use `sqlite:path/to/file`.

**`unsupported scheme '…'`**

Only PostgreSQL, MySQL / MariaDB and SQLite URLs are supported.

**It connects to PostgreSQL when I meant MySQL**

With only flags, quarry assumes PostgreSQL unless the port is 3306 or the socket path contains
`mysql`. Add `--backend mysql`, or use a `mysql://` URL.

**The password is wrong, but I set it in `~/.pgpass`**

A `PGPASSWORD` in your environment is used first. Also check the file's permissions: quarry ignores
`~/.pgpass` if it's readable by others (`chmod 600 ~/.pgpass`). For socket connections the host to
match is `localhost`.

**My script hangs**

It's probably waiting for a password. Add `-w` so it fails instead, and provide the password with
`~/.pgpass`, `~/.my.cnf` or an environment variable.

**``cannot run `ssh` (is OpenSSH installed?)``**

SSH tunnels use the system `ssh` binary. Install OpenSSH, and check that `ssh bastion.example.com`
works on its own first.

**TLS certificate errors with `verify-full` through an SSH tunnel**

Through a tunnel quarry connects to `127.0.0.1`, so the host name check fails. Use `verify-ca`
instead. See [TLS and SSH tunnels](/advanced/tls-ssh/).

**`\c prod` connects to a database called "prod"**

`\c` doesn't look up saved connections. Use the URL, or start quarry with `quarry prod`.

## Typing and running

**<kbd>Enter</kbd> adds a new line instead of running**

In multi-line mode a statement runs only once it ends with `;` (or `\G`). Type the `;`, press
<kbd>Alt</kbd>+<kbd>Enter</kbd> to run it anyway, or press <kbd>F3</kbd> to turn multi-line mode off.

**Completion doesn't suggest my new table**

The list is loaded when you connect and reloaded after `CREATE`, `ALTER` and `DROP` you run
yourself. If someone else changed the schema, run `\refresh`.

**Completion doesn't offer columns after `SELECT`**

Columns come from the tables in the statement. Write the `FROM` part first, or use an alias:
`select * from users u where u.` lists the columns of `users`.

**`Read-only mode: statement refused`**

The connection is read-only (`--readonly`, `readonly = true`, or `\readonly on`). Run
`\readonly off` if you really mean to write.

## Output

**Results open in `less` and I don't want that**

`\nopager` for this session, or `enable_pager = false` in the config.

**Long values end with `…`**

They're cut at `max_field_width` (500). Set `max_field_width = 0` for no limit, or use `\G` or a
machine format such as `\T json`.

**`The result has more than 1000 rows. Fetch and show all of them?`**

That's `row_limit`. Answer `y`, raise it in the config, pass `--row-limit 0`, or add a `LIMIT`.

**The output in my script has no colours, or looks like TSV**

When stdout isn't a terminal quarry prints TSV without colour. Pass `-F` to choose a format.

**Characters look broken or boxes don't line up**

Use a terminal font with box-drawing characters (most monospace fonts have them), or pick an ASCII
format with `\T ascii` or `table_format = "ascii"`.

**Colours look wrong**

Your terminal may not report 24-bit colour. Try `COLORTERM=truecolor quarry …`, or the `ansi`
theme, which uses your terminal's own palette.

## The TUI

**<kbd>Ctrl</kbd>+<kbd>Y</kbd> undoes my undo instead of opening themes**

In the editor <kbd>Ctrl</kbd>+<kbd>Y</kbd> is redo. Press <kbd>Esc</kbd> to leave the editor
first, or use *Switch theme…* in the palette.

**My theme setting in the config is ignored in the TUI**

A theme you picked in the TUI is remembered in `~/.local/share/quarry/ui-state.toml` and wins.
Delete that file, or pick the theme again in the TUI.

**The query stopped at 200,000 rows**

That's the TUI's limit. Use a `LIMIT` or `WHERE`, or export with `\export` in the REPL or with
`-e … -F csv` in a script.

**"Editing needs a primary key on this table"**

Table editing identifies rows by primary key. Use an `UPDATE` in a query tab instead.

**`\theme` in the editor says it's only available in the CLI**

Commands that change the REPL session (`\theme`, `\x`, `\T`, …) don't apply in the TUI. The
database-describing ones (`\dt`, `\d`, `\l`, …) do.

## Still stuck?

Run with a fresh configuration to rule out a setting:

```sh
QUARRY_CONFIG_DIR=$(mktemp -d) QUARRY_DATA_DIR=$(mktemp -d) quarry …
```
