---
title: Themes
description: Choose one of 27 built-in colour themes, or make your own from a TOML palette or a base16 / base24 scheme.
---

The REPL and the TUI share one set of colour themes. The default is **Tokyo Night**.

## Built-in themes

| Family | Themes |
|---|---|
| Tokyo Night | `tokyo-night`, `tokyo-night-storm`, `tokyo-night-day` |
| Catppuccin | `catppuccin-mocha`, `catppuccin-macchiato`, `catppuccin-frappe`, `catppuccin-latte` |
| Gruvbox | `gruvbox-dark`, `gruvbox-light` |
| Solarized | `solarized-dark`, `solarized-light` |
| Rosé Pine | `rose-pine`, `rose-pine-moon`, `rose-pine-dawn` |
| Everforest | `everforest-dark`, `everforest-light` |
| GitHub | `github-dark`, `github-light` |
| Others | `dracula`, `nord`, `one-dark`, `kanagawa`, `monokai`, `ayu-dark`, `nightfox`, `material-ocean` |
| Terminal | `ansi`: uses your terminal's own 16 colours |

Names are forgiving: case, spaces, `-` and `_` don't matter, so `Tokyo Night` and `tokyo_night`
both work. Family names work as shortcuts too: `catppuccin` means mocha, `gruvbox`, `solarized`,
`everforest`, `github` and `ayu` mean the dark variant, and `material` means `material-ocean`.

## Choosing a theme

| Where | How | Remembered? |
|---|---|---|
| Config | `theme = "catppuccin-mocha"` under `[main]` | Yes |
| Command line | `--theme dracula` | No, this run only (REPL and TUI) |
| REPL | `\theme` lists themes, `\theme nord` switches | No |
| TUI | <kbd>Ctrl</kbd>+<kbd>Y</kbd> outside the editor, or *Switch theme…* in the palette. Themes preview as you move; <kbd>Enter</kbd> keeps one, <kbd>Esc</kbd> goes back. | Yes, for the TUI |

The theme you pick in the TUI is saved in `~/.local/share/quarry/ui-state.toml` and takes precedence
over the config for the TUI from then on. Delete that file to go back to the config's theme. `--theme`
wins over both for one run, and `\tui` carries the REPL's current theme into the TUI.

## Your own theme

Put a file in `~/.config/quarry/themes/` (the `themes` folder next to your `config.toml`) and use
its name without the extension. Built-in names win over files with the same name.

### A quarry palette (`.toml`)

Start from an existing theme and change what you like:

```toml
# ~/.config/quarry/themes/midnight.toml
inherits = "tokyo-night"
dark = true

bg = "#0f111a"
accent = "#ff9e64"
keyword = "#ff9e64"
```

```toml
[main]
theme = "midnight"
```

Without `inherits`, a theme starts from `tokyo-night`. `inherits` can name a built-in theme or another
of your files, up to 8 levels deep.

Colours are `"#rrggbb"`, or a terminal colour name (`red`, `lightblue`, `gray`, …, or `reset` for the
terminal's default). The keys you can set:

| Group | Keys |
|---|---|
| Surfaces | `bg`, `surface` (panels, popups), `highlight` (current line, hovered row), `selection`, `fg`, `muted`, `border`, `border_focus`, `accent`, `accent2` |
| SQL | `keyword`, `datatype`, `function`, `string`, `number`, `comment`, `operator`, `identifier`, `quoted_ident`, `parameter`, `punctuation` |
| Values and messages | `null`, `boolean`, `temporal`, `json`, `error`, `warning`, `success`, `info`, `header` |

An unknown key or a bad colour is an error that names the file and the key.

### A base16 or base24 scheme (`.yaml`)

Drop any [base16 or base24](https://github.com/tinted-theming/schemes) scheme file into the themes
folder, under a name that isn't a built-in theme: save a Nord scheme as `nord-base16.yaml` and use
`theme = "nord-base16"`. Both the classic layout (`base00:` at the top level) and the newer
tinted-theming layout (a `palette:` map) work.
`base00`–`base0F` are required; if `base10`–`base17` are all present, quarry uses the extra base24
colours for surfaces and messages. Light or dark is taken from `variant:`, or worked out from the
background.

## Colour support

quarry detects what your terminal can show:

- **24-bit colour** when `COLORTERM` is `truecolor` or `24bit`, or the terminal is kitty,
  Alacritty, Ghostty or WezTerm.
- **256 colours** when `TERM` contains `256color`, or in Windows Terminal.
- **16 colours** otherwise, and **none** when `TERM=dumb`.

Colours are mapped to the nearest one your terminal supports. For an exact match with your terminal's
palette, use the `ansi` theme.

`NO_COLOR=1` or `--no-color` turns colour off, in the REPL and the TUI. Output to a pipe or file is
never coloured.
