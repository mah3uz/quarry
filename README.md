<p align="center"><img src="banner.svg" alt="quarry: a fast, beautiful SQL client for the terminal, for PostgreSQL, MySQL / MariaDB and SQLite"></p>

# quarry
### A Modern full-featured SQL client, smart and fast 🏎️💨 that lives in your terminal.

Point it at **PostgreSQL**, **MySQL**,  **MariaDB** or **SQLite** and you get a REPL in the spirit of `pgcli` and `mycli`,
with completion.

Want to look around, `\tui` opens the same connection full screen: a schema explorer,
tabbed editors, a results grid built for large results, and a table view where your edits are
staged and shown as SQL before anything is written.

It's a single binary with TLS built in and SQLite bundled. It asks before a `DROP` or a `DELETE`
without `WHERE`, keeps passwords out of history, and with `\llm` a model can draft a query for you
to review before it runs.

**Documentation: [quarry.asmechanics.com](https://quarry.asmechanics.com)**

## Install

```sh
paru -S quarry-sql-bin                                  # Arch Linux (AUR); or quarry-sql to build it
brew install mah3uz/tap/quarry                          # macOS, or Linux with Homebrew
cargo install --git https://github.com/mah3uz/quarry    # anywhere with Rust
```

Or download a Linux build from the [latest release](https://github.com/mah3uz/quarry/releases/latest).
There's nothing else to install: TLS is built in and SQLite is bundled. Icons need a
[Nerd Font](https://www.nerdfonts.com), or set `icons = "unicode"`.

More in [Installation](https://quarry.asmechanics.com/start/installation), including
[shell completion](https://quarry.asmechanics.com/start/installation#shell-completion).

## Use

```sh
quarry postgres://me@localhost/app       # the REPL
quarry --tui mysql://root@127.0.0.1/shop  # the full-screen TUI (or \tui from the REPL)
quarry data.db                           # a SQLite file
quarry                                   # the TUI, starting on your saved connections
```

In the REPL, end statements with `;`, press <kbd>Tab</kbd> to complete and `\?` for help. In the
TUI, <kbd>Ctrl</kbd>+<kbd>Enter</kbd> runs a statement, <kbd>Ctrl</kbd>+<kbd>P</kbd> opens the
command palette and <kbd>F1</kbd> lists every key.

`quarry --default-config > ~/.config/quarry/config.toml` gives you a fully commented config to
customise.

The [quick start](https://quarry.asmechanics.com/start/quick-start) walks through a first session.
Then:

| To | Read |
|---|---|
| Connect with URLs, flags, saved connections, `~/.pgpass`, TLS or SSH | [Connecting](https://quarry.asmechanics.com/guides/connecting) |
| Use the REPL: completion, special commands, output | [Using the REPL](https://quarry.asmechanics.com/guides/repl) |
| Use the TUI: explorer, editor, results, consoles, mouse | [Using the TUI](https://quarry.asmechanics.com/guides/tui) |
| Browse and edit table data | [Browsing and editing tables](https://quarry.asmechanics.com/guides/editing-data) |
| Script it: CSV, JSON, exit codes | [Scripts, exports and pipes](https://quarry.asmechanics.com/guides/scripting) |
| Ask a model to write SQL (`\llm`) | [Asking a model for SQL](https://quarry.asmechanics.com/guides/ai) |
| Change keys, or edit with vim keys | [Key bindings and vim mode](https://quarry.asmechanics.com/advanced/keybindings) |
| Pick or make a theme | [Themes](https://quarry.asmechanics.com/advanced/themes) |
| Look up an option, flag or command | [Configuration](https://quarry.asmechanics.com/reference/config), [command line](https://quarry.asmechanics.com/reference/cli), [special commands](https://quarry.asmechanics.com/reference/special-commands) |
| Fix something | [Troubleshooting](https://quarry.asmechanics.com/help/troubleshooting) |

## Development

You need Rust (stable) and, for the task runner, [`just`](https://github.com/casey/just).

```sh
git clone https://github.com/mah3uz/quarry && cd quarry
just            # list the tasks
just run        # run from source
just check      # lint and test
```

Tests that need a server use a local PostgreSQL (`postgres@127.0.0.1:5432`) and MySQL
(`root@127.0.0.1:3306`), or the URLs in `QUARRY_TEST_PG` and `QUARRY_TEST_MYSQL`, and skip when
they can't reach one. They only create and drop `quarry_test_*` databases. The docs site lives in `docs/`
(`just docs` previews it).

See [Contributing](https://quarry.asmechanics.com/help/contributing) for the project layout, the
generated artwork and the documentation site.

## License

[MIT](LICENSE)
