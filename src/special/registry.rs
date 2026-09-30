use super::{Category, CommandSpec};
use crate::db::Backend;

const PG: &[Backend] = &[Backend::Postgres];
const PG_MY: &[Backend] = &[Backend::Postgres, Backend::MySql];
const MY: &[Backend] = &[Backend::MySql];
const LITE: &[Backend] = &[Backend::Sqlite];
const ALL: &[Backend] = &[];

const fn cmd(
    names: &'static [&'static str],
    syntax: &'static str,
    description: &'static str,
    category: Category,
    backends: &'static [Backend],
) -> CommandSpec {
    CommandSpec { names, syntax, description, category, backends }
}

use Category::*;

pub(super) static COMMANDS: &[CommandSpec] = &[
    cmd(&["\\?", "help", "\\h", "\\help", ".help"], "\\? [command]", "Show help for all commands, or for one.", General, ALL),
    cmd(&["\\q", "quit", "exit", "\\quit", ".quit", ".exit"], "\\q", "Quit.", General, ALL),
    cmd(&["\\!", "system"], "\\! command", "Run a shell command.", General, ALL),
    cmd(&["\\echo"], "\\echo text", "Print text.", General, ALL),
    cmd(&["\\e", "\\edit"], "\\e [file] | query \\e", "Edit the query buffer (or a file) in $EDITOR.", General, ALL),
    cmd(&["\\i", "source", "\\ir", "\\.", ".read"], "\\i file", "Execute statements from a file.", General, ALL),
    cmd(&["\\history"], "\\history [n]", "Show the last n queries.", General, ALL),
    cmd(&["\\theme"], "\\theme [name]", "List themes, or switch to one.", General, ALL),
    cmd(&["\\R", "prompt", "\\prompt"], "\\R [format]", "Show or change the prompt format (\\u \\h \\p \\d \\n \\t \\T \\x).", General, ALL),
    cmd(&["\\refresh", "rehash", "\\#", "\\rehash"], "\\refresh", "Reload the catalog used for completion.", General, ALL),
    cmd(&["\\tui"], "\\tui", "Open the full-screen TUI on this connection.", General, ALL),
    cmd(&["\\l", "\\list", ".databases"], "\\l[+] [pattern]", "List databases (+: size, tablespace, description).", Info, ALL),
    cmd(&["\\d", "describe", "desc", "\\describe"], "\\d[+] [pattern]", "Describe a table, view or index; without a pattern list all relations.", Info, ALL),
    cmd(&["\\dt", ".tables"], "\\dt[+] [pattern]", "List tables.", Info, ALL),
    cmd(&["\\dv", ".views"], "\\dv[+] [pattern]", "List views.", Info, ALL),
    cmd(&["\\dm"], "\\dm[+] [pattern]", "List materialized views.", Info, PG),
    cmd(&["\\dE"], "\\dE[+] [pattern]", "List foreign tables.", Info, PG),
    cmd(&["\\di", ".indexes", ".indices"], "\\di[+] [pattern]", "List indexes (pattern matches index or table name).", Info, ALL),
    cmd(&["\\ds"], "\\ds [pattern]", "List sequences.", Info, PG_MY),
    cmd(&["\\df"], "\\df[+] [pattern]", "List functions and procedures.", Info, ALL),
    cmd(&["\\dn"], "\\dn [pattern]", "List schemas.", Info, ALL),
    cmd(&["\\du", "\\dg"], "\\du [pattern]", "List roles / users.", Info, PG_MY),
    cmd(&["\\dT"], "\\dT [pattern]", "List data types.", Info, PG),
    cmd(&["\\dx"], "\\dx [pattern]", "List extensions (MySQL: plugins).", Info, PG_MY),
    cmd(&["\\dp", "\\z"], "\\dp [pattern]", "List access privileges.", Info, PG_MY),
    cmd(&["\\sf"], "\\sf[+] function", "Show a function's definition.", Info, PG_MY),
    cmd(&["\\sv"], "\\sv[+] view", "Show a view's definition.", Info, ALL),
    cmd(&[".schema", "\\schema"], ".schema [pattern]", "Show CREATE statements.", Info, ALL),
    cmd(&["\\s", "status", ".status", "\\status"], "\\s", "Show connection and server status.", Info, ALL),
    cmd(&["\\conninfo"], "\\conninfo", "Show how you are connected.", Info, ALL),
    cmd(&["\\x", "\\expanded"], "\\x [on|off|auto]", "Toggle expanded (vertical) output.", Output, ALL),
    cmd(&["\\timing", "\\t"], "\\timing [on|off]", "Toggle query timing.", Output, ALL),
    cmd(&["\\T", "\\tableformat", ".mode"], "\\T [format]", "Show or change the table format.", Output, ALL),
    cmd(&["\\pager", "pager", "\\P"], "\\pager [command]", "Enable the pager, optionally setting its command.", Output, ALL),
    cmd(&["\\nopager", "nopager"], "\\nopager", "Disable the pager.", Output, ALL),
    cmd(&["tee", "\\tee", ".output"], "tee [-o] file", "Also write all results to a file (-o overwrites).", Output, ALL),
    cmd(&["notee", "\\notee"], "notee", "Stop writing results to a file.", Output, ALL),
    cmd(&["\\o", "\\once", ".once"], "\\o [-o] file", "Write the next result to a file instead of the screen.", Output, ALL),
    cmd(&["\\|", "\\pipe_once"], "\\| command", "Pipe the next result to a shell command.", Output, ALL),
    cmd(&["\\clip"], "query \\clip", "Copy the query (or the last query) to the clipboard.", Output, ALL),
    cmd(&["\\export"], "\\export format file query", "Write a query's full result to a file.", Output, ALL),
    cmd(&["\\watch", "watch"], "\\watch [sec] [-c] [query] | query \\watch [sec]", "Re-run a query every sec seconds (-c clears the screen).", Query, ALL),
    cmd(&["\\format"], "\\format [query]", "Pretty-print SQL.", Query, ALL),
    cmd(&["\\explain"], "\\explain [analyze] query", "Show the query plan as a tree.", Query, ALL),
    cmd(&["delimiter", "\\delimiter"], "delimiter string", "Change the statement delimiter.", Query, MY),
    cmd(&["\\readonly"], "\\readonly [on|off]", "Toggle read-only mode (refuses writes).", Query, ALL),
    cmd(&[".load", "\\load"], ".load path", "Load a SQLite extension.", Query, LITE),
    cmd(&["\\f", "\\n"], "\\f [name [args…]]", "List favorite queries, or run one with arguments.", Favorites, ALL),
    cmd(&["\\fs", "\\ns"], "\\fs name query", "Save a favorite ($1…$9, $* and ${name} are placeholders).", Favorites, ALL),
    cmd(&["\\fd", "\\nd"], "\\fd name", "Delete a favorite query.", Favorites, ALL),
    cmd(&["\\c", "\\connect", "use", "\\u", ".open"], "\\c [database|url]", "Connect to another database (no argument: reconnect).", Connection, ALL),
];

pub(super) fn lookup(name: &str) -> Option<&'static CommandSpec> {
    let name = name.trim();
    let bare = name.trim_start_matches(['\\', '.']);
    let bare = bare.strip_suffix('+').filter(|b| !b.is_empty()).unwrap_or(bare);
    COMMANDS.iter().find(|c| c.names.contains(&name)).or_else(|| {
        COMMANDS.iter().find(|c| c.names.iter().any(|n| n.trim_start_matches(['\\', '.']) == bare))
    })
}

fn available(c: &CommandSpec, backend: Backend) -> bool {
    c.backends.is_empty() || c.backends.contains(&backend)
}

fn category_title(c: Category) -> &'static str {
    match c {
        General => "General",
        Info => "Informational",
        Output => "Output",
        Query => "Query",
        Favorites => "Favorites",
        Connection => "Connection",
    }
}

pub(super) fn help_text(topic: Option<&str>, backend: Backend) -> String {
    if let Some(t) = topic.map(str::trim).filter(|t| !t.is_empty()) {
        return match lookup(t) {
            Some(c) => {
                let mut s = format!("{}\n    {}\n", c.syntax, c.description);
                if c.names.len() > 1 {
                    s.push_str(&format!("    Aliases: {}\n", c.names[1..].join(", ")));
                }
                if !c.backends.is_empty() {
                    let names: Vec<&str> = c.backends.iter().map(|b| b.name()).collect();
                    s.push_str(&format!("    Backends: {}\n", names.join(", ")));
                }
                s
            }
            None => format!("No help for '{t}'. Type \\? for a list of commands.\n"),
        };
    }
    let shown: Vec<&CommandSpec> = COMMANDS.iter().filter(|c| available(c, backend)).collect();
    let width = shown.iter().map(|c| c.syntax.chars().count()).max().unwrap_or(0);
    let mut out = String::new();
    for cat in [General, Connection, Info, Output, Query, Favorites] {
        let cmds: Vec<&&CommandSpec> = shown.iter().filter(|c| c.category == cat).collect();
        if cmds.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(category_title(cat));
        out.push('\n');
        for c in cmds {
            let pad = width - c.syntax.chars().count();
            out.push_str(&format!("  {}{}  {}\n", c.syntax, " ".repeat(pad), c.description));
        }
    }
    out
}
