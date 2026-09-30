---
title: Favourite queries
description: Save queries by name, run them with arguments, and use them from the REPL and the TUI.
---

Favourites are named queries you run often, with optional placeholders for the parts that change.

## Save, run, delete

```
\fs active_users select id, email from users where last_seen > now() - interval '7 days'
\f active_users
\fd active_users
```

| Command | Does |
|---|---|
| `\fs name query` | Save (or overwrite) a favourite |
| `\f` | List all favourites with their SQL |
| `\f name [args…]` | Run a favourite |
| `\fd name` | Delete a favourite |

`\n`, `\ns` and `\nd` are aliases, for anyone coming from mycli.

When a favourite runs, quarry prints `> ` and the expanded SQL first (turn that off with
`less_chatty`). Read-only mode and destructive-statement confirmation apply as usual. A favourite can
contain several statements.

## Placeholders

| Placeholder | Replaced with |
|---|---|
| `$1` … `$9` | The positional arguments |
| `$*` | All positional arguments, joined with `, ` |
| `${name}` | The value of a `--name=value` argument |

Arguments are split like a shell command line, so use quotes to group words:

```
\fs orders_for select * from orders where user_id = $1 and status = '$2'
\f orders_for 42 paid

\fs by_ids select * from users where id in ($*)
\f by_ids 1 2 3

\fs search select * from users where email like '%${q}%' limit ${n}
\f search --q=example.com --n=20
```

:::caution
Values are inserted **exactly as written**, without quoting or escaping. Put quotes in the favourite
where the value is a string (as in `'$2'` above), and only run favourites with arguments you trust.
:::

Using a placeholder you didn't supply is an error (`favorite 'x' needs 2 positional arguments`), and
so is passing extra arguments to a favourite that doesn't use `$*`.

## In the TUI

- <kbd>Ctrl</kbd>+<kbd>S</kbd> in a query tab saves the selection, or the whole editor, as a
  favourite.
- Your favourites are listed in the command palette (<kbd>Ctrl</kbd>+<kbd>P</kbd>) and under
  **Favorite queries…**. Choosing one opens it in a new tab for you to run.
- Favourites are also offered by completion in the editor.

Deleting a favourite is done from the REPL with `\fd`.

## Where they're stored

Favourites are saved in `favorites.toml`, next to your `config.toml` (so usually
`~/.config/quarry/favorites.toml`), readable only by you:

```toml
[queries]
active_users = "select id, email from users where last_seen > now() - interval '7 days'"
by_ids = "select * from users where id in ($*)"
```

You can edit the file by hand, or share it with your team.
