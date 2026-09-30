---
title: 'Quick start'
description: 'A ten-minute tour of quarry using a SQLite file, no server needed.'
---

# Quick start


This tour uses a SQLite file, so you don't need a database server. Everything you learn works the
same on PostgreSQL and MySQL.

## 1. Open a database

1. Start quarry with a file name. A file ending in `.db` is created if it doesn't exist:

   ```sh
   quarry tour.db
   ```

2. You see a short banner and a two-line prompt that shows where you are connected:

   ```
   quarry 0.1.0 · SQLite 3.53.2
   Type \? for help · \tui for the full-screen interface · Tab to complete · Ctrl-D to quit
   ╭─ SQLite ▸ tour.db
   ╰─❯
   ```


## 2. Create some data

Type these statements. A statement runs when it ends with `;`, so you can spread it over several
lines by pressing <kbd>Enter</kbd>:

```sql
create table books (
  id integer primary key,
  title text not null,
  author text,
  year integer
);

insert into books (title, author, year) values
  ('Dune', 'Frank Herbert', 1965),
  ('Neuromancer', 'William Gibson', 1984),
  ('The Left Hand of Darkness', 'Ursula K. Le Guin', 1969);
```

## 3. Query it

```sql
select title, year from books order by year;
```

```
╭───────────────────────────┬──────╮
│ title                     │ year │
├───────────────────────────┼──────┤
│ Dune                      │ 1965 │
│ The Left Hand of Darkness │ 1969 │
│ Neuromancer               │ 1984 │
╰───────────────────────────┴──────╯
3 rows · 0.31 ms
```

Try completion. Type `select * from bo` and the menu offers `books`; press <kbd>Tab</kbd> to take it.
Then continue with ` b where b.` and the menu lists the columns of `books`, because quarry knows
`b` is an alias for it.

## 4. Look around

These are **special commands**. They start with a backslash and run as soon as you press
<kbd>Enter</kbd>, with no `;` needed:

| Type | To |
|---|---|
| `\dt` | list tables |
| `\d books` | describe the `books` table |
| `\x` | switch to vertical output (run it again to cycle) |
| `\T csv` | print results as CSV (`\T` alone lists all formats) |
| `\?` | see every command |

## 5. Try the full-screen interface

Type `\tui`. The same connection opens in the TUI:

- The **explorer** on the left lists your tables. Press <kbd>Alt</kbd>+<kbd>0</kbd> to focus it,
  then <kbd>Enter</kbd> on `books` to browse its rows.
- The **editor** on the right runs the statement under the cursor with
  <kbd>Ctrl</kbd>+<kbd>Enter</kbd>, or everything with <kbd>F5</kbd>.
- <kbd>Ctrl</kbd>+<kbd>P</kbd> opens the command palette, and <kbd>F1</kbd> lists every shortcut.

Quit with <kbd>Ctrl</kbd>+<kbd>Q</kbd>. Leaving the TUI ends quarry; it doesn't go back to the REPL.

## 6. Use it in a script

The same binary runs SQL non-interactively and prints machine-friendly output:

```sh
quarry tour.db -e "select * from books" -F json
```

## 7. Connect to a server

Give quarry a URL instead of a file:

```sh
quarry postgres://me@localhost/app
quarry mysql://root@127.0.0.1:3306/shop
```

If a password is needed, quarry asks for one.

::: tip
Save a connection you use often with `--save`, then open it by name:
`quarry postgres://me@localhost/app --save app`, and next time just `quarry app`.
:::

## Where next

- [Connecting to a database](/guides/connecting) covers every way to reach a server.
- [Using the REPL](/guides/repl) and [Using the TUI](/guides/tui) go deeper into each interface.
- [Special commands](/reference/special-commands) lists everything you can type after `\`.
