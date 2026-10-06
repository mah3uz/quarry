---
title: 'Configuration file'
description: 'Every option in quarry''s config.toml, with defaults.'
---

# Configuration file

quarry reads `~/.config/quarry/config.toml` (or `$XDG_CONFIG_HOME/quarry/config.toml`). The first
time it runs it writes this file with every option and a comment explaining it. Every option is
optional: delete a line to get the default back.

- `quarry --default-config` prints that commented file, so you can start over from it any time:
  `quarry --default-config > ~/.config/quarry/config.toml`.
- `--config FILE` uses a different file for one run.
- `QUARRY_CONFIG_DIR` changes the whole config directory.
- A syntax error stops quarry with the file, line and column. Unknown keys are ignored.
- quarry edits the file when you use `--save`, `--setup-llm` or the TUI's connection manager. It
  changes only the settings involved, so your comments stay, and keeps the previous version as
  `config.toml.bak`.

## `[main]`

### Appearance

| Key | Default | Values | Meaning |
|---|---|---|---|
| `theme` | `"tokyo-night"` | A [theme](/advanced/themes) name | Colour theme for the REPL, and for the TUI until you pick one there |
| `table_format` | `"rounded"` | An [output format](/reference/output-formats) | How results are printed in the REPL |
| `row_lines` | `true` | `true`, `false` | A line between rows in the boxed formats (rounded, unicode, double, ascii) |
| `icons` | `"auto"` | `auto`, `nerd`, `unicode`, `ascii` | [Icons](/advanced/customising#icons) in the prompt, the TUI and messages. `nerd` needs a Nerd Font; `auto` uses it unless the glyphs can't be there. |
| `expanded` | `"auto"` | `on`, `off`, `auto` | Vertical output. `auto`: when a table is wider than the terminal. |
| `null_string` | `"NULL"` | Any text | How NULL is shown |
| `max_field_width` | `500` | Number, `0` = no limit | Cut longer values and add `…`. Machine formats are never cut. |
| `less_chatty` | `false` | `true`, `false` | Skip the banner and goodbye message |
| `prompt` | `"auto"` | `auto` or a format | The REPL prompt. See [the prompt](/advanced/customising#the-prompt). |
| `prompt_continuation` | `"… "` | Any text | Prefix for the second and later lines of a statement (custom prompts) |

### Behaviour

| Key | Default | Values | Meaning |
|---|---|---|---|
| `row_limit` | `1000` | Number, `0` = never ask | Ask before showing more rows than this (REPL) |
| `timing` | `true` | `true`, `false` | Show how long each statement took |
| `multi_line` | `true` | `true`, `false` | <kbd>Enter</kbd> runs only finished statements |
| `vi` | `false` | `true`, `false` | Vi keys in the REPL and [vim mode](/advanced/keybindings#vim-mode) in the TUI's editor |
| `restore_tabs` | `true` | `true`, `false` | The TUI reopens each connection's query, table and structure tabs. `false` also deletes what was kept. |
| `enable_pager` | `true` | `true`, `false` | Page output that doesn't fit the screen |
| `pager` | not set | A command | Pager command. Not set: `$PAGER`, then `less -SRXF`. |
| `history_size` | `10000` | Number (minimum 100) | History entries to keep |
| `log_queries` | `false` | `true`, `false` | Append every statement to `quarry.log` in the data directory |
| `destructive_warning` | see below | List of rules | Statements that ask before running. `[]` never asks. See [Staying safe](/guides/safety). |

`destructive_warning` defaults to:

```toml
destructive_warning = ["drop", "truncate", "shutdown", "unconditional_update", "unconditional_delete"]
```

### Completion

| Key | Default | Meaning |
|---|---|---|
| `smart_completion` | `true` | Suggest by context. Off: every keyword, table and column. |
| `complete_while_typing` | `true` | Open the completion menu as you type (<kbd>Tab</kbd> always opens it) |
| `join_suggestions` | `true` | Suggest `JOIN … ON …` clauses from foreign keys |
| `keyword_casing` | `"auto"` | `upper`, `lower` or `auto` (follow what you type) |
| `auto_suggest` | `true` | Grey suggestions from history in the REPL |
| `auto_refresh_catalog` | `true` | Reload table and column names after `CREATE`, `ALTER`, `DROP` and similar |

### TUI

| Key | Default | Meaning |
|---|---|---|
| `mouse` | `true` | [Mouse support](/guides/tui#using-the-mouse) in the TUI |
| `transparent` | `false` | Keep the terminal's own background instead of the theme's (e.g. a translucent terminal). Also in the palette: *Toggle transparent background*. |

## `[llm]`

How `\llm` reaches a model. `quarry --setup-llm` writes this section for you. See
[Asking a model for SQL](/guides/ai).

| Key | Default | Meaning |
|---|---|---|
| `provider` | `"anthropic"` | `anthropic`, `openai` (any OpenAI-compatible API), `claude-code` or `codex` |
| `model` | `"claude-opus-5-5"` | Model name. Empty means the CLI's own default (`claude-code`, `codex`). |
| `base_url` | not set | Server for the `openai` provider, e.g. `http://localhost:11434/v1` |

API keys are **not** stored here; they go in `credentials.toml` in the data directory.

## `[keys]`

TUI key bindings, one line per action: a key or a list of keys, `[]` for none. A line replaces the
action's default keys; a key you bind is taken away from any action that had it by default. See
[Key bindings and vim mode](/advanced/keybindings) for the action names and the rules.

```toml
[keys]
run_statement = ["ctrl+enter", "ctrl+e"]
run_all = "f9"
themes = "alt+t"
```

## `[connections.NAME]`

One table per saved connection. See [Saved connections](/advanced/saved-connections).

| Key | Default | Meaning |
|---|---|---|
| `url` | | The target: a URL or SQLite file |
| `password_command` | not set | Shell command that prints the password |
| `ssh` | not set | SSH tunnel: `[user@]host[:port]` |
| `readonly` | `false` | Open in read-only mode |
| `color` | not set | Tag colour in the TUI, e.g. `"red"`: `"#rrggbb"` or a colour name |
| `init_commands` | `[]` | SQL to run after connecting |

## A complete example

```toml
[main]
theme = "catppuccin-mocha"
table_format = "psql"
row_limit = 5000
vi = true
keyword_casing = "upper"
destructive_warning = ["drop", "truncate", "alter", "unconditional_update", "unconditional_delete"]
prompt = "\\t \\u@\\h:\\d\\T> "

[keys]
run_all = "f9"

[llm]
provider = "openai"
model = "qwen2.5-coder"
base_url = "http://localhost:11434/v1"

[connections.local]
url = "postgres://me@localhost/app"

[connections.prod]
url = "postgres://deploy@db.internal/app?sslmode=verify-full"
password_command = "pass show db/prod"
ssh = "deploy@bastion.example.com"
readonly = true
init_commands = ["SET statement_timeout = '30s'"]
```
