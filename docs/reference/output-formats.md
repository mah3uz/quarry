---
title: 'Output formats'
description: 'Every table and machine-readable output format, with examples.'
---

# Output formats

Pick a format with `--format` / `-F` on the command line, `\T` in the REPL, or `table_format` in the
config. `\T` alone lists them.

## Table formats

For reading. They respect `max_field_width`, switch to vertical layout when `expanded = "auto"` and
the table is too wide, and print a status line (`3 rows · 0.4 ms`) in the REPL.

| Format | Also accepted | Looks like |
|---|---|---|
| `rounded` (default) | `fancy_grid`, `fancy`, `round` | `╭──┬──╮` with rounded corners |
| `unicode` | `psql_unicode`, `box` | `┌──┬──┐` single lines |
| `double` | `double_grid` | `╔══╦══╗` double lines |
| `ascii` | `grid`, `mysql` | `+----+----+`, like the mysql client |
| `psql` | | Like psql: a `-+-` rule under the header, no outer border |
| `simple` | | A `----` rule under the header |
| `minimal` | | Column gaps and a `─` rule under the header |
| `plain` | | Columns separated by spaces only |
| `markdown` | `github`, `md`, `pipe` | A GitHub-flavoured Markdown table |
| `vertical` | `expanded` | One `name: value` line per column, one block per row |

The same result in four of them:

```
rounded                   ascii                     markdown              psql
╭────────┬────────╮       +--------+--------+       | status | orders |    status | orders
│ status │ orders │       | status | orders |       |--------|-------:|   --------+--------
├────────┼────────┤       +--------+--------+       | new    |    666 |    new    |    666
│ new    │    666 │       | new    |    666 |       | paid   |    667 |    paid   |    667
│ paid   │    667 │       | paid   |    667 |
╰────────┴────────╯       +--------+--------+
```

## Machine formats

For other programs. They're never coloured, cut or turned vertical, and print only the data.

| Format | Also accepted | Output |
|---|---|---|
| `csv` | | Header row, then one line per row. NULL is an empty field. |
| `tsv` | | Header row, tab-separated. Tabs, newlines and backslashes are escaped (`\t`, `\n`, `\\`). NULL is `null_string`. |
| `json` | | An array of objects. Numbers and booleans stay typed, JSON columns are embedded, NULL is `null`. |
| `jsonl` | `jsonlines`, `ndjson` | One JSON object per line |
| `html` | | A `<table>` with `<thead>` and `<tbody>` |
| `sql-insert` | `sql_insert` | `INSERT INTO table (…) VALUES (…);` per row |
| `sql-update` | `sql_update` | `UPDATE table SET … WHERE first_column = …;` per row |

When quarry's output goes to a pipe or a file and you haven't chosen a format, it uses `tsv`.

In the REPL and in scripts, `sql-insert` and `sql-update` use the placeholder table name `table`;
replace it with the real name. The TUI's export fills in the real name.

## SQLite `.mode`

On SQLite, `.mode` accepts sqlite3's names too:

| `.mode` | quarry format |
|---|---|
| `line` | `vertical` |
| `list` | `plain` |
| `tabs` | `tsv` |
| `column` | `simple` |
| `table` | `ascii` |
| `insert` | `sql-insert` |
| `qbox` | `unicode` |

## Vertical output

`\x` cycles vertical output off → on → auto, and `\x on`, `\x off`, `\x auto` set it directly. End a
single statement with `\G` instead of `;` to show just that result vertically:

```
select id, email, name from users where id = 1\G
*************************** 1. row ***************************
   id: 1
email: user1@example.com
 name: User 1
```
