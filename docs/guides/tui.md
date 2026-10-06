---
title: 'Using the TUI'
description: 'The explorer, query editor, results grid, tabs, database consoles, mouse and command palette.'
---

# Using the TUI

The TUI is quarry's full-screen interface. Open it in any of these ways:

```sh
quarry --tui postgres://me@localhost/app   # or -T
quarry                                     # no target: starts on the connection list
```

From the REPL, `\tui` opens it on the current connection.

Two keys get you everywhere: <kbd>Ctrl</kbd>+<kbd>P</kbd> opens the command palette, which lists
every action by name, and <kbd>F1</kbd> lists every shortcut (type to filter it).

## The layout

<Terminal
  title="quarry --tui shop.db"
  capture="tui"
  label="The quarry TUI: the explorer on the left, a query editor and a results grid on the right, and a status bar"
/>

- **Header:** your tabs, each with a close button, then **+** for a new query tab. On the right,
  the theme (click to change it) and the command palette.
- **Explorer** (left): connections, databases, schemas, tables, views, functions and columns.
  <kbd>Ctrl</kbd>+<kbd>B</kbd> hides it.
- **Main area:** the active tab. A query tab has an editor on top and results below.
- **Status bar:** the mode (`EDITOR`, `RESULTS`, `EXPLORER`, or `NORMAL`/`INSERT` in
  [vim mode](/advanced/keybindings#vim-mode)), the connection and database (click to open
  Connections), badges for an open transaction (`TX`), `READ-ONLY`, `ssh` and `TLS`, hints for the
  keys you can use now, your position in the results, and the time.

Move focus with <kbd>F6</kbd> (explorer → editor → results) or a click. Resize the explorer or the
editor/results split by dragging the border between them (or <kbd>Ctrl</kbd>+<kbd>↑</kbd> /
<kbd>↓</kbd> for the split).

The icons come from a Nerd Font. Without one, set `icons = "unicode"`; see
[Icons](/advanced/customising#icons).

## Connecting

<kbd>Ctrl</kbd>+<kbd>O</kbd> opens **Connections**, a list of your saved connections. Each shows
its type and address; a check mark means it's already open.

- <kbd>Enter</kbd> or a click connects. Choosing one that's already open takes you to it instead of
  opening it twice.
- <kbd>n</kbd> (or **+ New connection**) opens a short form. Paste a URL, or pick the type and fill
  in host, port, user, password and database. Give it a name under **Save as** to save it in your
  config; leave the name empty to connect just this once. The password is never saved.
- <kbd>d</kbd> deletes a saved connection. <kbd>Esc</kbd> goes back, then closes.

You can have several connections open at once. Each gets its own tree in the explorer, and each
tab belongs to one connection. Every connection uses two sessions, one for your queries and one for
the explorer and table views, so the explorer keeps working while a long query runs.
<kbd>Ctrl</kbd>+<kbd>X</kbd> on a connection in the explorer disconnects it.

## Writing and running queries

Type SQL in the editor, then run it with the **▶ Run** button on the editor's border, or a key:

| Key | Runs |
|---|---|
| <kbd>Ctrl</kbd>+<kbd>Enter</kbd> (also <kbd>Ctrl</kbd>+<kbd>E</kbd>, <kbd>Alt</kbd>+<kbd>Enter</kbd>) | The selection, or the statement under the cursor |
| <kbd>F5</kbd> or <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Enter</kbd> | Everything in the editor |
| <kbd>F7</kbd> / <kbd>Shift</kbd>+<kbd>F7</kbd> | Explain / explain analyze |
| <kbd>Esc</kbd>, <kbd>Ctrl</kbd>+<kbd>C</kbd> or **■ Stop** while running | Cancel at the server |

Statements run in order and stop at the first error. Each statement that returns rows gets its own
result tab; switch with <kbd>[</kbd> and <kbd>]</kbd> or a click. The **Messages** tab
(<kbd>m</kbd>) logs what happened, with row counts, notices and errors. After an error, the failing
position is marked in the editor.

The editor completes as you type: keywords, tables after `FROM`, columns of the tables in the
statement, joins, functions. <kbd>Ctrl</kbd>+<kbd>Space</kbd> asks for it, and you can pick with
the mouse too. It also has bracket matching, auto-closing quotes and brackets, undo and redo, and
<kbd>Ctrl</kbd>+<kbd>/</kbd> to toggle comments. <kbd>Alt</kbd>+<kbd>F</kbd> formats the whole
buffer.

Prefer vim keys? Set `vi = true`: see [vim mode](/advanced/keybindings#vim-mode).

Special commands that describe the database (`\dt`, `\d users`, `\l`, `\df`, `\s`, …) also work in
the editor; their output lands in the results grid. `\llm` asks a model to write SQL; see
[Asking a model for SQL](/guides/ai).

## Query consoles for a database

A query tab can belong to one database (or, on PostgreSQL, one schema), like a console in an IDE.
Select a database, schema, or anything inside one in the explorer and press <kbd>c</kbd> (or
<kbd>Ctrl</kbd>+<kbd>T</kbd> while the explorer has focus). The new tab is named after it, and its
border shows where it runs, for example `local-mysql ▸ shop`.

In that tab:

- Queries run in that database, so you can write `select * from users` without `shop.users`.
  quarry switches the session there before each run (`USE shop` on MySQL; on PostgreSQL the schema
  goes first on the `search_path`). If the switch fails, nothing runs.
- Completion suggests that database's tables and columns first.
- New tabs you open from it start in the same place.

Tabs on the same connection can work in different databases side by side. Another PostgreSQL
database can't be switched to in the same session, so a console for one opens its own connection,
named like `local-postgres/analytics`, and reuses it next time.

On MySQL, every query tab remembers the database it was opened in, so running a console in `shop`
doesn't move your other tabs.

## The results grid

The grid handles large results: rows appear as they arrive, and a query stops fetching at 200,000
rows (the Messages tab says so).

| Key | Does |
|---|---|
| <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd> or arrows | Move (<kbd>Shift</kbd> extends the selection) |
| <kbd>g</kbd> / <kbd>G</kbd>, <kbd>0</kbd> / <kbd>$</kbd> | First / last row, first / last column |
| <kbd>v</kbd> / <kbd>V</kbd> | Select a block / whole rows |
| <kbd>Enter</kbd> or double-click | View the cell, with JSON pretty-printed, and the whole row |
| <kbd>y</kbd> / <kbd>Y</kbd> | Copy the selection as TSV / copy rows with a header |
| <kbd>/</kbd>, <kbd>n</kbd>, <kbd>N</kbd> | Search the results |
| <kbd>&lt;</kbd> <kbd>&gt;</kbd> <kbd>=</kbd> | Narrow, widen or auto-fit the column |
| <kbd>Ctrl</kbd>+<kbd>X</kbd> | Export to a file |

**Export** writes the rows in the grid to a file, choosing the format from the extension: `.csv`,
`.tsv`, `.json`, `.jsonl`, `.md`, `.html`, `.sql` (INSERT statements) or `.txt`.

The palette also has **Copy results as CSV / JSON / Markdown / SQL INSERT**, which copy the whole
result to the clipboard. <kbd>y</kbd> and <kbd>Y</kbd> copy just the selection.

## The explorer

| Key | Does |
|---|---|
| <kbd>Enter</kbd> on a table or view | Browse its rows in a [table view](/guides/editing-data) |
| <kbd>Enter</kbd> on a function | Show its definition |
| <kbd>c</kbd> | A [query console](#query-consoles-for-a-database) for the database or schema you're in |
| <kbd>s</kbd> | Structure: columns, indexes, foreign keys, referencing tables, constraints, triggers, DDL |
| <kbd>i</kbd> | Insert the name into the editor |
| <kbd>g</kbd> then <kbd>s</kbd> <kbd>i</kbd> <kbd>u</kbd> <kbd>d</kbd> <kbd>c</kbd> <kbd>x</kbd> <kbd>n</kbd> | Write a SELECT, INSERT, UPDATE, DELETE, CREATE, DROP or COUNT for this table into an editor. Nothing runs until you run it. |
| <kbd>/</kbd> | Filter the tree |
| <kbd>r</kbd> | Reload the schema |
| <kbd>n</kbd> | Open Connections |

The current database is highlighted. On MySQL, the other databases load when you open them (or
when you type `thatdb.` in the editor), and then complete too. On PostgreSQL the explorer also lists
the server's other databases; <kbd>Enter</kbd> on one switches the connection to it, <kbd>c</kbd>
opens a console on it. System schemas are hidden. The schema reloads by itself after statements
such as `CREATE`, `ALTER` and `DROP`.

<kbd>Ctrl</kbd>+<kbd>G</kbd> jumps to any table by fuzzy name, across all open connections.

## Other tabs

| Tab | Opened with | Shows |
|---|---|---|
| Query | <kbd>Ctrl</kbd>+<kbd>T</kbd>, the **+** button | Editor and results |
| Table | <kbd>Enter</kbd> on a table | Rows, page by page, with filters, sorting and edits |
| Structure | <kbd>s</kbd> in the explorer | Columns, indexes, keys, constraints, triggers, DDL |
| Explain | <kbd>F7</kbd> | The plan as a tree, with bars for cost or time, and details per node |
| Activity | Palette → *Server activity* | Live sessions on a PostgreSQL or MySQL server, refreshed every 2 s. <kbd>K</kbd> kills one. |
| History | <kbd>Ctrl</kbd>+<kbd>R</kbd> | Past statements; type to filter, <kbd>Enter</kbd> (or click it twice) opens one in a new tab |

Close a tab with <kbd>Ctrl</kbd>+<kbd>W</kbd>, its close button, or a middle click. Move between
tabs with <kbd>Alt</kbd>+<kbd>←</kbd> / <kbd>→</kbd>, <kbd>Alt</kbd>+<kbd>1</kbd>…<kbd>9</kbd>, or
a click.

## Using the mouse

The mouse works everywhere unless you set `mouse = false`:

- **Click** to focus a pane, pick a tab or result set, place the cursor, select a cell, open a
  tree item, or press a button (tab close, **+**, **▶ Run**, the theme and palette buttons, the
  connection in the status bar).
- **Drag** to select text in the editor or cells in the grid, or drag a border to resize the
  explorer or the editor/results split.
- **Scroll** the editor, grid, messages and any list. In lists (palette, completion, History,
  Explain) the wheel moves the selection.
- **Dialogs:** click an item or button to choose it; click outside a dialog to close it.
- **Middle click** a tab to close it; **double-click** a cell to view it; click a column header in
  a table view to sort.

## Keys and customising

- [Key bindings and vim mode](/advanced/keybindings): rebind the app-wide and pane keys under `[keys]`, and
  edit SQL with vim keys.
- [Keyboard shortcuts](/reference/keys#tui): every key, including the ones inside each pane.
- [Themes](/advanced/themes): <kbd>Ctrl</kbd>+<kbd>Y</kbd> previews themes live. `transparent =
  true` lets a translucent terminal show through.

## Favourites and files

- <kbd>Ctrl</kbd>+<kbd>S</kbd> in a query tab saves the selection (or the whole editor) as a
  [favourite query](/advanced/favorites). Saved favourites appear in the palette.
- **Open SQL file…** and **Save editor to file…** are in the palette.

## Transactions

Run `BEGIN` in the editor to start a transaction; the `TX` badge appears. **Commit transaction** and
**Rollback transaction** are in the palette, or type `COMMIT` / `ROLLBACK`. Quitting with an open
transaction asks first, and the transaction is rolled back.

## Things to know

- Your query tabs and what you typed in them come back the next time you open the same connection.
  They are kept per connection, saved as you work, so a closed terminal doesn't lose them. Empty
  tabs, results, and table and structure tabs aren't kept. The text is stored in `ui-state.toml`
  (see [Files](/reference/files)), readable only by you.
- Quitting asks for confirmation if a query is running, a transaction is open, or a table view has
  unapplied edits.
- `--theme`, `--icons` and `--no-color` apply to the TUI too. After `\tui`, the TUI keeps the
  REPL's theme.
- `row_limit`, `table_format`, `timing` and the pager settings apply to the REPL, not the TUI.

## Related

- [Browsing and editing tables](/guides/editing-data)
- [Key bindings and vim mode](/advanced/keybindings)
- [Asking a model for SQL](/guides/ai)
