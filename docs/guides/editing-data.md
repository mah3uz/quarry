---
title: 'Browsing and editing tables'
description: 'The TUI''s table view - paging, filtering, sorting, and staging edits that apply in one transaction.'
---

# Browsing and editing tables

In the TUI, press <kbd>Enter</kbd> on a table or view in the explorer (or pick it with
<kbd>Ctrl</kbd>+<kbd>G</kbd>) to open a **table view**. It shows the rows page by page, and lets you
filter, sort and edit them without writing SQL.

## Browsing

- Rows load 500 at a time. The next page loads as you scroll near the end.
- The toolbar shows the table, the current filter and sort, the total row count and how many rows
  are loaded.
- Rows are ordered by the primary key by default, when the table has a single-column one.
- <kbd>r</kbd> or <kbd>F5</kbd> reloads.

All the [results grid keys](/guides/tui#the-results-grid) work here too: move, select, search,
copy, view a cell.

## Filtering and sorting

| Key | Does |
|---|---|
| <kbd>f</kbd> | Filter with a `WHERE` condition you type, such as `status = 'paid' and total > 100`. Leave it empty to clear. |
| <kbd>F</kbd> | Filter on the value of the current cell (`column = value`, or `IS NULL`) |
| <kbd>Esc</kbd> | Clear the filter |
| <kbd>s</kbd> or click a header | Sort by the column: ascending, then descending, then off |

Filtering and sorting run on the server, so they work on the whole table, not just the loaded rows.
The filter is used as written, so it can be any condition your database understands.

::: info
In a query tab, sorting would mean re-running your query. Add an `ORDER BY` there instead.
:::

## Editing

Edits are **staged**: nothing is written until you review and apply them.

| Key | Does |
|---|---|
| <kbd>e</kbd> or <kbd>F2</kbd> | Edit the current cell. Type `\N` for NULL. |
| <kbd>o</kbd> | Add a new row (all NULL; fill it in with <kbd>e</kbd>) |
| <kbd>D</kbd> or <kbd>Delete</kbd> | Mark the selected rows for deletion (again to unmark) |
| <kbd>u</kbd> | Discard all staged changes |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> | Review and apply |

Staged changes show in the grid: edited cells are underlined, new rows are marked `+`, and rows to
delete are marked `−` and crossed out. The tab title gets a `●` and the toolbar counts them.

### Review and apply

<kbd>Ctrl</kbd>+<kbd>S</kbd> shows the exact SQL quarry will run: DELETEs, then UPDATEs of only the
columns you changed, then INSERTs. Rows are matched by their **original** primary key, so editing a
key column works too. Confirm and all the statements run in **one transaction**. If any statement
fails, the whole transaction is rolled back and nothing changes.

After a successful apply, the marks clear and the data reloads.

### When editing isn't allowed

- **The table has no primary key.** quarry needs one to target rows safely.
- **It's a view.**
- **The connection is read-only.**

### Safety nets

- Reloading is refused while changes are staged.
- Closing the tab with staged changes asks first.
- Quitting quarry with staged changes asks first.
- The review dialog is highlighted in red when it contains a DELETE.

## Exporting

<kbd>Ctrl</kbd>+<kbd>X</kbd> exports what's in the grid to a file. In a table view that means the
rows loaded so far, not the whole table. To export everything, run a query instead, or use
[`\export`](/guides/scripting#from-the-repl) in the REPL.
