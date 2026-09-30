---
title: 'Key bindings and vim mode'
description: 'Change the TUI''s keys in the config, see what each action is called, and edit SQL with vim keys.'
---

# Key bindings and vim mode

The TUI's keys work out of the box, and you can change any of the app-wide ones. You can also edit
SQL with vim keys.

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
- **Keys the editor uses itself** (<kbd>Ctrl</kbd>+<kbd>C</kbd>, <kbd>X</kbd>, <kbd>V</kbd>,
  <kbd>A</kbd>, <kbd>Z</kbd>, <kbd>Y</kbd>, <kbd>D</kbd>, <kbd>/</kbd>, <kbd>Space</kbd>) keep
  their editor meaning while the editor has focus. A binding on one of them works everywhere else,
  and quarry warns you about it.
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

Keys inside a pane, such as moving in the results grid or the explorer, are fixed. They're listed
in [Keyboard shortcuts](/reference/keys).

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

Not supported: search (`/`), marks, macros, registers other than the clipboard, `.` to repeat,
and `:` commands.

## Related

- [Keyboard shortcuts](/reference/keys): every key, including the fixed ones
- [Using the TUI](/guides/tui)
- [Configuration file](/reference/config#keys)
