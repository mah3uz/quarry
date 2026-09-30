pub mod pager;
pub mod sink;

use std::sync::Arc;
use std::time::Duration;

use crate::db::{Backend, Column, Row, Summary};
use crate::theme::{ColorDepth, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableFormat {
    /// `+---+` borders + `|` separators (psql / mysql client look).
    Ascii,
    /// psql default: header underline with `-+-`, no outer border.
    Psql,
    /// Unicode box drawing with rounded corners (default).
    Rounded,
    /// Unicode single line.
    Unicode,
    /// Unicode double line.
    Double,
    /// Only column gaps and a header rule.
    Minimal,
    /// Whitespace-separated, no header rule.
    Plain,
    /// `---- ----` header rule (tabulate "simple").
    Simple,
    /// GitHub-flavoured markdown.
    Markdown,
    Csv,
    Tsv,
    Json,
    JsonLines,
    Html,
    Vertical,
    SqlInsert,
    SqlUpdate,
}

impl TableFormat {
    pub const ALL: &'static [(&'static str, TableFormat)] = &[
        ("rounded", TableFormat::Rounded),
        ("psql", TableFormat::Psql),
        ("ascii", TableFormat::Ascii),
        ("unicode", TableFormat::Unicode),
        ("double", TableFormat::Double),
        ("minimal", TableFormat::Minimal),
        ("plain", TableFormat::Plain),
        ("simple", TableFormat::Simple),
        ("markdown", TableFormat::Markdown),
        ("csv", TableFormat::Csv),
        ("tsv", TableFormat::Tsv),
        ("json", TableFormat::Json),
        ("jsonl", TableFormat::JsonLines),
        ("html", TableFormat::Html),
        ("vertical", TableFormat::Vertical),
        ("sql-insert", TableFormat::SqlInsert),
        ("sql-update", TableFormat::SqlUpdate),
    ];

    pub fn parse(s: &str) -> Option<TableFormat> {
        let s = s.trim().to_ascii_lowercase();
        let alias = match s.as_str() {
            "github" | "md" | "pipe" => "markdown",
            "fancy_grid" | "fancy" | "round" => "rounded",
            "grid" | "mysql" => "ascii",
            "psql_unicode" | "box" => "unicode",
            "double_grid" => "double",
            "jsonlines" | "ndjson" => "jsonl",
            "sql_insert" => "sql-insert",
            "sql_update" => "sql-update",
            "expanded" => "vertical",
            other => other,
        };
        Self::ALL.iter().find(|(n, _)| *n == alias).map(|(_, f)| *f)
    }

    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(_, f)| *f == self).map(|(n, _)| *n).unwrap_or("rounded")
    }

    /// Machine formats are never colored, truncated or auto-verticalized.
    pub fn is_machine(self) -> bool {
        matches!(
            self,
            TableFormat::Csv | TableFormat::Tsv | TableFormat::Json | TableFormat::JsonLines | TableFormat::Html
                | TableFormat::SqlInsert | TableFormat::SqlUpdate
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Expanded {
    On,
    #[default]
    Off,
    /// Vertical when the table would be wider than the terminal.
    Auto,
}

#[derive(Clone)]
pub struct OutputOptions {
    pub format: TableFormat,
    pub expanded: Expanded,
    pub null_string: String,
    /// Truncate cell display to this many columns (None = unlimited). Machine formats ignore it.
    pub max_field_width: Option<usize>,
    pub terminal_width: usize,
    pub color: ColorDepth,
    pub theme: Arc<Theme>,
    pub backend: Backend,
    /// Target table for sql-insert / sql-update (defaults to `table`).
    pub table_name: Option<String>,
    /// Right-align numeric columns.
    pub align_numbers: bool,
}

/// Renders a result set to a string (with trailing newline). Colors only when `color != None`
/// and the format is not a machine format.
pub fn render(_columns: &[Column], _rows: &[Row], _opts: &OutputOptions) -> String {
    todo!()
}

/// Status line like `3 rows in set · 12.4 ms` / `Query OK, 1 row affected · 3 ms`.
pub fn render_status(_summary: &Summary, _row_count: usize, _elapsed: Option<Duration>, _opts: &OutputOptions) -> String {
    todo!()
}
