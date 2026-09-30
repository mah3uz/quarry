---
title: 'Using the TUI'
description: 'The explorer, query editor, results grid, tabs, command palette and connections.'
---

# Using the TUI


The TUI is quarry's full-screen interface. Open it in any of these ways:

```sh
quarry --tui postgres://me@localhost/app   # or -T
quarry                                     # no target: starts on the connection manager
```

From the REPL, `\tui` opens it on the current connection.

<kbd>F1</kbd> (or <kbd>?</kbd> outside the editor) shows every shortcut, and
<kbd>Ctrl</kbd>+<kbd>P</kbd> opens the command palette, which lists every action by name.

## The layout

<Terminal
  title="quarry --tui shop.db"
  capture="tui"
  label="The quarry TUI: the explorer on the left, a query editor and a results grid on the right, and a status bar"
/>

- **Header:** your tabs, the current theme, and `^P` as a reminder of the palette.
- **Explorer** (left): connections, databases, schemas, tables, views, functions and columns.
  <kbd>Ctrl</kbd>+<kbd>B</kbd> hides it.
- **Main area:** the active tab. A query tab has an editor on top and results below;
  <kbd>Ctrl</kbd>+<kbd>↑</kbd> / <kbd>↓</kbd> moves the split.
- **Status bar:** where the focus is, the connection, and badges for `TX` (open transaction),
  `READ-ONLY`, `⇄ ssh` and `TLS`, plus hints for the keys you can use right now.

Move focus with <kbd>F6</kbd> (explorer → editor → results) or jump to the explorer with
<kbd>Alt</kbd>+<kbd>0</kbd>.

## Running queries

Type SQL in the editor, then:

| Key | Runs |
|---|---|
| <kbd>Ctrl</kbd>+<kbd>Enter</kbd> (also <kbd>Ctrl</kbd>+<kbd>E</kbd>, <kbd>Alt</kbd>+<kbd>Enter</kbd>) | The selection, or the statement under the cursor |
| <kbd>F5</kbd> or <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Enter</kbd> | Everything in the editor |
| <kbd>F7</kbd> / <kbd>Shift</kbd>+<kbd>F7</kbd> | Explain / explain analyze |
| <kbd>Esc</kbd> or <kbd>Ctrl</kbd>+<kbd>C</kbd> while running | Cancel at the server |

Statements run in order and stop at the first error. Each statement that returns rows gets its own
result tab; switch with <kbd>[</kbd> and <kbd>]</kbd>. The **Messages** tab (<kbd>m</kbd>) logs
what happened, with row counts, notices and errors. After an error, the failing position is marked
in the editor.

The editor has the comforts you'd expect: completion as you type (<kbd>Ctrl</kbd>+<kbd>Space</kbd>
to ask), bracket matching, auto-closing quotes and brackets, undo and redo, and
<kbd>Ctrl</kbd>+<kbd>/</kbd> to toggle comments. <kbd>Alt</kbd>+<kbd>F</kbd> formats the whole
buffer. <kbd>Ctrl</kbd>+<kbd>D</kbd> duplicates a line and <kbd>Alt</kbd>+<kbd>↑</kbd> /
<kbd>↓</kbd> move lines.

Special commands that describe the database (`\dt`, `\d users`, `\l`, `\df`, `\s`, …) also work in
the editor; their output lands in the results grid.

## The results grid

The grid handles large results: rows appear as they arrive, and a query stops fetching at 200,000
rows (the Messages tab says so).

| Key | Does |
|---|---|
| <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd> or arrows | Move (<kbd>Shift</kbd> extends the selection) |
| <kbd>g</kbd> / <kbd>G</kbd>, <kbd>0</kbd> / <kbd>$</kbd> | First / last row, first / last column |
| <kbd>v</kbd> / <kbd>V</kbd> | Select a block / whole rows |
| <kbd>Enter</kbd> | View the cell, with JSON pretty-printed, and the whole row |
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
| <kbd>s</kbd> | Structure: columns, indexes, foreign keys, referencing tables, constraints, triggers, DDL |
| <kbd>i</kbd> | Insert the name into the editor |
| <kbd>g</kbd> then <kbd>s</kbd> <kbd>i</kbd> <kbd>u</kbd> <kbd>d</kbd> <kbd>c</kbd> <kbd>x</kbd> <kbd>n</kbd> | Write a SELECT, INSERT, UPDATE, DELETE, CREATE, DROP or COUNT for this table into an editor. Nothing runs until you run it. |
| <kbd>/</kbd> | Filter the tree |
| <kbd>r</kbd> | Reload the schema |
| <kbd>n</kbd> | Open the connection manager |

On PostgreSQL the explorer also lists the server's other databases; <kbd>Enter</kbd> on one
switches to it. System schemas are hidden. The schema reloads by itself after statements such as
`CREATE`, `ALTER` and `DROP`.

<kbd>Ctrl</kbd>+<kbd>G</kbd> jumps to any table by fuzzy name, across all open connections.

## Other tabs

| Tab | Opened with | Shows |
|---|---|---|
| Query ⌘ | <kbd>Ctrl</kbd>+<kbd>T</kbd> | Editor and results |
| Table ▦ | <kbd>Enter</kbd> on a table | Rows, page by page, with filters, sorting and edits |
| Structure ⚙ | <kbd>s</kbd> in the explorer | Columns, indexes, keys, constraints, triggers, DDL |
| Explain ⊿ | <kbd>F7</kbd> | The plan as a tree, with bars for cost or time, and details per node |
| Activity ⚡ | Palette → *Server activity* | Live sessions on a PostgreSQL or MySQL server, refreshed every 2 s. <kbd>K</kbd> kills one. |
| History ↺ | <kbd>Ctrl</kbd>+<kbd>R</kbd> | Past statements; type to filter, <kbd>Enter</kbd> opens one in a new tab |

Close a tab with <kbd>Ctrl</kbd>+<kbd>W</kbd>, and move between tabs with
<kbd>Alt</kbd>+<kbd>←</kbd> / <kbd>→</kbd> or <kbd>Alt</kbd>+<kbd>1</kbd>…<kbd>9</kbd>.

## Connections

<kbd>Ctrl</kbd>+<kbd>O</kbd> opens the connection manager. On the left are your saved connections;
<kbd>Enter</kbd> connects, <kbd>d</kbd> deletes one. On the right is a form for a new connection:
paste a URL, or fill in type, host, port, user, password, database, TLS mode and read-only. Leave
**Save** on to add it to your config. The password is never saved.

You can have several connections open at once. Each gets its own tree in the explorer, and each
tab belongs to one connection. Every connection uses two sessions, one for your queries and one for
the explorer and table views, so the explorer keeps working while a long query runs.

<kbd>Ctrl</kbd>+<kbd>X</kbd> on a connection in the explorer disconnects it.

## Favourites, files and themes

- <kbd>Ctrl</kbd>+<kbd>S</kbd> in a query tab saves the selection (or the whole editor) as a
  [favourite query](/advanced/favorites). Saved favourites appear in the palette.
- **Open SQL file…** and **Save editor to file…** are in the palette.
- <kbd>Ctrl</kbd>+<kbd>Y</kbd> (outside the editor, where it means redo) opens the theme picker.
  Themes preview as you move; <kbd>Enter</kbd> keeps one and remembers it for next time.

## Transactions

Run `BEGIN` in the editor to start a transaction; the `TX` badge appears. **Commit transaction** and
**Rollback transaction** are in the palette, or type `COMMIT` / `ROLLBACK`. Quitting with an open
transaction asks first, and the transaction is rolled back.

## Things to know

- The TUI doesn't restore your tabs or editor text between runs. Only the theme is remembered.
- Quitting asks for confirmation if a query is running, a transaction is open, or a table view has
  unapplied edits.
- `--theme` and `--no-color` apply to the TUI too. After `\tui`, the TUI keeps the REPL's theme.
- `row_limit`, `table_format`, `timing`, `vi` and the pager settings apply to the REPL, not the TUI.

## Related

- [Browsing and editing tables](/guides/editing-data)
- [Keyboard shortcuts](/reference/keys#tui)
- [Asking a model for SQL](/guides/ai)
