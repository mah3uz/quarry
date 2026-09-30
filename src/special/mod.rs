pub mod favorites;
pub mod introspect;
mod parse;
mod registry;
#[cfg(test)]
mod tests;

use crate::db::{Backend, ResultSet};
use crate::output::{Expanded, TableFormat};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    General,
    Info,
    Output,
    Query,
    Favorites,
    Connection,
}

pub struct CommandSpec {
    /// First name is canonical; the rest are aliases (`\dt`, `.tables`).
    pub names: &'static [&'static str],
    pub syntax: &'static str,
    pub description: &'static str,
    pub category: Category,
    /// Only meaningful for these backends (empty = all).
    pub backends: &'static [Backend],
}

/// Relation kinds for `\dt \dv \dm \dE` style listings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelFilter {
    Tables,
    Views,
    MaterializedViews,
    ForeignTables,
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Special {
    Help(Option<String>),
    Quit,

    // --- introspection (special::introspect::run) ---
    /// `\l`
    ListDatabases { pattern: Option<String>, verbose: bool },
    /// `\d [pattern]`, `\d+`, `describe t`, `desc t`
    Describe { pattern: Option<String>, verbose: bool },
    /// `\dt \dv \dm \dE`, `.tables`, `.views`, `show tables`
    ListRelations { kind: RelFilter, pattern: Option<String>, verbose: bool },
    /// `\di`, `.indexes [table]`
    ListIndexes { pattern: Option<String>, verbose: bool },
    /// `\ds`
    ListSequences { pattern: Option<String> },
    /// `\df`
    ListFunctions { pattern: Option<String>, verbose: bool },
    /// `\dn`
    ListSchemas { pattern: Option<String> },
    /// `\du`, `\dg`
    ListRoles { pattern: Option<String> },
    /// `\dT`
    ListTypes { pattern: Option<String> },
    /// `\dx`
    ListExtensions { pattern: Option<String> },
    /// `\dp`, `\z`
    ListPrivileges { pattern: Option<String> },
    /// `\sf func`, `\sv view`
    ShowSource { name: String, kind: String },
    /// `.schema [table]` — CREATE statements.
    Schema { pattern: Option<String> },
    /// `\s`, `status`, `.status`
    Status,
    /// `\conninfo`
    ConnInfo,

    // --- session / REPL state ---
    /// `\c [db|url]`, `\connect`, `use db`, `.open file`
    Connect { target: Option<String> },
    /// `\x [on|off|auto]`
    Expanded(Option<Expanded>),
    /// `\timing [on|off]`
    Timing(Option<bool>),
    /// `\T [format]`, `\tableformat`, `.mode`
    TableFormat(Option<TableFormat>),
    /// `\pager [cmd]`, `pager`
    Pager(Option<String>),
    NoPager,
    /// `tee [-o] file`, `.output`
    Tee { path: String, overwrite: bool },
    NoTee,
    /// `\o [-o] file`, `\once`, `.once`
    Once { path: String, overwrite: bool },
    /// `\| cmd`, `\pipe_once cmd`
    PipeOnce { command: String },
    /// `\e [file]` or `query \e`
    Edit { file: Option<String>, query: Option<String> },
    /// `\i file`, `source file`, `.read file`
    Source { path: String },
    /// `query \clip`
    Clip { query: Option<String> },
    /// `\watch [sec] [-c] query` or `query \watch sec`
    Watch { seconds: f64, clear: bool, query: Option<String> },
    /// `\f [name [args…]]`, `\n`
    Favorite { name: Option<String>, args: Vec<String> },
    /// `\fs name query`, `\ns`
    FavoriteSave { name: String, query: String },
    /// `\fd name`, `\nd`
    FavoriteDelete { name: String },
    /// `\refresh`, `rehash`, `\#`
    Refresh,
    /// `\! cmd`, `system cmd`
    System { command: String },
    /// `\echo text`
    Echo(String),
    /// `\R` / `prompt` / `\prompt [fmt]`
    Prompt(Option<String>),
    /// `delimiter //`
    Delimiter(String),
    /// `\theme [name]`
    Theme(Option<String>),
    /// `\format [query]` — pretty-print SQL.
    Format { query: Option<String> },
    /// `\explain [analyze] query`
    Explain { analyze: bool, query: String },
    /// `\readonly [on|off]`
    ReadOnly(Option<bool>),
    /// `\tui`
    Tui,
    /// `\export format file query` — write a query's full result to a file.
    Export { format: TableFormat, path: String, query: String },
    /// `.load path`
    LoadExtension { path: String },
    /// `\history [n]`
    History(Option<usize>),
}

/// Output of an introspection command.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Titled {
    pub title: Option<String>,
    pub result: ResultSet,
    pub footer: Option<String>,
    /// Pre-formatted text (DDL, function source); rendered verbatim instead of a table.
    pub text: Option<String>,
}

pub fn registry() -> &'static [CommandSpec] {
    registry::COMMANDS
}

/// Finds a command by any of its names (`dt`, `\dt`, `.tables`, `help`).
pub fn lookup(name: &str) -> Option<&'static CommandSpec> {
    registry::lookup(name)
}

/// `\?` output: every command for `backend` grouped by category, or details for one `topic`.
pub fn help_text(topic: Option<&str>, backend: Backend) -> String {
    registry::help_text(topic, backend)
}

/// `None` → not a special command (treat as SQL). `Some(Err)` → special command with bad arguments.
/// Also recognises trailing suffix forms: `select 1 \e`, `select 1 \clip`, `select 1 \watch 2`.
pub fn parse(input: &str, backend: Backend) -> Option<Result<Special, String>> {
    parse::parse(input, backend)
}

/// Whether the REPL should submit `input` on Enter without a statement terminator.
pub fn submits_immediately(input: &str, backend: Backend) -> bool {
    parse::parse(input, backend).is_some()
}
