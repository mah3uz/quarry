---
title: 'Using the REPL'
description: 'Running statements, completion, results, history and the everyday special commands.'
---

# Using the REPL

The REPL is quarry's command line. Start it by giving quarry a target without `--tui`:

```sh
quarry postgres://me@localhost/app
```

## The prompt

The default prompt takes two lines and tells you where you are:

```
╭─ PostgreSQL me@localhost:5432 ▸ app  TX  RO  ✓ 12.0 ms
╰─❯
```

- `TX` appears while a transaction is open, and the `❯` changes colour.
- `RO` appears in read-only mode.
- `✓ 12.0 ms` (or `✗`) shows how the last statement went and how long it took.

You can replace it with your own format; see [Prompt, keys and completion](/advanced/customising).

## Running statements

With multi-line mode on (the default), <kbd>Enter</kbd> runs the buffer only when the statement is
complete. Otherwise it starts a new line:

- A statement ends with `;`.
- End it with `\G` instead to show that one result vertically.
- Special commands (`\dt`, `use db`, `describe t`, `status`) run straight away.
- <kbd>Alt</kbd>+<kbd>Enter</kbd> runs whatever is in the buffer, complete or not.

Several statements in one buffer run in order and stop at the first error. Semicolons inside
strings, comments, `$$` blocks and `BEGIN … END` bodies of triggers and procedures don't end the
statement, so MySQL procedures work without changing the delimiter. If you prefer, `delimiter //`
changes it, and `delimiter ;` restores it.

<kbd>F3</kbd> toggles multi-line mode. With it off, <kbd>Enter</kbd> always runs the buffer.

## Completion

A menu of suggestions opens as you type. Take the highlighted one with <kbd>Tab</kbd> or
<kbd>Enter</kbd>, move with <kbd>↑</kbd> / <kbd>↓</kbd>. While the menu is open <kbd>Enter</kbd>
completes, and a second <kbd>Enter</kbd> runs the line. When the highlighted suggestion is exactly
what you have typed there is nothing to complete, so <kbd>Enter</kbd> runs the line at once: `\d`
runs even though `\dt` and `\df` are listed below it.

What you get depends on where the cursor is:

| Where | Suggestions |
|---|---|
| Start of a statement | Statement keywords and special commands |
| After `FROM`, `JOIN`, `INTO`, `UPDATE` | Tables, views, CTE names and schemas |
| In `SELECT`, `WHERE` and other expressions | Columns of the tables in the statement, then aliases, functions and keywords |
| After `alias.`, `table.` or `schema.` | That table's columns, or that schema's objects |
| `SELECT *` then <kbd>Tab</kbd> | The full column list |
| After `JOIN` | Whole join clauses built from foreign keys: `orders o ON o.user_id = u.id` |
| After `::` or `CAST(… AS` | Data types |
| After `\i`, `\o`, `tee`, `.read` and similar | File paths |

Matching is fuzzy: `ui` finds `user_id` (initials of each word), and so does `uid` (letters in
order). Exact prefixes rank first.

Grey text after the cursor is a suggestion from your history. Press <kbd>→</kbd> to accept it.

If completion gets in your way, <kbd>F2</kbd> turns off the context awareness (you get every
keyword, table and column, matched against the word you're typing), and
`complete_while_typing = false` in the config opens the menu only on <kbd>Tab</kbd>.

## Results

Results stream from the server and print as a table:

```
╭─────────┬────────╮
│ status  │ orders │
├─────────┼────────┤
│ paid    │    667 │
├─────────┼────────┤
│ shipped │    667 │
╰─────────┴────────╯
2 rows · 0.57 ms
```

- **Rows** are separated by a line in the boxed formats; `row_lines = false` in the config turns
  that off.
- **Wide results** (wider than the terminal) switch to [vertical layout](/reference/output-formats#vertical-output)
  automatically: one framed record per row. `\x` cycles between on, off and auto; `\x off` keeps
  the table and the pager scrolls it sideways.
- **Long values** are cut at 500 characters (`max_field_width`) and end with `…`.
- **Big results:** past 1000 rows (`row_limit`), quarry asks before fetching the rest. Say no and
  it shows the first 1000 and cancels the query at the server.
- **Output that doesn't fit the screen** goes through a pager, `less -SRXF` by default. `\nopager`
  turns it off, and `\pager cmd` picks another.
- **Formats:** `\T` lists them and `\T markdown` switches. See [Output formats](/reference/output-formats).
- **Timing** is shown after each result. `\timing` toggles it.

Server notices and warnings print before the result. Errors show the SQLSTATE or error number, and
a caret under the failing position when the server reports one:

```
✗ ERROR 42703  column "nmae" does not exist
```

<kbd>Ctrl</kbd>+<kbd>C</kbd> while a query runs cancels it **at the server**: a cancel request on
PostgreSQL, `KILL QUERY` on MySQL, an interrupt on SQLite.

## History

Every statement you run is saved to `~/.local/share/quarry/history.txt` (10,000 entries by default).

- <kbd>Ctrl</kbd>+<kbd>R</kbd> searches it.
- <kbd>↑</kbd> and <kbd>↓</kbd> step through it.
- `\history 20` prints the last 20.

Lines containing `password`, `identified by`, `secret` or `encrypted` are never saved.

## Everyday special commands

Special commands start with `\` and run immediately. On SQLite the familiar dot commands work too
(`.tables`, `.schema`, `.mode`). The ones you'll use most:

| Command | Does |
|---|---|
| `\?` | List every command. `\? export` filters the list. |
| `\dt`, `\dv`, `\di`, `\df` | List tables, views, indexes, functions. Add a pattern: `\dt user*` |
| `\d name` | Describe a table: columns, indexes, keys, triggers. `\d+` adds sizes and comments. |
| `\l` | List databases |
| `\c db` / `use db` | Switch database |
| `\x`, `\T fmt` | Vertical output, output format |
| `\e` | Edit the last query in `$EDITOR` |
| `\format` | Pretty-print the last query |
| `\explain query` | Show the plan as a tree. `\explain analyze query` runs it. |
| `\watch 5 query` | Re-run a query every 5 seconds, until <kbd>Ctrl</kbd>+<kbd>C</kbd> |
| `\s` | Connection and server status |
| `\q` | Quit (<kbd>Ctrl</kbd>+<kbd>D</kbd> works too) |

The full list, with every alias and option, is in [Special commands](/reference/special-commands).

### Editing in your editor

`\e` opens the last query you ran in `$VISUAL` or `$EDITOR` (`vi` by default). When you save and
quit, the text comes back to the prompt for you to run. Type `query \e` to open a query you've just
written instead, and `\e file.sql` to edit a file.

### Sending results somewhere else

| Command | Result goes to |
|---|---|
| `\o file` | The next result goes to `file` instead of the screen |
| `\| command` | The next result is piped to a shell command, e.g. `\| wc -l` |
| `tee file` | Every result also goes to `file` until `notee` (`-o` overwrites instead of appending) |
| `\export csv file query` | The complete result of `query`, written to `file` in any format |
| `query \clip` | The query text is copied to the clipboard |

## Switching to the TUI

`\tui` opens the full-screen interface on the same connection. Quitting the TUI ends quarry.

## Related

- [Keyboard shortcuts](/reference/keys#repl)
- [Scripts, exports and pipes](/guides/scripting)
- [Prompt, keys and completion](/advanced/customising)
