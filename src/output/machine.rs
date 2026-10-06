use super::{OutputOptions, TableFormat};
use crate::db::{Backend, Column, Row, TypeKind, Value, hex_bytes, quote_ident};

pub(super) fn render(columns: &[Column], rows: &[Row], opts: &OutputOptions) -> String {
    match opts.format {
        TableFormat::Csv => csv(columns, rows),
        TableFormat::Tsv => tsv(columns, rows, &opts.null_string),
        TableFormat::Json => json(columns, rows, false),
        TableFormat::JsonLines => json(columns, rows, true),
        TableFormat::Html => html(columns, rows, &opts.null_string),
        TableFormat::SqlInsert => sql_insert(columns, rows, opts),
        TableFormat::SqlUpdate => sql_update(columns, rows, opts),
        _ => String::new(),
    }
}

const HTML_TAIL: &str = "  </tbody>\n</table>\n";

/// What a format writes after its last row.
pub(super) fn tail(format: TableFormat) -> &'static str {
    match format {
        TableFormat::Json => "\n]\n",
        TableFormat::Html => HTML_TAIL,
        _ => "",
    }
}

fn get(row: &Row, i: usize) -> &Value {
    row.get(i).unwrap_or(&Value::Null)
}

fn csv(columns: &[Column], rows: &[Row]) -> String {
    let mut w = csv::WriterBuilder::new().terminator(csv::Terminator::Any(b'\n')).from_writer(Vec::new());
    let _ = w.write_record(columns.iter().map(|c| c.name.as_bytes()));
    let mut record: Vec<std::borrow::Cow<str>> = Vec::with_capacity(columns.len());
    for row in rows {
        record.clear();
        record.extend((0..columns.len()).map(|i| get(row, i).display()));
        let _ = w.write_record(record.iter().map(|s| s.as_bytes()));
    }
    let bytes = w.into_inner().unwrap_or_default();
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn tsv_escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
}

fn tsv(columns: &[Column], rows: &[Row], null: &str) -> String {
    let mut out = String::with_capacity((rows.len() + 1) * columns.len() * 12);
    for (i, c) in columns.iter().enumerate() {
        if i > 0 {
            out.push('\t');
        }
        tsv_escape(&c.name, &mut out);
    }
    out.push('\n');
    for row in rows {
        for i in 0..columns.len() {
            if i > 0 {
                out.push('\t');
            }
            match get(row, i) {
                Value::Null => out.push_str(null),
                v => tsv_escape(&v.display(), &mut out),
            }
        }
        out.push('\n');
    }
    out
}

/// JSON number grammar, so server-printed numerics can be emitted verbatim without precision loss.
pub(super) fn is_json_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - start
    };
    let int_start = i;
    let n = digits(&mut i);
    if n == 0 || (n > 1 && b[int_start] == b'0') {
        return false;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if digits(&mut i) == 0 {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == b.len()
}

fn json_string(s: &str, out: &mut String) {
    out.push_str(&serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()));
}

fn json_value(v: &Value, kind: TypeKind, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::UInt(u) => out.push_str(&u.to_string()),
        Value::Float(f) if f.is_finite() => out.push_str(&serde_json::to_string(f).unwrap_or_else(|_| f.to_string())),
        Value::Float(f) => json_string(&f.to_string(), out),
        Value::Bytes(b) => json_string(&hex_bytes(b), out),
        Value::Text(s) => match kind {
            TypeKind::Json => match serde_json::from_str::<serde_json::Value>(s) {
                Ok(j) => out.push_str(&j.to_string()),
                Err(_) => json_string(s, out),
            },
            k if k.is_numeric() && is_json_number(s.trim()) => out.push_str(s.trim()),
            TypeKind::Bool => match s.as_str() {
                "t" | "true" | "TRUE" | "1" => out.push_str("true"),
                "f" | "false" | "FALSE" | "0" => out.push_str("false"),
                _ => json_string(s, out),
            },
            _ => json_string(s, out),
        },
    }
}

fn json(columns: &[Column], rows: &[Row], lines: bool) -> String {
    let keys: Vec<String> = columns.iter().map(|c| serde_json::to_string(&c.name).unwrap_or_default()).collect();
    let (colon, comma) = if lines { (":", ",") } else { (": ", ", ") };
    let mut out = String::with_capacity((rows.len() + 1) * columns.len() * 24);
    if !lines {
        if rows.is_empty() {
            return "[]\n".into();
        }
        out.push_str("[\n");
    }
    for (r, row) in rows.iter().enumerate() {
        if !lines {
            out.push_str("  ");
        }
        out.push('{');
        for (i, col) in columns.iter().enumerate() {
            if i > 0 {
                out.push_str(comma);
            }
            out.push_str(&keys[i]);
            out.push_str(colon);
            json_value(get(row, i), col.kind, &mut out);
        }
        out.push('}');
        if !lines && r + 1 < rows.len() {
            out.push(',');
        }
        out.push('\n');
    }
    if !lines {
        out.push_str("]\n");
    }
    out
}

fn html_escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

fn html(columns: &[Column], rows: &[Row], null: &str) -> String {
    let mut out = String::with_capacity((rows.len() + 2) * columns.len() * 20 + 64);
    out.push_str("<table>\n  <thead>\n    <tr>");
    for c in columns {
        out.push_str("<th>");
        html_escape(&c.name, &mut out);
        out.push_str("</th>");
    }
    out.push_str("</tr>\n  </thead>\n  <tbody>\n");
    for row in rows {
        out.push_str("    <tr>");
        for i in 0..columns.len() {
            match get(row, i) {
                Value::Null => {
                    out.push_str("<td class=\"null\">");
                    html_escape(null, &mut out);
                }
                v => {
                    out.push_str("<td>");
                    html_escape(&v.display(), &mut out);
                }
            }
            out.push_str("</td>");
        }
        out.push_str("</tr>\n");
    }
    out.push_str(HTML_TAIL);
    out
}

fn target_table(opts: &OutputOptions) -> String {
    match opts.table_name.as_deref() {
        Some(t) if t.contains(['"', '`', '[']) => t.to_string(),
        Some(t) => t.split('.').map(|p| quote_ident(p, opts.backend)).collect::<Vec<_>>().join("."),
        None => quote_ident("table", opts.backend),
    }
}

fn sql_literal(v: &Value, kind: TypeKind, backend: Backend) -> String {
    match v {
        Value::Text(s) if kind.is_numeric() && is_json_number(s.trim()) => s.trim().to_string(),
        Value::Text(s) if kind == TypeKind::Bool && backend == Backend::Postgres && matches!(s.as_str(), "t" | "f") => {
            if s == "t" { "TRUE".into() } else { "FALSE".into() }
        }
        v => v.to_sql_literal(backend),
    }
}

fn sql_insert(columns: &[Column], rows: &[Row], opts: &OutputOptions) -> String {
    let table = target_table(opts);
    let cols = columns.iter().map(|c| quote_ident(&c.name, opts.backend)).collect::<Vec<_>>().join(", ");
    let prefix = format!("INSERT INTO {table} ({cols}) VALUES (");
    let mut out = String::with_capacity(rows.len() * (prefix.len() + columns.len() * 12));
    for row in rows {
        out.push_str(&prefix);
        for (i, c) in columns.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&sql_literal(get(row, i), c.kind, opts.backend));
        }
        out.push_str(");\n");
    }
    out
}

fn sql_update(columns: &[Column], rows: &[Row], opts: &OutputOptions) -> String {
    let table = target_table(opts);
    let names: Vec<String> = columns.iter().map(|c| quote_ident(&c.name, opts.backend)).collect();
    let mut out = String::new();
    for row in rows {
        out.push_str("UPDATE ");
        out.push_str(&table);
        out.push_str(" SET ");
        let set_cols = if columns.len() > 1 { 1..columns.len() } else { 0..columns.len() };
        for (n, i) in set_cols.enumerate() {
            if n > 0 {
                out.push_str(", ");
            }
            out.push_str(&names[i]);
            out.push_str(" = ");
            out.push_str(&sql_literal(get(row, i), columns[i].kind, opts.backend));
        }
        out.push_str(" WHERE ");
        out.push_str(&names[0]);
        match get(row, 0) {
            Value::Null => out.push_str(" IS NULL"),
            v => {
                out.push_str(" = ");
                out.push_str(&sql_literal(v, columns[0].kind, opts.backend));
            }
        }
        out.push_str(";\n");
    }
    out
}
