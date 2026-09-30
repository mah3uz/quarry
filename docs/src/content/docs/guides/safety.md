---
title: Staying safe
description: Destructive-statement confirmation, read-only mode, transactions, and how quarry protects secrets.
---

quarry assumes you'll one day be connected to production by mistake, and tries to make that
survivable.

## Confirmation before destructive statements

Some statements ask before they run:

```
⚠ Destructive statement: DELETE without a WHERE clause affects every row
  DELETE FROM orders
Do you want to proceed? [y/N]
```

Only `y` or `yes` runs it; anything else prints `Aborted.` In the TUI a dialog lists the statements
and why each one is dangerous, and mentions when an open transaction still lets you roll back.

By default these ask:

| Rule | Matches |
|---|---|
| `drop` | `DROP …` |
| `truncate` | `TRUNCATE …` |
| `shutdown` | `SHUTDOWN` |
| `unconditional_update` | `UPDATE` with no `WHERE` |
| `unconditional_delete` | `DELETE` with no `WHERE` |

Change the list with `destructive_warning` in the config. A rule is either a statement's first word
in lower case (`alter`, `grant`, `delete`, …) or one of the two `unconditional_` rules:

```toml
[main]
destructive_warning = ["drop", "truncate", "alter", "unconditional_update", "unconditional_delete"]
```

`destructive_warning = []` turns confirmation off. `delete` and `update` rules also catch those
statements inside `WITH … DELETE` and `EXPLAIN ANALYZE DELETE`.

:::caution
The REPL only asks when you're typing at a terminal. **Scripts (`-e`, `-f`, piped input) never
ask**, so review what a script will do before you run it against real data.
:::

In the REPL, answering no skips just that statement; later statements in the same input still run.

## Read-only mode

Read-only mode refuses anything that could change data or schema. Turn it on with:

- `--readonly` / `-r` on the command line,
- `readonly = true` on a [saved connection](/advanced/saved-connections/),
- `?readonly=true` in a URL (or `?mode=ro` for SQLite),
- **Read-only** in the TUI connection form, or the palette's *Toggle read-only for this connection*,
- `\readonly on` in the REPL (`\readonly off` turns it off again).

It's enforced twice:

1. **By the server.** quarry sets the session read-only: `SET SESSION CHARACTERISTICS AS TRANSACTION
   READ ONLY` on PostgreSQL, `SET SESSION TRANSACTION READ ONLY` on MySQL, and on SQLite it opens the
   file read-only with `PRAGMA query_only`.
2. **By quarry.** Every statement is checked before it's sent. `SELECT`, `WITH`, `SHOW`, `DESCRIBE`,
   `EXPLAIN`, `SET`, `USE` and transaction control are allowed; statements that write are refused:

   ```
   ✗ Read-only mode: statement refused (use \readonly off to allow writes)
   ```

The prompt shows `RO`, and the TUI status bar shows `READ-ONLY`. Table editing is disabled.

## Transactions

quarry doesn't change your database's autocommit behaviour. When you run `BEGIN`, it notices:

- The REPL prompt shows `TX` and the `❯` changes colour. `\s` reports the transaction state.
- The TUI status bar shows `TX`. Quitting asks first and warns that the transaction will be rolled
  back.

## Cancelling

<kbd>Ctrl</kbd>+<kbd>C</kbd> in the REPL, or <kbd>Esc</kbd> in the TUI, cancels a running query **at
the server**, not just in quarry. PostgreSQL gets a cancel request, MySQL a `KILL QUERY`, and SQLite
an interrupt.

## Secrets

- **Passwords are never printed or logged.** Connection URLs shown by quarry leave the password out.
- **History skips secrets.** Statements containing `password`, `identified by`, `secret` or
  `encrypted` are not saved to history or the query log. This is a simple word check: a `\c` URL with
  a password in it *would* be saved, so prefer `~/.pgpass`, `~/.my.cnf` or `password_command`.
- **Files are private.** History, the query log, exports, favourites, credentials and the config are
  written with mode 600.
- **API keys live apart from your config**, in the data directory. See [Asking a model for
  SQL](/guides/ai/#api-keys).

## Related

- [Passwords and secrets](/advanced/passwords/)
- [Configuration file](/reference/config/)
