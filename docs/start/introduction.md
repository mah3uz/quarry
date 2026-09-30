---
title: 'What is quarry?'
description: 'quarry is a SQL client for the terminal with a smart REPL and a full-screen TUI, for PostgreSQL, MySQL / MariaDB and SQLite.'
---

# What is quarry?

quarry is a SQL client that runs in your terminal. It connects to **PostgreSQL**, **MySQL**,
**MariaDB** and **SQLite**, and gives you two ways to work with them from one binary:

- **The REPL**: a command line in the tradition of `pgcli`, `mycli` and `litecli`. You type SQL and
  get a table back. Completion knows your tables and columns, statements are highlighted as you type,
  and psql-style commands such as `\d users` work on every database.
- **The TUI**: a full-screen interface with a schema explorer, tabbed query editors, a results grid,
  a table browser where you can stage and apply edits, structure and DDL views, an explain-plan
  viewer and a command palette.

You can move from the REPL to the TUI at any time with `\tui`, keeping the same connection.

```sh
quarry postgres://me@localhost/app        # REPL
quarry --tui mysql://root@127.0.0.1/shop  # TUI
quarry data.db                            # a SQLite file
quarry                                    # the TUI's connection manager
```

## Who it is for

quarry is for people who spend time in a terminal and want one tool for all three databases:

- **Developers** checking data while they work, with completion that saves typing.
- **Anyone exploring a database** they did not design, using the explorer, `\d` and the structure view.
- **Scripts and pipelines** that need query results as CSV, JSON, TSV or Markdown.

## What makes it different

- **One tool, three databases.** The same keys, commands and output formats everywhere. The `\d`
  family is translated to each database's catalog.
- **Nothing else to install.** TLS is built in and SQLite is bundled. SSH tunnels use your system
  `ssh`.
- **Careful by default.** Destructive statements ask first, read-only mode is enforced at the server
  as well as in quarry, and query results stream so you can cancel a runaway query.
- **Help from a model when you want it.** `\llm` writes SQL for your schema and leaves it for you to
  review. It never runs anything by itself.

## How these docs are organised

| Section | Read it when |
|---|---|
| [Start here](/start/installation) | You are installing quarry or using it for the first time. |
| [Guides](/guides/connecting) | You want to learn one part of quarry properly: connecting, the REPL, the TUI, scripting. |
| [Advanced](/advanced/saved-connections) | You want saved connections, secrets, tunnels, favourites or your own theme. |
| [Reference](/reference/cli) | You need the exact name of a flag, command, key or config option. |
| [Help](/help/troubleshooting) | Something is not working, or you are coming from pgcli, mycli or litecli. |

Ready? Start with [Installation](/start/installation), then take the [Quick start](/start/quick-start).
