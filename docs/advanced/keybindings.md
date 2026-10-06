---
title: 'Key bindings and vim mode'
description: 'Change the TUI''s keys in the config, see what each action is called, and edit SQL with vim keys.'
---

# Key bindings and vim mode

The TUI's keys work out of the box, and you can change them: the app-wide ones and the ones each
pane uses for itself. You can also edit SQL with vim keys.

To see every key in quarry itself, press <kbd>F1</kbd> and type to filter, for example `run`,
`ctrl` or `vim`. Each rebindable action shows its name on the right. That name is what you write
in the config.

## Change a key

Add a `[keys]` table to `~/.config/quarry/config.toml` (run `quarry --default-config` to see a
commented example). Each line names an action and gives it one key or a list of keys:

```toml
[keys]
run_statement = ["ctrl+enter", "ctrl+e"]   # several keys
run_all = "f9"                              # one key
themes = "alt+t"
quit = []                                   # no key (Quit stays in the command palette)
```

A line replaces all of that action's default keys. Actions you don't list keep their defaults.

### How to write a key

Modifiers, then the key, joined with `+`. Case doesn't matter.

| Write | For |
|---|---|
| `ctrl`, `alt`, `shift` | Modifiers, e.g. `ctrl+shift+enter` |
| `a` … `z`, `0` … `9`, `?`, `/`, … | A character |
| `enter`, `esc`, `tab`, `space`, `backspace`, `delete`, `insert` | Named keys |
| `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown` | Movement keys |
| `f1` … `f24` | Function keys |

### When keys collide

quarry sorts collisions out for you, so a new binding never silently breaks another:

- **Your binding wins over a default.** If you bind `run_all = "ctrl+r"`, <kbd>Ctrl</kbd>+<kbd>R</kbd>
  stops opening History. When an action is left without any key this way, quarry tells you when
  the TUI starts, and you can give it a new one.
- **Two of your own bindings on one key:** one action keeps it and you get a warning naming both.
  The action that comes first in the list below wins.
- **Alt+1…9** always jump to tabs and can't be rebound.
- **Keys the editor uses itself** (its actions under [Editor](#editor), such as
  <kbd>Ctrl</kbd>+<kbd>C</kbd> or <kbd>Ctrl</kbd>+<kbd>Y</kbd>) keep their editor meaning while
  the editor has focus. An app-wide binding on one of them works everywhere else, and quarry
  warns you about it. Give the editor action another key and the app-wide one works in the editor too.
- **A pane's keys are its own.** `grid_copy = "c"` changes <kbd>c</kbd> in the grid only; the
  explorer's <kbd>c</kbd> is untouched. Within one pane a key runs one action, and a table view
  counts as a grid. A key held by a Global action never reaches a pane, and quarry says so.
- **A character can't be an editor shortcut**, since you type it there. Editor actions need
  <kbd>Ctrl</kbd>, <kbd>Alt</kbd> or a function key.
- **A plain character** such as `?` works only when you aren't typing in a text field, so it
  never gets in the way of your SQL.
- An **unknown action or key** is skipped with a warning; the rest of `[keys]` still applies.

Warnings appear as notices when the TUI starts.

## Actions you can rebind

### Global

| Action | Default keys | Does |
|---|---|---|
| `commands` | <kbd>Ctrl</kbd>+<kbd>P</kbd> | Command palette |
| `help` | <kbd>F1</kbd>, <kbd>?</kbd> | Keyboard shortcuts |
| `connections` | <kbd>Ctrl</kbd>+<kbd>O</kbd> | Connections |
| `new_query` | <kbd>Ctrl</kbd>+<kbd>T</kbd> | New query tab (in the explorer: for the selected database) |
| `close_tab` | <kbd>Ctrl</kbd>+<kbd>W</kbd> | Close tab |
| `next_tab` | <kbd>Alt</kbd>+<kbd>→</kbd>, <kbd>Ctrl</kbd>+<kbd>PgDn</kbd> | Next tab |
| `prev_tab` | <kbd>Alt</kbd>+<kbd>←</kbd>, <kbd>Ctrl</kbd>+<kbd>PgUp</kbd> | Previous tab |
| `next_pane` | <kbd>F6</kbd> | Focus the next pane: explorer, editor, results |
| `prev_pane` | <kbd>Shift</kbd>+<kbd>F6</kbd> | Focus the previous pane |
| `focus_explorer` | <kbd>Alt</kbd>+<kbd>0</kbd> | Focus the explorer |
| `toggle_explorer` | <kbd>Ctrl</kbd>+<kbd>B</kbd> | Show or hide the explorer |
| `go_to_table` | <kbd>Ctrl</kbd>+<kbd>G</kbd> | Go to a table |
| `themes` | <kbd>Ctrl</kbd>+<kbd>Y</kbd> | Theme picker |
| `history` | <kbd>Ctrl</kbd>+<kbd>R</kbd> | Query history |
| `quit` | <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Quit |

### Query tabs

| Action | Default keys | Does |
|---|---|---|
| `run_statement` | <kbd>Ctrl</kbd>+<kbd>Enter</kbd>, <kbd>Alt</kbd>+<kbd>Enter</kbd>, <kbd>Ctrl</kbd>+<kbd>E</kbd> | Run the statement under the cursor, or the selection |
| `run_all` | <kbd>F5</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Enter</kbd> | Run everything in the editor |
| `cancel` | <kbd>Esc</kbd>, <kbd>Ctrl</kbd>+<kbd>C</kbd> | Cancel the running query |
| `explain` | <kbd>F7</kbd> | Explain |
| `explain_analyze` | <kbd>Shift</kbd>+<kbd>F7</kbd> | Explain analyze |
| `format_sql` | <kbd>Alt</kbd>+<kbd>F</kbd> | Format the SQL |
| `save_favorite` | <kbd>Ctrl</kbd>+<kbd>S</kbd> | Save the query as a favourite |
| `export` | <kbd>Ctrl</kbd>+<kbd>X</kbd> | Export results to a file |
| `editor_smaller` | <kbd>Ctrl</kbd>+<kbd>↑</kbd> | Make the editor smaller |
| `editor_larger` | <kbd>Ctrl</kbd>+<kbd>↓</kbd> | Make the editor larger |

### Editor

These apply while you type, and in vim's insert mode. vim's normal and visual modes keep their own keys.

| Action | Default keys | Does |
|---|---|---|
| `editor_complete` | <kbd>Ctrl</kbd>+<kbd>Space</kbd> | Completion (it also opens as you type) |
| `editor_select_all` | <kbd>Ctrl</kbd>+<kbd>A</kbd> | Select all |
| `editor_copy` | <kbd>Ctrl</kbd>+<kbd>C</kbd> | Copy the selection |
| `editor_cut` | <kbd>Ctrl</kbd>+<kbd>X</kbd> | Cut the selection |
| `editor_paste` | <kbd>Ctrl</kbd>+<kbd>V</kbd> | Paste |
| `editor_undo` | <kbd>Ctrl</kbd>+<kbd>Z</kbd> | Undo |
| `editor_redo` | <kbd>Ctrl</kbd>+<kbd>Y</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> | Redo |
| `editor_duplicate` | <kbd>Ctrl</kbd>+<kbd>D</kbd> | Duplicate the line or selection |
| `editor_comment` | <kbd>Ctrl</kbd>+<kbd>/</kbd>, <kbd>Ctrl</kbd>+<kbd>7</kbd> | Comment or uncomment |
| `editor_line_up` | <kbd>Alt</kbd>+<kbd>↑</kbd> | Move the line up |
| `editor_line_down` | <kbd>Alt</kbd>+<kbd>↓</kbd> | Move the line down |
| `editor_delete_word_left` | <kbd>Ctrl</kbd>+<kbd>Backspace</kbd>, <kbd>Ctrl</kbd>+<kbd>H</kbd> | Delete the word before the cursor |
| `editor_delete_word_right` | <kbd>Ctrl</kbd>+<kbd>Delete</kbd> | Delete the word after the cursor |

### Results and grids

The results of a query, and the grid of a table view.

| Action | Default keys | Does |
|---|---|---|
| `grid_up` | <kbd>k</kbd>, <kbd>↑</kbd> | Up |
| `grid_down` | <kbd>j</kbd>, <kbd>↓</kbd> | Down |
| `grid_left` | <kbd>h</kbd>, <kbd>←</kbd> | Left |
| `grid_right` | <kbd>l</kbd>, <kbd>→</kbd> | Right |
| `grid_page_up` | <kbd>PgUp</kbd> | Page up |
| `grid_page_down` | <kbd>PgDn</kbd> | Page down |
| `grid_half_page_up` | <kbd>Ctrl</kbd>+<kbd>U</kbd> | Half a page up |
| `grid_half_page_down` | <kbd>Ctrl</kbd>+<kbd>D</kbd> | Half a page down |
| `grid_first_row` | <kbd>g</kbd> | First row |
| `grid_last_row` | <kbd>G</kbd> | Last row |
| `grid_first_column` | <kbd>0</kbd>, <kbd>Home</kbd> | First column |
| `grid_last_column` | <kbd>$</kbd>, <kbd>End</kbd> | Last column |
| `grid_columns_right` | <kbd>w</kbd> | A screen of columns right |
| `grid_columns_left` | <kbd>b</kbd> | A screen of columns left |
| `grid_next_cell` | <kbd>Tab</kbd> | Next cell |
| `grid_prev_cell` | <kbd>Shift</kbd>+<kbd>Tab</kbd> | Previous cell |
| `grid_select` | <kbd>v</kbd> | Select a block of cells |
| `grid_select_rows` | <kbd>V</kbd> | Select whole rows |
| `grid_view_cell` | <kbd>Enter</kbd> | View the cell and its row |
| `grid_copy` | <kbd>y</kbd> | Copy cells as TSV |
| `grid_copy_rows` | <kbd>Y</kbd> | Copy rows with a header |
| `grid_search` | <kbd>/</kbd> | Search in the results |
| `grid_search_next` | <kbd>n</kbd> | Next match |
| `grid_search_prev` | <kbd>N</kbd> | Previous match |
| `grid_narrow_column` | <kbd>&lt;</kbd> | Narrow the column |
| `grid_widen_column` | <kbd>&gt;</kbd> | Widen the column |
| `grid_fit_column` | <kbd>=</kbd> | Fit the column to its content |
| `grid_prev_result` | <kbd>[</kbd> | Previous result set |
| `grid_next_result` | <kbd>]</kbd> | Next result set |
| `grid_messages` | <kbd>m</kbd> | Messages |
| `grid_to_editor` | <kbd>i</kbd> | Back to the editor |

### Table view

On top of the grid's keys.

| Action | Default keys | Does |
|---|---|---|
| `table_filter` | <kbd>f</kbd> | Filter with a WHERE condition |
| `table_filter_by_value` | <kbd>F</kbd> | Filter by the current cell's value |
| `table_sort` | <kbd>s</kbd> | Sort by the column |
| `table_edit_cell` | <kbd>e</kbd>, <kbd>F2</kbd> | Edit the cell |
| `table_add_row` | <kbd>o</kbd> | Add a row |
| `table_delete_rows` | <kbd>D</kbd>, <kbd>Delete</kbd> | Mark rows for deletion |
| `table_apply` | <kbd>Ctrl</kbd>+<kbd>S</kbd> | Review and apply staged changes |
| `table_discard` | <kbd>u</kbd> | Discard staged changes |
| `table_reload` | <kbd>r</kbd>, <kbd>F5</kbd> | Reload |

### Explorer

After the `explorer_script` key, the next key (<kbd>s</kbd> <kbd>i</kbd> <kbd>u</kbd> <kbd>d</kbd> <kbd>c</kbd> <kbd>x</kbd> <kbd>n</kbd>) picks the statement and is fixed.

| Action | Default keys | Does |
|---|---|---|
| `explorer_up` | <kbd>k</kbd>, <kbd>↑</kbd> | Up |
| `explorer_down` | <kbd>j</kbd>, <kbd>↓</kbd> | Down |
| `explorer_expand` | <kbd>l</kbd>, <kbd>→</kbd> | Expand |
| `explorer_collapse` | <kbd>h</kbd>, <kbd>←</kbd> | Collapse, or go to the parent |
| `explorer_toggle` | <kbd>Space</kbd> | Expand or collapse |
| `explorer_last` | <kbd>G</kbd>, <kbd>End</kbd> | Last row |
| `explorer_open` | <kbd>Enter</kbd> | Open a table, show a function, insert a column, switch database |
| `explorer_console` | <kbd>c</kbd> | New query tab for this database or schema |
| `explorer_structure` | <kbd>s</kbd> | Table structure |
| `explorer_insert_name` | <kbd>i</kbd> | Insert the name into the editor |
| `explorer_script` | <kbd>g</kbd> | Write a statement for the table, with s i u d c x n next |
| `explorer_filter` | <kbd>/</kbd> | Filter the tree |
| `explorer_reload` | <kbd>r</kbd>, <kbd>F5</kbd> | Reload the schema |
| `explorer_new_connection` | <kbd>n</kbd> | New connection |
| `explorer_disconnect` | <kbd>Ctrl</kbd>+<kbd>X</kbd> | Disconnect |

A few keys stay fixed: arrows and the other typing keys in the editor, <kbd>Shift</kbd>+arrows
to select in a grid, <kbd>Esc</kbd>, vim's keys, and the keys of dialogs and the other tabs.
They're listed in [Keyboard shortcuts](/reference/keys). The hints in the status bar, in notices
and in the empty editor follow your bindings.

## Vim mode

Set `vi = true` under `[main]`. The REPL gets vi editing, and the TUI's SQL editor gets vim's
modes:

- **Normal** is where you start. The cursor is a block and the status bar says `NORMAL`.
- **Insert** (<kbd>i</kbd>, <kbd>a</kbd>, <kbd>o</kbd>, …) is ordinary typing, with completion as
  you type. The cursor turns into a bar.
- **Visual** (<kbd>v</kbd>) and **visual-line** (<kbd>V</kbd>) select text for an operator.

<kbd>Esc</kbd> goes back to normal mode. In normal mode, <kbd>Esc</kbd> moves to the results, as
it does without vim. The app-wide keys (run, explain, tabs, palette) work in every mode.

| Keys | Do |
|---|---|
| <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd> | Left, down, up, right |
| <kbd>w</kbd> <kbd>b</kbd> <kbd>e</kbd> | Next word, previous word, end of word |
| <kbd>0</kbd> <kbd>^</kbd> <kbd>$</kbd> | Line start, first non-blank, line end |
| <kbd>g</kbd><kbd>g</kbd> <kbd>G</kbd> | First line, last line (`5G` goes to line 5) |
| <kbd>Ctrl</kbd>+<kbd>D</kbd> / <kbd>Ctrl</kbd>+<kbd>U</kbd> | Half a page down / up |
| <kbd>/</kbd> + text, <kbd>Enter</kbd> | Search forward for the text (<kbd>Esc</kbd> cancels) |
| <kbd>n</kbd> <kbd>N</kbd> | Next / previous match, wrapping round the text |
| <kbd>i</kbd> <kbd>a</kbd> <kbd>I</kbd> <kbd>A</kbd> | Insert before, after, at line start, at line end |
| <kbd>o</kbd> <kbd>O</kbd> | New line below / above |
| <kbd>d</kbd> <kbd>c</kbd> <kbd>y</kbd> <kbd>&gt;</kbd> <kbd>&lt;</kbd> + a motion | Delete, change, yank, indent, dedent: `dw`, `c$`, `y2j`, `>G` |
| <kbd>d</kbd><kbd>d</kbd> <kbd>c</kbd><kbd>c</kbd> <kbd>y</kbd><kbd>y</kbd> <kbd>&gt;</kbd><kbd>&gt;</kbd> <kbd>&lt;</kbd><kbd>&lt;</kbd> | The same on whole lines |
| `ciw`, `diw`, `yaw` | Change, delete, yank a word (`a` includes the space after it) |
| <kbd>x</kbd> <kbd>X</kbd> | Delete the character under / before the cursor |
| <kbd>s</kbd> <kbd>S</kbd> | Replace the character / the line |
| <kbd>D</kbd> <kbd>C</kbd> <kbd>Y</kbd> | Delete / change to the end of the line, yank the line |
| <kbd>r</kbd> + a character | Replace the character under the cursor |
| <kbd>J</kbd> | Join the next line onto this one |
| <kbd>p</kbd> <kbd>P</kbd> | Paste after / before. Whole lines go below / above. |
| <kbd>u</kbd>, <kbd>Ctrl</kbd>+<kbd>R</kbd> | Undo, redo |

A number in front repeats: `3j`, `2dd`, `5x`. Yanks and deletes go to the system clipboard, so you
can paste them elsewhere, and text copied elsewhere pastes with <kbd>p</kbd>.

A search is for the text as typed, not a pattern. All in lowercase it matches any case; with a
capital letter in it, the case must match too.

Not supported: backward search (`?`), search as a motion (`d/x`), marks, macros, registers other
than the clipboard, `.` to repeat, and `:` commands.

## Related

- [Keyboard shortcuts](/reference/keys): every key, including the fixed ones
- [Using the TUI](/guides/tui)
- [Configuration file](/reference/config#keys)
