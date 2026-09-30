---
title: Coming from pgcli, mycli or litecli
description: What carries over from the dbcli tools, and what is different in quarry.
---

quarry is built to feel familiar if you've used `pgcli`, `mycli` or `litecli`. Most of your habits
carry over.

## What works the same

- **Connecting:** `quarry dbname user` like pgcli, the same short flags (`-h -p -u -d`, and `-U -P -D`),
  URLs, and the usual environment variables and password files (`PG*`, `~/.pgpass`, `MYSQL_*`,
  `~/.my.cnf`).
- **Commands:**
  - pgcli's `\d` family (`\dt`, `\dv`, `\di`, `\df`, `\dn`, `\du`, `\l`, …), on every database.
  - mycli's words: `use`, `source`, `tee`, `notee`, `pager`, `nopager`, `status`, `delimiter`,
    `rehash`, `system`, `prompt`.
  - litecli's dot commands: `.tables`, `.schema`, `.indexes`, `.databases`, `.mode`, `.read`,
    `.open`, `.load`, `.once`, `.output`.
- **Output:** `\x`, `\G`, `\T` (with the tabulate-style names `fancy_grid`, `grid`, `github`, `psql`),
  and the `less -SRXF` pager.
- **Favourites:** `\f`, `\fs`, `\fd` with `$1` and `$*` placeholders, and `\n`, `\ns`, `\nd` as
  aliases.
- **Safety:** destructive-statement confirmation.
- **Editing:** `\e` in your editor, `\watch`, `F2`–`F4` toggles, and history search with
  <kbd>Ctrl</kbd>+<kbd>R</kbd>.

## What's different

- **One tool for all three databases.** No more remembering which client you're in.
- **A full-screen TUI** is built in: `\tui` or `quarry --tui`.
- **No Python.** quarry is a single native binary. TLS is built in and SQLite is bundled.
- **One config file**, `~/.config/quarry/config.toml`, in TOML. Setting names are similar but not
  identical. See [Configuration file](/reference/config/). Your old `~/.config/pgcli/config` or
  `~/.myclirc` isn't read.
- **Saved connections** replace pgcli's `[alias_dsn]` and mycli's `[alias_dsn]`:
  `quarry URL --save NAME`, then `quarry NAME`.
- **`\t` is timing**, as in mycli, not psql's tuples-only mode.
- **`\R` sets the prompt**, and `prompt` does too.
- **Destructive warnings** are configured as a list of rules (`drop`, `unconditional_delete`, …)
  rather than on/off. See [Staying safe](/guides/safety/).
- **Themes** replace Pygments syntax styles. There are 27 built in, and you can use base16 schemes.
  See [Themes](/advanced/themes/).
- **`\llm`** asks a language model to write SQL for your schema. See
  [Asking a model for SQL](/guides/ai/).

## Settings that map across

| pgcli / mycli / litecli | quarry `[main]` |
|---|---|
| `smart_completion` | `smart_completion` |
| `multi_line` | `multi_line` |
| `vi` | `vi` |
| `keyword_casing` | `keyword_casing` |
| `table_format` | `table_format` |
| `destructive_warning` | `destructive_warning` (a list of rules) |
| `prompt` | `prompt` (same `\u \h \d` escapes, plus `\t \T \x \D \R`) |
| `row_limit` | `row_limit` |
| `less_chatty` | `less_chatty` |
| `log_file` | `log_queries = true` (fixed location in the data directory) |
| `syntax_style` | `theme` |
| `enable_pager`, `pager` | `enable_pager`, `pager` |
| `max_field_width` | `max_field_width` |
| `null_string` | `null_string` |
