---
title: 'Prompt, icons and completion'
description: 'Make quarry yours: start from the default config, prompt format, icons, vi mode, completion, pager and output defaults.'
---

# Prompt, icons and completion

Everything on this page is set under `[main]` in `~/.config/quarry/config.toml`. The file created on
first run lists every option with a comment. See [Configuration file](/reference/config) for the
full reference.

## Start from the default config

`quarry --default-config` prints the full config file with every option and a comment on each.
Save it and edit what you like:

```sh
quarry --default-config > ~/.config/quarry/config.toml
```

It overwrites the file, so if you already have saved connections, save the output somewhere else
and copy over the parts you want.

## The prompt

`prompt = "auto"` (the default) draws the two-line prompt:

```
╭─ PostgreSQL me@localhost:5432 ▸ app  TX  RO  ✓ 12.0 ms
╰─❯
```

That's the `unicode` [icon set](#icons); with a Nerd Font the product, the database and the badges
get icons too.

Or write your own with these escapes:

| Escape | Becomes |
|---|---|
| `\u` | User |
| `\h` | Host |
| `\p` | Port |
| `\d` | Database |
| `\t` | Product: PostgreSQL, MySQL, MariaDB or SQLite |
| `\n` | New line |
| `\T` | `*` while a transaction is open |
| `\x` | `(ro)` in read-only mode |
| `\D` | Date and time, e.g. `Wed Sep 30 14:05:11 2026` |
| `\R` | Time, e.g. `14:05:11` |
| `\\` | A backslash |

In TOML, double every backslash:

```toml
prompt = "\\t \\u@\\h:\\d\\T> "
prompt_continuation = "… "
```

That gives `PostgreSQL me@localhost:app> `, with `*` before the `>` inside a transaction.
`prompt_continuation` is shown on the second and later lines of a statement.

Try formats at runtime with `\R`: `\R '\u@\d> '` (quote it to keep the trailing space). `\R` alone
goes back to `auto`. The `--prompt` flag sets it for one session.

## Icons

quarry draws icons from a [Nerd Font](https://www.nerdfonts.com) by default: database logos in the
prompt and the status bar, and icons for tables, views, columns, keys and functions in the TUI's
explorer, its tabs and the completion menus. Your terminal needs a Nerd Font (v3) for these, for
example *JetBrainsMono Nerd Font*.

| `icons` | Looks like | For |
|---|---|---|
| `"auto"` (default) | One of the three below | Letting quarry choose |
| `"nerd"` | Database and object icons | Terminals using a Nerd Font |
| `"unicode"` | `▦ ◫ ƒ ✓ ✗` | Any font with ordinary Unicode symbols |
| `"ascii"` | `T V f + x` | Fonts or consoles with no symbols at all |

```toml
icons = "unicode"
```

`--icons unicode` sets it for one run.

A terminal doesn't tell programs which font it uses, so `"auto"` can only rule Nerd icons out, not
confirm them. It picks `nerd` unless one of these holds:

- You're on the Linux console (`TERM=linux`): it picks `ascii`.
- No Nerd Font is installed on this machine, the terminal isn't one that draws the symbols itself
  (kitty, Ghostty and WezTerm do), and you aren't connected over SSH: it picks `unicode`.

Over SSH the fonts are on your own machine, where quarry can't look, so it stays with `nerd`. If
`auto` guesses wrong, set `icons` yourself.

With `"ascii"`, everything quarry draws itself is plain ASCII: the TUI's borders, tree lines,
scrollbars and spinners, and the REPL's prompt and menus. Your own SQL and data are shown as they
are. Result tables in the REPL follow `table_format`, so set `table_format = "ascii"` as well (or
`\T ascii`) for results without box-drawing characters.

## Vi or Emacs keys

The REPL uses Emacs-style editing by default. `vi = true` switches to vi mode, and <kbd>F4</kbd>
toggles between them at runtime. In vi normal mode the `❯` becomes `❮`.

`vi = true` also gives the TUI's SQL editor vim's modes and keys; see
[Key bindings and vim mode](/advanced/keybindings), which also covers changing the TUI's keys.

## Completion

| Option | Default | Effect |
|---|---|---|
| `smart_completion` | `true` | Suggest by context: tables after `FROM`, columns in expressions. Off: every keyword, table and column, matched against the current word. <kbd>F2</kbd> toggles. |
| `complete_while_typing` | `true` | Open the menu as you type. Off: only on <kbd>Tab</kbd>. |
| `join_suggestions` | `true` | Suggest whole `JOIN … ON …` clauses from foreign keys |
| `keyword_casing` | `"auto"` | `upper`, `lower`, or `auto`: upper case unless you're typing in lower case |
| `auto_suggest` | `true` | Grey, fish-style suggestions from your history; <kbd>→</kbd> accepts |
| `auto_refresh_catalog` | `true` | Reload table and column names after `CREATE`, `ALTER`, `DROP` and similar |

If someone else changes the schema, `\refresh` reloads the names quarry completes.

## Running statements

| Option | Default | Effect |
|---|---|---|
| `multi_line` | `true` | <kbd>Enter</kbd> runs only a finished statement (ending in `;`, `\G` or your delimiter). Off: <kbd>Enter</kbd> always runs. <kbd>F3</kbd> toggles. |
| `history_size` | `10000` | Entries kept in history (at least 100) |
| `log_queries` | `false` | Append every statement to `quarry.log` in the data directory |

## Results

| Option | Default | Effect |
|---|---|---|
| `table_format` | `"rounded"` | Default [output format](/reference/output-formats). `\T` changes it for the session. |
| `row_lines` | `true` | A line between rows in the boxed formats. `false` gives a more compact table. |
| `expanded` | `"auto"` | `on`, `off`, or `auto`: vertical when a table is wider than the terminal. `\x` changes it. |
| `max_field_width` | `500` | Cut longer values and add `…`. `0` means no limit. Never applies to machine formats. |
| `row_limit` | `1000` | Ask before showing more rows than this. `0` never asks. `--row-limit` overrides. |
| `null_string` | `"NULL"` | How NULL is shown |
| `timing` | `true` | Show how long each statement took. `\timing` toggles. |

## Paging

| Option | Default | Effect |
|---|---|---|
| `enable_pager` | `true` | Page output that doesn't fit the screen. `\nopager` / `\pager` toggle. |
| `pager` | unset | The pager command. Unset: `$PAGER`, then `less -SRXF`. |

When `$LESS` isn't set, quarry sets it to `-SRXF` for the pager: long lines scroll sideways instead of
wrapping, colours work, and short output doesn't wait for a key.

## Quieter output

`less_chatty = true` (or `--less-chatty`) skips the start-up banner, the goodbye message and the
echo of a favourite's SQL.

## TUI options

| Option | Default | Effect |
|---|---|---|
| `mouse` | `true` | Mouse support in the TUI: click, drag to select or resize, scroll |
| `transparent` | `false` | Keep the terminal's background instead of the theme's; see [Themes](/advanced/themes#transparent-background) |

Keys are set in their own `[keys]` table: see [Key bindings and vim mode](/advanced/keybindings).

The TUI also reads `theme`, `icons`, `vi`, `null_string`, `smart_completion`, `complete_while_typing`,
`join_suggestions`, `keyword_casing`, `auto_refresh_catalog` and `destructive_warning`. The other
options on this page apply to the REPL only.
