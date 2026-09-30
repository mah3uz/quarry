---
title: Scripts, exports and pipes
description: Run SQL non-interactively, choose an output format, use exit codes, and export results.
---

quarry works well in scripts and pipelines. It runs **non-interactively** whenever you give it
`-e`, `-f`, or input on stdin.

```sh
quarry app.db -e "select * from users" -F csv > users.csv
quarry postgres://me@localhost/app -f migration.sql
cat report.sql | quarry mysql://root@127.0.0.1/shop
```

## Where the SQL comes from

| Option | Runs |
|---|---|
| `-e "sql"` | That SQL. Repeat `-e` to run several, in order. |
| `-f file.sql` | Every statement in the file, after any `-e`. |
| stdin | Everything piped in. Only read when there is no `-e` or `-f`. |

A single `-e` can hold several statements separated by `;`. It can also be one special command,
such as `-e "\dt"` or `-e "describe users"`.

In a file or on stdin, lines that start with `\` (or `.` on SQLite), and `delimiter` lines, are run
as special commands when they appear between statements. Everything else is sent to the server as
SQL. Word forms such as `use db` are sent as SQL there too.

## Output

| Situation | Format |
|---|---|
| You pass `-F` / `--format` | That format |
| stdout is a pipe or a file | TSV, with a header row |
| stdout is a terminal | Your configured `table_format` (a rounded table by default) |

```sh
quarry app.db -e "select id, email from users limit 2"
```

```
id	email
1	user1@example.com
2	user2@example.com
```

The machine formats, `csv`, `tsv`, `json`, `jsonl`, `html`, `sql-insert` and `sql-update`, print
only the data. The table formats also print a status line such as `3 rows`. See
[Output formats](/reference/output-formats/) for all of them.

In a script, quarry doesn't page, truncate long values, switch to vertical layout, ask before large
results, or ask for confirmation before destructive statements. Colour is off when stdout isn't a
terminal; `--no-color` or `NO_COLOR=1` turns it off in the REPL as well.

## Errors and exit codes

- A failing statement stops the script and quarry **exits with status 1**. Errors go to stderr.
- `--continue-on-error` keeps going after a failure. The exit status is still 1 if anything failed.
- A connection failure also exits with status 1.
- On success the exit status is 0.

```sh
if ! quarry "$DATABASE_URL" -f migrate.sql; then
  echo "migration failed" >&2
  exit 1
fi
```

Add `-w` (`--no-password`) in automation so quarry never waits for a password prompt.

## From the REPL

You don't need a separate command to save results from an interactive session:

| Command | Does |
|---|---|
| `\export csv out.csv select …` | Writes the complete result of the query to a file, in any format |
| `\o out.txt` | Sends the **next** result to a file instead of the screen |
| `tee out.txt` | Copies **every** result to a file until `notee`. Appends by default; `tee -o` overwrites. |
| `\| wc -l` | Pipes the next result into a shell command |

Files written by quarry are readable only by you (mode 600).

:::caution
In an interactive session, `\export` still asks before fetching more than `row_limit` rows, and
answering no truncates the export. For very large exports, run the query from a script with `-F`,
or set `row_limit = 0` to turn the question off.
:::

## Logging every statement

Set `log_queries = true` in the config and quarry appends one line per statement to
`~/.local/share/quarry/quarry.log`:

```
2026-09-30 12:04:11	0.8 ms	ok	select * from users where id = 1
2026-09-30 12:04:15	1.2 ms	error: no such column: nmae	select nmae from users
```

The fields are tab-separated: time, duration, outcome, and the statement with newlines flattened.
It covers the REPL and scripts, not the TUI. Statements that look like they contain passwords are
left out.

## Related

- [Command-line options](/reference/cli/)
- [Output formats](/reference/output-formats/)
