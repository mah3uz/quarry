# quarry

A fast, good-looking SQL client for **PostgreSQL**, **MySQL / MariaDB** and **SQLite**, written in Rust.
It comes in two forms, both in one binary:

- **A command-line REPL**, in the tradition of `pgcli`, `mycli` and `litecli`: context-aware completion,
  syntax highlighting, special commands, pagers, favourites and more.
- **A full-screen TUI**: a schema explorer, tabbed query editors, a results grid built for large result
  sets, a table browser with staged edits, structure and DDL views, an explain-plan viewer, a
  server-activity monitor and a command palette.

Colour themes are shared by the CLI and the TUI: Tokyo Night, Catppuccin, Gruvbox, Dracula, Nord,
Solarized, Rosé Pine, Kanagawa, Everforest, GitHub, One Dark, Monokai, Ayu, Nightfox and Material,
plus your own TOML themes and any base16/base24 scheme.

```
quarry postgres://me@localhost/app       # REPL
quarry --tui mysql://root@127.0.0.1/shop  # TUI
quarry data.db                           # SQLite file
quarry                                   # TUI with the connection manager
```

## Install

```
cargo install --path .
```

TLS uses rustls and SQLite is bundled, so there are no system libraries to install. SSH tunnels use
your system `ssh` binary.

## Connecting

| Form | Example |
|---|---|
| URL | `postgres://user:pass@host:5432/db?sslmode=require`, `mysql://root@127.0.0.1/shop`, `mariadb://…` |
| SQLite | `sqlite:///abs/path.db`, `sqlite:rel.db`, `file.db`, `:memory:` |
| Flags | `-h host -p port -u user -d db -S socket --backend mysql` (pgcli- and mycli-style short flags both work) |
| Saved | `quarry prod` uses `[connections.prod]` from the config; `quarry <url> --save prod` stores one |
| SSH | `--ssh deploy@bastion:22 [--ssh-key ~/.ssh/id_ed25519]` |

Passwords are taken from the URL, `PGPASSWORD` / `MYSQL_PWD`, `~/.pgpass`, `~/.my.cnf`, or a
`password_command` in a saved connection (for example `pass show db/prod`). If none of those work,
quarry prompts for one. Passwords are never shown, logged or written to history.

TLS modes are `disable`, `prefer` (the default), `require`, `verify-ca` and `verify-full`, set with
`--ssl-mode` / `--ssl-ca` / `--ssl-cert` / `--ssl-key` or `?sslmode=` in the URL.

## The REPL

- **Completion** that knows its context: columns of the tables in scope (aliases included, even
  before `FROM` has been typed), tables after `FROM`/`JOIN`, JOIN clauses suggested from foreign
  keys, `SELECT *` expansion, schemas, databases, functions with their signatures, types after `::`,
  special commands, favourites and file paths. Matching is fuzzy (`ui` finds `user_id`), and a menu
  pops up as you type.
- **Highlighting** from the same lexer the server dialect uses, including dollar quotes, backticks
  and `E''` strings. Matching brackets are highlighted.
- **Multi-line editing**: a statement runs when it ends with `;` or `\G`. Trigger and procedure
  bodies (`BEGIN … END`) and `$$` blocks are handled without `DELIMITER`. `Alt-Enter` runs the buffer
  immediately.
- **Safety**:
  - Statements such as `DROP`, `TRUNCATE`, and `DELETE`/`UPDATE` without a `WHERE` ask for
    confirmation first; the rules are configurable.
  - `--readonly` blocks writes both in the client and at the server session.
  - Statements that contain credentials are never saved to history.
- **Large results**: rows stream from the server. Past `row_limit` rows quarry asks before fetching
  the rest, and `Ctrl-C` cancels at the server (Postgres cancel request, MySQL `KILL QUERY`, SQLite
  interrupt).
- **Errors** show SQLSTATE or the error number, and a caret under the failing position.
- **Keys**: `F2` toggles smart completion, `F3` multi-line mode, `F4` vi/emacs mode. `Ctrl-R` searches
  history, and hints are suggested fish-style from history.

### Special commands

`\?` lists them all. The families:

| | |
|---|---|
| Info | `\l` `\d [pattern]` `\d+` `\dt` `\dv` `\dm` `\di` `\ds` `\df` `\dn` `\du` `\dT` `\dx` `\dp` `\sf` `\sv` `\conninfo` `\s`/`status`, plus SQLite's `.tables` `.schema` `.indexes` `.databases` |
| Output | `\x [on/off/auto]` `\G` `\T format` `\timing` `\pager cmd` `\nopager` `tee file` `notee` `\o file` `\| command` |
| Query | `\e` (external editor) `\i file` `\watch 2 query` `\format` `\explain [analyze] query` `\export csv file query` `\clip` `delimiter //` |
| Favourites | `\f` `\f name args…` `\fs name query` `\fd name`, with `$1`, `$*` and `${name}` placeholders |
| Session | `\c db-or-url` `use db` `\readonly` `\theme name` `\prompt fmt` `\refresh` `\! shell` `\tui` `\q` |

Output formats: `rounded` (the default), `psql`, `ascii`, `unicode`, `double`, `minimal`, `plain`,
`simple`, `markdown`, `csv`, `tsv`, `json`, `jsonl`, `html`, `vertical`, `sql-insert` and `sql-update`.
Wide results switch to vertical layout automatically (`\x auto`).

### Batch mode

```
quarry app.db -e "select * from users" -F csv > users.csv
quarry postgres://… -f migration.sql
cat script.sql | quarry mysql://…
```

When stdout is not a terminal the output is TSV. The exit code is non-zero if a statement fails, and a
script stops at the first error unless `--continue-on-error` is given.

## The TUI

Launch it with `quarry --tui <target>`, with `\tui` from the REPL, or with plain `quarry`.

| Area | What it does |
|---|---|
| Explorer | Connections → databases → schemas → tables, views, functions → columns. Keys work on the selected node: `/` filters, `s` shows structure, `g`+`s/i/u/d/c/x/n` generates SELECT, INSERT, UPDATE, DELETE, CREATE, DROP or COUNT scripts, and `i` inserts the name into the editor. |
| Query tabs | Editor with highlighting, completion as you type, bracket matching, undo and auto-indent. `Ctrl+Enter` runs the statement under the cursor, `F5` runs everything, and `Esc` cancels. |
| Results grid | Virtualized, so millions of rows scroll smoothly. Header stays in place. Types are colour-coded. Select with `v`/`V`, copy as TSV, CSV, JSON, Markdown or SQL, search with `/`, resize columns, and press `Enter` to view a cell (JSON is pretty-printed). |
| Table browser | Pages load as you scroll. Filter with a `WHERE` (`f`, or `F` for the current cell's value) and sort by column (`s`). Edits, inserts and deletes are staged and shown in the grid; `Ctrl+S` shows the generated SQL and applies it in one transaction. |
| Structure | Columns, indexes, foreign keys, referencing tables, constraints, triggers and highlighted DDL. |
| Explain | Plan tree with bars showing how cost or time is shared, plus details per node. Analyze runs inside a transaction that is rolled back. |
| Activity | Live server sessions, with kill (Postgres and MySQL). |
| Everything else | Command palette (`Ctrl+P`), go-to-table (`Ctrl+G`), live theme picker (`Ctrl+Y`), history (`Ctrl+R`), favourites, open/save `.sql` files, export results, commit/rollback, several connections at once, mouse support. |

`F1` shows every shortcut.

## Themes

`\theme` in the REPL or `Ctrl+Y` in the TUI (which previews live). Built-in themes:

```
tokyo-night tokyo-night-storm tokyo-night-day catppuccin-mocha catppuccin-macchiato catppuccin-frappe
catppuccin-latte gruvbox-dark gruvbox-light dracula nord one-dark solarized-dark solarized-light
rose-pine rose-pine-moon rose-pine-dawn kanagawa everforest-dark everforest-light github-dark
github-light monokai ayu-dark nightfox material-ocean ansi
```

You can also put your own themes in `~/.config/quarry/themes/`:

- `mytheme.toml` sets `inherits = "nord"` plus any colour field, for example `keyword = "#ff79c6"`.
- Any base16 or base24 `*.yaml` scheme is mapped to a full theme automatically.

Colours fall back to 256 or 16 colours on terminals without truecolor, and `NO_COLOR` is respected.

## Configuration

`~/.config/quarry/config.toml` is created with comments on first run. It covers the theme, table
format, null string, row limit, pager, destructive-warning rules, prompt format, keyword casing,
vi mode, completion behaviour and saved connections. Favourites are kept in `favorites.toml`, and
history in `~/.local/share/quarry/`, readable only by you.

## Development

```
cargo test                    # unit + integration tests
QUARRY_TEST_PG=postgres://postgres@127.0.0.1/postgres QUARRY_TEST_MYSQL=mysql://root@127.0.0.1 cargo test
```

The integration tests create their own `quarry_test_*` databases and drop them afterwards. If a
server is unreachable, its tests are skipped.
