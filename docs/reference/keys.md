---
title: 'Keyboard shortcuts'
description: 'Every key in the quarry REPL and TUI.'
---

# Keyboard shortcuts

## REPL

The REPL uses Emacs-style editing by default; set `vi = true` or press <kbd>F4</kbd> for vi mode.

| Key | Does |
|---|---|
| <kbd>Enter</kbd> | Take the highlighted suggestion if the completion menu is open. Otherwise run the buffer if the statement is finished (ends with `;` or `\G`, or is a special command), or start a new line |
| <kbd>Alt</kbd>+<kbd>Enter</kbd> | Run the buffer now, finished or not |
| <kbd>Tab</kbd> | Take the highlighted suggestion, or open the completion menu |
| <kbd>Shift</kbd>+<kbd>Tab</kbd> | Previous suggestion |
| <kbd>Ctrl</kbd>+<kbd>Space</kbd> | Open the completion menu |
| <kbd>↑</kbd> / <kbd>↓</kbd> | Move in the menu, or through lines and history |
| <kbd>→</kbd> | Accept the grey suggestion from history |
| <kbd>Ctrl</kbd>+<kbd>R</kbd> | Search history |
| <kbd>Ctrl</kbd>+<kbd>C</kbd> | Cancel the running query (at the server), or clear the line |
| <kbd>Ctrl</kbd>+<kbd>D</kbd> | Quit, on an empty line |
| <kbd>Ctrl</kbd>+<kbd>L</kbd> | Clear the screen |
| <kbd>F2</kbd> | Toggle smart (context-aware) completion |
| <kbd>F3</kbd> | Toggle multi-line mode |
| <kbd>F4</kbd> | Toggle vi / Emacs editing |

## TUI

<kbd>F1</kbd> shows these inside quarry (type to filter), and the command palette
(<kbd>Ctrl</kbd>+<kbd>P</kbd>) lists every action by name.

These are the defaults. You can rebind the keys under **Everywhere** and **Query tab**, and most
of the keys of the editor, the grids, the table view and the explorer: see
[Key bindings and vim mode](/advanced/keybindings) for the action names.

### Everywhere

| Key | Does |
|---|---|
| <kbd>Ctrl</kbd>+<kbd>P</kbd> | Command palette |
| <kbd>F1</kbd>, <kbd>?</kbd> | Shortcuts, filtered as you type (<kbd>?</kbd> doesn't work while typing in the editor) |
| <kbd>Ctrl</kbd>+<kbd>O</kbd> | Connection manager |
| <kbd>Ctrl</kbd>+<kbd>T</kbd> / <kbd>Ctrl</kbd>+<kbd>W</kbd> | New query tab (from the explorer: for the selected database) / close tab |
| <kbd>Alt</kbd>+<kbd>←</kbd> / <kbd>→</kbd>, <kbd>Ctrl</kbd>+<kbd>PgUp</kbd> / <kbd>PgDn</kbd> | Previous / next tab |
| <kbd>Alt</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | Go to tab 1–9 |
| <kbd>F6</kbd> / <kbd>Shift</kbd>+<kbd>F6</kbd> | Move focus: explorer → editor → results |
| <kbd>Alt</kbd>+<kbd>0</kbd> | Focus the explorer |
| <kbd>Ctrl</kbd>+<kbd>B</kbd> | Show or hide the explorer |
| <kbd>Ctrl</kbd>+<kbd>G</kbd> | Go to a table (fuzzy search) |
| <kbd>Ctrl</kbd>+<kbd>Y</kbd> | Theme picker (outside the editor, where it's redo) |
| <kbd>Ctrl</kbd>+<kbd>R</kbd> | History |
| <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Quit |

### Query tab

| Key | Does |
|---|---|
| <kbd>Ctrl</kbd>+<kbd>Enter</kbd>, <kbd>Ctrl</kbd>+<kbd>E</kbd>, <kbd>Alt</kbd>+<kbd>Enter</kbd> | Run the selection, or the statement under the cursor |
| <kbd>F5</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Enter</kbd> | Run everything in the editor |
| <kbd>Esc</kbd>, <kbd>Ctrl</kbd>+<kbd>C</kbd> (while running) | Cancel the query |
| <kbd>F7</kbd> / <kbd>Shift</kbd>+<kbd>F7</kbd> | Explain / explain analyze |
| <kbd>Alt</kbd>+<kbd>F</kbd> | Format the SQL |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> | Save as a favourite |
| <kbd>Ctrl</kbd>+<kbd>X</kbd> | Export results to a file (outside the editor, where it's cut) |
| <kbd>Ctrl</kbd>+<kbd>↑</kbd> / <kbd>↓</kbd> | Resize the editor / results split |

### Editor

| Key | Does |
|---|---|
| Arrows, <kbd>Shift</kbd>+arrows | Move, select |
| <kbd>Ctrl</kbd>+<kbd>←</kbd> / <kbd>→</kbd> | Move by word |
| <kbd>Home</kbd> / <kbd>End</kbd> | Start of text (then column 0) / end of line |
| <kbd>Ctrl</kbd>+<kbd>Home</kbd> / <kbd>End</kbd> | Start / end of the editor |
| <kbd>Ctrl</kbd>+<kbd>Space</kbd> | Completion (it also opens as you type) |
| <kbd>Tab</kbd> / <kbd>Shift</kbd>+<kbd>Tab</kbd> | Indent / unindent (the selected lines) |
| <kbd>Ctrl</kbd>+<kbd>A</kbd> | Select all |
| <kbd>Ctrl</kbd>+<kbd>C</kbd> / <kbd>X</kbd> / <kbd>V</kbd> | Copy / cut / paste (system clipboard) |
| <kbd>Ctrl</kbd>+<kbd>Z</kbd> | Undo |
| <kbd>Ctrl</kbd>+<kbd>Y</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> | Redo |
| <kbd>Ctrl</kbd>+<kbd>D</kbd> | Duplicate the line or selection |
| <kbd>Alt</kbd>+<kbd>↑</kbd> / <kbd>↓</kbd> | Move lines up / down |
| <kbd>Ctrl</kbd>+<kbd>/</kbd> | Comment or uncomment |
| <kbd>Ctrl</kbd>+<kbd>Backspace</kbd>, <kbd>Ctrl</kbd>+<kbd>Delete</kbd> | Delete a word left / right |
| <kbd>Esc</kbd> | Move focus to the results |

With `vi = true` the editor uses vim's modes and keys instead; see
[vim mode](/advanced/keybindings#vim-mode).

In the completion menu: <kbd>↓</kbd> / <kbd>↑</kbd> (or <kbd>Ctrl</kbd>+<kbd>N</kbd> /
<kbd>P</kbd>) move, <kbd>Tab</kbd> or <kbd>Enter</kbd> accept, <kbd>Esc</kbd> closes.

### Results and grids

| Key | Does |
|---|---|
| <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd>, arrows | Move (<kbd>Shift</kbd> extends the selection) |
| <kbd>PgUp</kbd> / <kbd>PgDn</kbd>, <kbd>Ctrl</kbd>+<kbd>U</kbd> / <kbd>D</kbd> | Page / half page |
| <kbd>g</kbd> / <kbd>G</kbd> | First / last row |
| <kbd>0</kbd> / <kbd>$</kbd>, <kbd>Home</kbd> / <kbd>End</kbd> | First / last column |
| <kbd>w</kbd> / <kbd>b</kbd> | A screen of columns right / left |
| <kbd>Tab</kbd> / <kbd>Shift</kbd>+<kbd>Tab</kbd> | Next / previous cell |
| <kbd>v</kbd> / <kbd>V</kbd> | Select a block / whole rows |
| <kbd>Enter</kbd> | View the cell and its row (double-click works too) |
| <kbd>y</kbd> / <kbd>Y</kbd> | Copy cells as TSV / copy rows with a header |
| <kbd>/</kbd>, <kbd>n</kbd>, <kbd>N</kbd> | Search, next, previous |
| <kbd>&lt;</kbd> / <kbd>&gt;</kbd> / <kbd>=</kbd> | Narrow / widen / auto-fit the column |
| <kbd>[</kbd> / <kbd>]</kbd> | Previous / next result set |
| <kbd>m</kbd> | Messages |
| <kbd>i</kbd> | Back to the editor |
| <kbd>Ctrl</kbd>+<kbd>X</kbd> | Export to a file |

### Table view

| Key | Does |
|---|---|
| <kbd>f</kbd> | Filter with a `WHERE` condition |
| <kbd>F</kbd> | Filter by the current cell's value |
| <kbd>Esc</kbd> | Clear the filter |
| <kbd>s</kbd> | Sort by the column (ascending, descending, off) |
| <kbd>e</kbd>, <kbd>F2</kbd> | Edit the cell (`\N` for NULL) |
| <kbd>o</kbd> | Add a row |
| <kbd>D</kbd>, <kbd>Delete</kbd> | Mark rows for deletion |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> | Review and apply staged changes |
| <kbd>u</kbd> | Discard staged changes |
| <kbd>r</kbd>, <kbd>F5</kbd> | Reload |

### Explorer

| Key | Does |
|---|---|
| <kbd>j</kbd> / <kbd>k</kbd>, arrows | Move |
| <kbd>→</kbd> / <kbd>l</kbd>, <kbd>←</kbd> / <kbd>h</kbd>, <kbd>Space</kbd> | Expand, collapse, toggle |
| <kbd>Enter</kbd> | Open a table, show a function, insert a column name, or switch database |
| <kbd>c</kbd> | New [query console](/guides/tui#query-consoles-for-a-database) for the database or schema you're in |
| <kbd>s</kbd> | Table structure |
| <kbd>i</kbd> | Insert the name into the editor |
| <kbd>g</kbd> + <kbd>s</kbd> / <kbd>i</kbd> / <kbd>u</kbd> / <kbd>d</kbd> / <kbd>c</kbd> / <kbd>x</kbd> / <kbd>n</kbd> | Write a SELECT / INSERT / UPDATE / DELETE / CREATE / DROP / COUNT |
| <kbd>/</kbd> | Filter the tree |
| <kbd>r</kbd>, <kbd>F5</kbd> | Reload the schema |
| <kbd>n</kbd> | New connection |
| <kbd>Ctrl</kbd>+<kbd>X</kbd> | Disconnect |

### Other tabs

| Tab | Keys |
|---|---|
| Structure | <kbd>Tab</kbd> / <kbd>]</kbd> and <kbd>Shift</kbd>+<kbd>Tab</kbd> / <kbd>[</kbd> change section, <kbd>1</kbd>–<kbd>7</kbd> jump to one. In DDL: <kbd>y</kbd> copies, <kbd>e</kbd> opens it in an editor. |
| Activity | <kbd>p</kbd> or <kbd>Space</kbd> pauses refreshing, <kbd>r</kbd> refreshes, <kbd>K</kbd> kills the selected session, <kbd>Enter</kbd> shows details |
| Explain | <kbd>j</kbd> / <kbd>k</kbd> select a plan node; details show below |
| History | Type to filter, <kbd>Enter</kbd> opens the statement in a new tab |
| Definition | <kbd>y</kbd> copies, <kbd>e</kbd> opens it in an editor |

### Dialogs and prompts

| Key | Does |
|---|---|
| <kbd>Enter</kbd>, <kbd>y</kbd> | Confirm |
| <kbd>Esc</kbd>, <kbd>n</kbd>, <kbd>q</kbd> | Cancel |
| <kbd>Ctrl</kbd>+<kbd>U</kbd> / <kbd>Ctrl</kbd>+<kbd>K</kbd> | Delete to start / end of the input |
| <kbd>Ctrl</kbd>+<kbd>W</kbd> | Delete a word |

### Connections

| Key | Does |
|---|---|
| <kbd>Enter</kbd>, click | Connect to the selected saved connection (or go to it, if it's open) |
| <kbd>j</kbd> / <kbd>k</kbd>, arrows, wheel | Move |
| <kbd>n</kbd>, <kbd>Tab</kbd> | New connection form |
| <kbd>d</kbd> | Delete the selected connection (no confirmation) |
| <kbd>Esc</kbd> | Close |

In the new-connection form:

| Key | Does |
|---|---|
| <kbd>Tab</kbd> / <kbd>↑</kbd> <kbd>↓</kbd> | Move between fields |
| <kbd>←</kbd> / <kbd>→</kbd>, <kbd>Space</kbd> | Change the type or TLS mode, tick read-only |
| <kbd>Enter</kbd> | Connect (from a text field), or use the **Connect** button |
| <kbd>Esc</kbd> | Back to the list |

### Mouse

With `mouse = true` (the default):

| Do | To |
|---|---|
| Click | Focus a pane; pick a tab, result set, tree item, cell, list item or button |
| Double-click a cell | View it |
| Click a column header in a table view | Sort by it |
| Middle-click a tab | Close it |
| Drag in the editor or grid | Select |
| Drag the explorer's edge or the editor/results border | Resize |
| Wheel | Scroll; in lists, move the selection. <kbd>Shift</kbd>+wheel scrolls the grid sideways. |
| Click outside a dialog | Close it |
