mod machine;
pub mod pager;
pub mod sink;
mod table;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use crate::db::{Backend, Column, Row, Summary, Value};
use crate::special::Titled;
use crate::theme::{self, ColorDepth, Theme};

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

impl Default for OutputOptions {
    fn default() -> Self {
        OutputOptions {
            format: TableFormat::Rounded,
            expanded: Expanded::Off,
            null_string: "NULL".into(),
            max_field_width: None,
            terminal_width: 0,
            color: ColorDepth::None,
            theme: Arc::new(Theme::default()),
            backend: Backend::Postgres,
            table_name: None,
            align_numbers: true,
        }
    }
}

/// Renders a result set to a string (with trailing newline). Colors only when `color != None`
/// and the format is not a machine format.
pub fn render(columns: &[Column], rows: &[Row], opts: &OutputOptions) -> String {
    if columns.is_empty() {
        return String::new();
    }
    if opts.format.is_machine() {
        return machine::render(columns, rows, opts);
    }
    table::render(columns, rows, opts)
}

/// Display text of one value, with NULL replaced by `null`. Shared with the TUI grid.
pub fn cell_text<'a>(v: &'a Value, null: &'a str) -> Cow<'a, str> {
    match v {
        Value::Null => Cow::Borrowed(null),
        v => v.display(),
    }
}

/// Status line like `3 rows · 12.4 ms` / `Query OK, 1 row affected · 3 ms`.
/// Guesses whether the statement returned rows; prefer [`render_status_for`] when that is known.
pub fn render_status(summary: &Summary, row_count: usize, elapsed: Option<Duration>, opts: &OutputOptions) -> String {
    let returned_rows = row_count > 0 || summary.status.as_deref().is_some_and(is_row_tag);
    render_status_for(summary, row_count, returned_rows, elapsed, opts)
}

/// Status line for a statement known to have (`returned_rows`) or not have a result set.
pub fn render_status_for(
    summary: &Summary,
    row_count: usize,
    returned_rows: bool,
    elapsed: Option<Duration>,
    opts: &OutputOptions,
) -> String {
    let mut text = if returned_rows {
        plural(row_count as u64, "row")
    } else {
        match summary.status.as_deref() {
            Some(tag) if !tag.is_empty() && !tag.eq_ignore_ascii_case("query ok") => tag.to_string(),
            _ => format!("Query OK, {} affected", plural(summary.rows_affected.unwrap_or(0), "row")),
        }
    };
    if summary.warnings > 0 {
        text.push_str(&format!(", {}", plural(summary.warnings as u64, "warning")));
    }
    if let Some(d) = elapsed {
        text.push_str(" · ");
        text.push_str(&format_duration(d));
    }
    if opts.color == ColorDepth::None {
        text
    } else {
        format!("{}{text}{}", theme::ansi_fg(opts.theme.muted, opts.color), theme::RESET)
    }
}

fn is_row_tag(tag: &str) -> bool {
    let verb = tag.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    matches!(verb.as_str(), "SELECT" | "SHOW" | "FETCH" | "VALUES" | "TABLE" | "EXPLAIN")
}

fn plural(n: u64, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

pub fn format_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        format!("{ms:.2} ms")
    } else if ms < 1000.0 {
        format!("{ms:.1} ms")
    } else if ms < 60_000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else {
        let mins = d.as_secs() / 60;
        format!("{mins}m {:.1}s", d.as_secs_f64() - (mins * 60) as f64)
    }
}

/// Renders an introspection result: title, table (or verbatim text), then footer.
pub fn render_titled(t: &Titled, opts: &OutputOptions) -> String {
    let body = match &t.text {
        Some(text) if text.ends_with('\n') => text.clone(),
        Some(text) => format!("{text}\n"),
        None => render(&t.result.columns, &t.result.rows, opts),
    };
    if opts.format.is_machine() {
        return body;
    }
    let mut out = String::with_capacity(body.len() + 256);
    if let Some(title) = &t.title {
        let width = body.lines().map(|l| table::str_width(&strip_ansi(l))).max().unwrap_or(0);
        let tw = table::str_width(title);
        if t.text.is_none() && width > tw {
            out.push_str(&" ".repeat((width - tw) / 2));
        }
        match opts.color {
            ColorDepth::None => out.push_str(title),
            c => {
                out.push_str(theme::BOLD);
                out.push_str(&theme::ansi_fg(opts.theme.header, c));
                out.push_str(title);
                out.push_str(theme::RESET);
            }
        }
        out.push('\n');
    }
    out.push_str(&body);
    if let Some(footer) = &t.footer {
        out.push_str(footer);
        if !footer.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Removes ANSI CSI/OSC escape sequences (width math, plain-text sinks).
pub fn strip_ansi(s: &str) -> Cow<'_, str> {
    if !s.contains('\x1b') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    Cow::Owned(out)
}
