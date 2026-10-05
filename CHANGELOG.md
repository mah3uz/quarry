# Changelog

What changed in each release of quarry, newest first. Each release's section is also its GitHub Release notes.

## Unreleased

### Fixed

- **Paste works in every field of the TUI.** Pasting from the terminal (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd>,
  middle click) was ignored everywhere except the SQL editor and one-line prompts. It now goes into whatever you're
  typing in: the fields of the New connection form, the command palette, the theme picker, the explorer filter, the
  history filter and the shortcuts filter.
- **A mistyped MySQL user name is reported as a failed login.** MySQL 8 sometimes answers an unknown user by asking
  for a sign-in method quarry doesn't support, and quarry showed that as a driver error. It now says to check the user
  name, and asks for the password again as it does for any refused login.
- **The clipboard on Wayland.** <kbd>Ctrl</kbd>+<kbd>V</kbd> and copying now use the Wayland clipboard directly; before,
  they could paste nothing or copy to a clipboard other programs didn't see.

## 0.1.0 - 2026-10-01 02:40 +06:00

The first release of quarry: a SQL client for the terminal, for **PostgreSQL**, **MySQL / MariaDB** and **SQLite**,
with a smart REPL and a full-screen TUI in one binary. Nothing else to install: TLS is built in and SQLite is
bundled.

### The REPL

**Completion that reads your schema.** Tables after `FROM`, columns of the tables in the statement (aliases
included), whole `JOIN … ON …` clauses from foreign keys, `SELECT *` expansion, functions, types after `::`, special
commands, favourites and file paths. Matching is fuzzy, so `ui` finds `user_id`.

**Multi-line editing that knows SQL.** A statement runs when it ends with `;` or `\G`; trigger and procedure bodies
and `$$` blocks work without changing the delimiter. Highlighting follows each database's dialect, with bracket
matching, fish-style suggestions from history, and vi or Emacs keys.

**Results for people and for scripts.** Rounded tables with a line between rows by default (`row_lines`), wide
results as one framed record per row, and 17 formats in all: `psql`, `ascii`,
Markdown, CSV, TSV, JSON, JSON Lines, HTML, SQL `INSERT` / `UPDATE` and more. Wide results turn vertical by
themselves, long ones go through a pager, and past 1,000 rows quarry asks before fetching the rest. `Ctrl-C` cancels
a query at the server.

**Special commands on every database.** psql's `\d` family translated for MySQL and SQLite, mycli's `use`,
`source`, `tee` and `status`, litecli's dot commands, plus `\watch`, `\explain` as a tree, `\export`, `\e` in your
editor, and favourite queries with `$1`, `$*` and `${name}` placeholders.

### The TUI

`quarry --tui`, or `\tui` from the REPL on the same connection:

- A schema explorer that writes SELECT, INSERT, UPDATE, DELETE, CREATE, DROP and COUNT scripts for you.
- Tabbed editors with completion as you type, auto-closing brackets, undo and SQL formatting, and vim's modes
  with `vi = true`.
- Query consoles per database: <kbd>c</kbd> in the explorer opens a tab that runs in that database or schema,
  with completion for its tables, while other tabs stay where they are.
- A results grid for large results: select, search, copy as TSV, CSV, JSON, Markdown or SQL, and export to a file.
- A table browser with server-side filters and sorting, where edits, inserts and deletes are staged and applied in
  one transaction after you review the SQL.
- Structure and DDL views, an explain-plan tree, a live view of server sessions, a command palette, go-to-table,
  history, and several connections at once.
- The mouse works everywhere: a Run button, clickable tabs, dialogs and lists, drag-to-resize panes.
- Keys you can change under `[keys]`, with conflicts sorted out and reported, and <kbd>F1</kbd> lists every
  shortcut, filtered as you type.
- A lualine-style status bar and header, rounded dialogs, and a spinner while a model writes SQL.

### Connecting

URLs, psql- and mysql-style flags, or `quarry dbname user`. Passwords come from `~/.pgpass`, `~/.my.cnf`, the usual
environment variables or a `password_command`, and quarry asks when none of them work. TLS modes from `disable` to
`verify-full`, SSH tunnels through your own `ssh`, and saved connections with read-only mode, start-up SQL and a
tag colour (red for production). `\c name` switches to one without restarting.

### Shell completion

Tab completion for bash, zsh and fish that knows your setup: saved connections (with their URLs,
never their passwords), SQLite files, your themes, output formats, TLS modes, hosts from
`~/.ssh/config`, and the hosts, users and databases your saved connections use. The AUR packages
install it; elsewhere add `source <(quarry --completions zsh)` (or `bash`, or
`quarry --completions fish | source`) to your shell's startup file.

### Staying safe

`DROP`, `TRUNCATE` and `DELETE` or `UPDATE` without a `WHERE` ask first. `--readonly` blocks writes in quarry and at
the server. Statements with passwords stay out of history, `--save` never writes a password to the config, and
every file quarry writes is readable only by you.

### Asking a model for SQL

`\llm top 10 customers by revenue` writes SQL for your schema and leaves it for you to review; it never runs anything
by itself. Set it up once with `quarry --setup-llm`: an Anthropic API key, any OpenAI-compatible server (OpenAI,
OpenRouter, Ollama, LM Studio), or the Claude Code or Codex CLI you're already signed in to. Only your question and
the schema are sent, never rows.

### Themes and icons

27 built-in themes shared by the REPL and the TUI, among them Tokyo Night, Catppuccin, Gruvbox, Dracula, Nord,
Solarized, Rosé Pine and Kanagawa, plus your own TOML palettes and any base16 or base24 scheme.

Nerd Font icons throughout: database logos in the prompt and status bar, and icons for tables, views, keys, columns
and functions in the explorer, tabs and completion menus. `icons = "unicode"` or `"ascii"` (or `--icons`) for
terminals without a Nerd Font. `transparent = true` lets a translucent terminal show through the TUI.

### Getting started with the config

`quarry --default-config` prints the whole config file with every option explained, ready to save and edit.

### Install

- **Arch Linux:** `quarry-sql-bin` (prebuilt) or `quarry-sql` (from source) from the AUR, e.g. `paru -S quarry-sql-bin`.
- **Other Linux (x86_64):** download `quarry-<version>-x86_64-unknown-linux-gnu.tar.gz` from this release.
- **Anywhere with Rust:** `cargo install --git https://github.com/mah3uz/quarry`.
