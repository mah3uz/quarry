---
title: 'Asking a model for SQL'
description: 'Use \llm to have a language model write SQL for your schema, with an API key, a local model, or the Claude Code or Codex CLI.'
---

# Asking a model for SQL

`\llm` (or `\ai`) turns a question into SQL for the database you're connected to:

```
\llm top 10 customers by revenue this month
```

quarry sends your question and a description of your schema to a model, then puts the SQL it
writes **in front of you to review**. It never runs anything by itself.

## Set it up once

```sh
quarry --setup-llm
```

The setup asks which provider to use and only the questions that apply to it. It sends a small
test request, then saves your answers. Run it again whenever you want to change something, and
press <kbd>Ctrl</kbd>+<kbd>C</kbd> at any step to leave your settings as they were.

| Provider | Choose it when | Before running the setup |
|---|---|---|
| **Anthropic API** | You have an Anthropic API key | Create a key in the [Claude Console](https://platform.claude.com/) |
| **Claude Code** | You use Claude Code and want `\llm` to use its login | Install `claude` and sign in (`claude auth login`) |
| **OpenAI-compatible API** | You use OpenAI, OpenRouter, a local Ollama or LM Studio, or any other `/chat/completions` server | Have the key ready, or start the local server |
| **Codex** | You use Codex and want `\llm` to use its login | Install `codex` and sign in (`codex login`) |

What each provider asks for:

- **Anthropic API:** your key, then the model: Claude Opus 5.5 (the default), Sonnet 5.5, Haiku 4.5,
  or any other model id.
- **Claude Code:** the model: Claude Code's default, or Opus, Sonnet or Haiku. quarry runs
  `claude -p` with no tools, so it can only reply with text.
- **OpenAI-compatible API:** the server (OpenAI, OpenRouter, Ollama, LM Studio, or any base URL), a
  key if the server needs one, and a model picked from the server's own list. Type to filter it.
- **Codex:** the model (leave it empty for Codex's default). quarry runs `codex exec` in a read-only
  sandbox.

Claude Code and Codex run from a temporary folder, so they don't read your project's `CLAUDE.md` or
`AGENTS.md`. Their usage counts against whichever account that CLI is signed in to.

## Using it

### In the REPL

```
╭─ PostgreSQL me@localhost:5432 ▸ shop
╰─❯ \llm customers who never ordered
⠹ Asking claude-opus-5-5…  2.1 s
This finds customers with no matching order.
Review the query below and press Enter to run it.
╰─❯ SELECT c.id, c.name FROM customers c LEFT JOIN orders o ON o.customer_id = c.id WHERE o.id IS NULL;
```

The SQL is waiting at the prompt. Read it, change it if you like, then run it.

::: tip
In multi-line mode <kbd>Enter</kbd> only runs a statement that ends with `;`. If the model left the
`;` off, type one, or press <kbd>Alt</kbd>+<kbd>Enter</kbd>.
:::

### In the TUI

Type `\llm your question` in the editor and run it with <kbd>Ctrl</kbd>+<kbd>Enter</kbd>, or choose
**Ask the model to write SQL…** in the command palette (<kbd>Ctrl</kbd>+<kbd>P</kbd>). The answer
replaces the `\llm` line (or is added at the end), and the model's explanation goes to the Messages
tab. Run it with <kbd>Ctrl</kbd>+<kbd>Enter</kbd> when you're happy with it.

While the model works, the results pane shows a spinner with your question and the time so far
(the REPL shows the same spinner on one line, and clears it when the answer arrives).

## What is sent

Only this goes to the model:

- your question,
- the database product and server version,
- the current database and search path,
- your schema: tables, views, columns with their types, primary keys and NOT NULL, and foreign keys.
  Very large schemas are cut at about 120,000 characters.

**No rows are ever sent.** Nor are indexes, defaults or comments.

The model is asked to answer with one SQL statement, to prefer read-only queries, and never to add
`DROP` or `TRUNCATE` unless you ask for it.

## API keys

Keys typed into the setup are saved in `~/.local/share/quarry/credentials.toml`, readable only by
you. They're kept out of `~/.config/quarry/`, so backing up or sharing your dotfiles doesn't share
your keys. Each key is tied to its server: a key saved for OpenRouter is never sent to Ollama or
anywhere else.

- **Anthropic:** `ANTHROPIC_API_KEY` (or `ANTHROPIC_AUTH_TOKEN`) in the environment takes precedence
  over a saved key, so you can skip saving one and just export it. `ANTHROPIC_BASE_URL` points
  requests at a gateway.
- **OpenAI-compatible:** only the saved key is used. `OPENAI_API_KEY` is deliberately not read,
  because the server is configurable and that key could otherwise be sent to a different service.
- **To remove a key,** delete its line from `credentials.toml`.

## Editing the settings by hand

The setup writes the `[llm]` section of `config.toml`:

```toml
[llm]
provider = "anthropic"      # anthropic | openai | claude-code | codex
model = "claude-opus-5-5"   # empty = the CLI's default (claude-code, codex)
# base_url = "http://localhost:11434/v1"   # openai provider only
```

Running `--setup-llm` changes only the `[llm]` settings in `config.toml`; your comments and other
settings stay. The previous file is kept as `config.toml.bak`.

## Troubleshooting

| Message | What to do |
|---|---|
| `no Anthropic API key` | Run `quarry --setup-llm`, or export `ANTHROPIC_API_KEY` |
| `` `claude` is not on PATH `` / `` `codex` is not on PATH `` | Install the CLI, or run the setup and pick another provider |
| `` `claude` failed `` / `` `codex` failed `` | Usually not signed in: run `claude auth login` or `codex login` |
| `Could not list models` during setup | The server isn't running or the key is wrong. You can still type a model name |
| A timeout (requests give up after 180 s) | The model or local server is too slow. Try a smaller model |
| `Claude declined this request` | A declined request is already retried on a fallback model, so this is the final answer. Rephrase the question |
