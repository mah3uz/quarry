use std::borrow::Cow;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{Expanded, OutputOptions, TableFormat, cell_text};
use crate::db::{Column, Row, TypeKind, Value};
use crate::theme::{self, ColorDepth};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    Null,
    Number,
    Bool,
    Temporal,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
    Center,
}

struct Cell<'a> {
    text: Cow<'a, str>,
    width: usize,
    multiline: bool,
    style: Style,
}

#[derive(Clone, Copy)]
struct Rule {
    left: &'static str,
    fill: &'static str,
    cross: &'static str,
    right: &'static str,
}

struct Frame {
    top: Option<Rule>,
    head: Option<Rule>,
    bottom: Option<Rule>,
    left: &'static str,
    sep: &'static str,
    /// Drawn between rows when row_lines is on (the boxed formats only).
    row: Option<Rule>,
    right: &'static str,
    /// Rule fill per column = column width + `pad`.
    pad: usize,
    center_header: bool,
    markdown: bool,
}

const fn rule(left: &'static str, fill: &'static str, cross: &'static str, right: &'static str) -> Option<Rule> {
    Some(Rule { left, fill, cross, right })
}

fn frame(format: TableFormat) -> Frame {
    let boxed = |top, head: Option<Rule>, bottom, left, sep, right| Frame {
        top,
        row: head,
        head,
        bottom,
        left,
        sep,
        right,
        pad: 2,
        center_header: false,
        markdown: false,
    };
    match format {
        TableFormat::Rounded => boxed(
            rule("╭", "─", "┬", "╮"),
            rule("├", "─", "┼", "┤"),
            rule("╰", "─", "┴", "╯"),
            "│ ",
            " │ ",
            " │",
        ),
        TableFormat::Unicode => boxed(
            rule("┌", "─", "┬", "┐"),
            rule("├", "─", "┼", "┤"),
            rule("└", "─", "┴", "┘"),
            "│ ",
            " │ ",
            " │",
        ),
        TableFormat::Double => boxed(
            rule("╔", "═", "╦", "╗"),
            rule("╠", "═", "╬", "╣"),
            rule("╚", "═", "╩", "╝"),
            "║ ",
            " ║ ",
            " ║",
        ),
        TableFormat::Ascii => boxed(
            rule("+", "-", "+", "+"),
            rule("+", "-", "+", "+"),
            rule("+", "-", "+", "+"),
            "| ",
            " | ",
            " |",
        ),
        TableFormat::Psql => Frame {
            top: None,
            head: rule("", "-", "+", ""),
            bottom: None,
            left: " ",
            sep: " | ",
            right: " ",
            row: None,
            pad: 2,
            center_header: true,
            markdown: false,
        },
        TableFormat::Markdown => Frame {
            top: None,
            head: rule("|", "-", "|", "|"),
            bottom: None,
            left: "| ",
            sep: " | ",
            right: " |",
            row: None,
            pad: 2,
            center_header: false,
            markdown: true,
        },
        TableFormat::Minimal | TableFormat::Simple | TableFormat::Plain => Frame {
            top: None,
            head: match format {
                TableFormat::Minimal => rule("", "─", "  ", ""),
                TableFormat::Simple => rule("", "-", "  ", ""),
                _ => None,
            },
            bottom: None,
            left: "",
            sep: "  ",
            right: "",
            row: None,
            pad: 0,
            center_header: false,
            markdown: false,
        },
        _ => frame(TableFormat::Rounded),
    }
}

struct Paint {
    border: String,
    header: String,
    null: String,
    number: String,
    boolean: String,
    temporal: String,
    json: String,
}

impl Paint {
    fn new(opts: &OutputOptions) -> Paint {
        let d = opts.color;
        if d == ColorDepth::None {
            return Paint {
                border: String::new(),
                header: String::new(),
                null: String::new(),
                number: String::new(),
                boolean: String::new(),
                temporal: String::new(),
                json: String::new(),
            };
        }
        let t = &opts.theme;
        Paint {
            border: theme::ansi_fg(t.border, d),
            header: format!("{}{}", theme::BOLD, theme::ansi_fg(t.header, d)),
            null: format!("{}{}", theme::ITALIC, theme::ansi_fg(t.null, d)),
            number: theme::ansi_fg(t.number, d),
            boolean: theme::ansi_fg(t.boolean, d),
            temporal: theme::ansi_fg(t.temporal, d),
            json: theme::ansi_fg(t.json, d),
        }
    }

    fn style(&self, s: Style) -> &str {
        match s {
            Style::Plain => "",
            Style::Null => &self.null,
            Style::Number => &self.number,
            Style::Bool => &self.boolean,
            Style::Temporal => &self.temporal,
            Style::Json => &self.json,
        }
    }

    /// Colors only the visible glyphs so trailing-space trimming still works on colored lines.
    fn border(&self, s: &str) -> String {
        let core = s.trim_matches(' ');
        if self.border.is_empty() || core.is_empty() {
            return s.to_string();
        }
        let lead = &s[..s.len() - s.trim_start_matches(' ').len()];
        let trail = &s[s.trim_end_matches(' ').len()..];
        format!("{lead}{}{core}{}{trail}", self.border, theme::RESET)
    }
}

pub(crate) fn str_width(s: &str) -> usize {
    if s.is_ascii() { s.len() } else { UnicodeWidthStr::width(s) }
}

fn text_width(s: &str) -> usize {
    if s.contains('\n') { s.split('\n').map(str_width).max().unwrap_or(0) } else { str_width(s) }
}

fn is_c1(c: char) -> bool {
    ('\u{80}'..='\u{9f}').contains(&c)
}

fn needs_cleaning(s: &str, markdown: bool) -> bool {
    s.bytes().any(|b| (b < 0x20 && b != b'\n') || b == 0x7f || (markdown && (b == b'|' || b == b'\n')))
        || (!s.is_ascii() && s.chars().any(is_c1))
}

/// Makes control characters visible so they cannot corrupt the terminal or the alignment.
fn clean(s: &str, markdown: bool) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\t' => out.push_str("    "),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' if markdown => out.push_str("<br>"),
            '\n' => out.push('\n'),
            '|' if markdown => out.push_str("\\|"),
            '\x7f' => out.push('\u{2421}'),
            c if (c as u32) < 0x20 => out.push(char::from_u32(0x2400 + c as u32).unwrap_or('?')),
            c if is_c1(c) => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

fn truncate_line(line: &str, max: usize, out: &mut String) {
    if str_width(line) <= max {
        out.push_str(line);
        return;
    }
    let mut w = 0;
    for c in line.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.push('…');
}

fn truncate(s: &str, max: usize) -> Option<String> {
    if !s.split('\n').any(|l| l.len() > max && str_width(l) > max) {
        return None;
    }
    let mut out = String::with_capacity(s.len().min(max * 4 + 8));
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        truncate_line(line, max, &mut out);
    }
    Some(out)
}

fn prepare<'a>(text: Cow<'a, str>, max: Option<usize>, markdown: bool) -> Cow<'a, str> {
    let text = if needs_cleaning(&text, markdown) { Cow::Owned(clean(&text, markdown)) } else { text };
    match max.filter(|m| *m > 0).and_then(|m| truncate(&text, m)) {
        Some(t) => Cow::Owned(t),
        None => text,
    }
}

fn make_cell<'a>(text: Cow<'a, str>, style: Style, max: Option<usize>, markdown: bool) -> Cell<'a> {
    let text = prepare(text, max, markdown);
    let multiline = text.contains('\n');
    let width = if multiline { text_width(&text) } else { str_width(&text) };
    Cell { text, width, multiline, style }
}

fn looks_numeric(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s.parse::<f64>().is_ok()
}

/// Declared numeric type, or (untyped columns) every non-null value is a number.
fn is_numeric_column(col: &Column, rows: &[Row], idx: usize) -> bool {
    if col.kind.is_numeric() {
        return true;
    }
    if col.kind != TypeKind::Other {
        return false;
    }
    let mut seen = false;
    for row in rows {
        match row.get(idx) {
            None | Some(Value::Null) => {}
            Some(Value::Int(_) | Value::UInt(_) | Value::Float(_)) => seen = true,
            Some(Value::Text(t)) if col.type_name.is_empty() && looks_numeric(t) => seen = true,
            Some(_) => return false,
        }
    }
    seen
}

fn style_of(v: &Value, kind: TypeKind, numeric: bool) -> Style {
    match v {
        Value::Null => Style::Null,
        Value::Bool(_) => Style::Bool,
        _ if numeric => Style::Number,
        _ if kind == TypeKind::Bool => Style::Bool,
        _ if kind.is_temporal() => Style::Temporal,
        _ if kind == TypeKind::Json => Style::Json,
        _ => Style::Plain,
    }
}

struct Grid<'a> {
    header: Vec<Cell<'a>>,
    cells: Vec<Cell<'a>>,
    widths: Vec<usize>,
    numeric: Vec<bool>,
}

impl<'a> Grid<'a> {
    fn build(columns: &'a [Column], rows: &'a [Row], opts: &'a OutputOptions, markdown: bool) -> Grid<'a> {
        let n = columns.len();
        let max = opts.max_field_width;
        let numeric: Vec<bool> = columns.iter().enumerate().map(|(i, c)| is_numeric_column(c, rows, i)).collect();
        let header: Vec<Cell> = columns
            .iter()
            .map(|c| make_cell(Cow::Borrowed(c.name.as_str()), Style::Plain, max, markdown))
            .collect();
        let mut widths: Vec<usize> = header.iter().map(|c| c.width).collect();
        let mut cells = Vec::with_capacity(rows.len() * n);
        static NULL: Value = Value::Null;
        let null = &NULL;
        for row in rows {
            for (i, col) in columns.iter().enumerate() {
                let v = row.get(i).unwrap_or(null);
                let cell = make_cell(cell_text(v, &opts.null_string), style_of(v, col.kind, numeric[i]), max, markdown);
                if cell.width > widths[i] {
                    widths[i] = cell.width;
                }
                cells.push(cell);
            }
        }
        Grid { header, cells, widths, numeric }
    }

    fn rows(&self) -> impl Iterator<Item = &[Cell<'a>]> {
        self.cells.chunks(self.header.len().max(1))
    }
}

pub(super) fn render(columns: &[Column], rows: &[Row], opts: &OutputOptions) -> String {
    let fr = frame(opts.format);
    let vertical_only = opts.format == TableFormat::Vertical || opts.expanded == Expanded::On;
    let grid = Grid::build(columns, rows, opts, fr.markdown && !vertical_only);
    let paint = Paint::new(opts);
    if vertical_only {
        return render_vertical(&grid, &paint);
    }
    let n = grid.widths.len();
    let total = str_width(fr.left) + str_width(fr.right) + str_width(fr.sep) * n.saturating_sub(1)
        + grid.widths.iter().sum::<usize>();
    if opts.expanded == Expanded::Auto && opts.terminal_width > 0 && total > opts.terminal_width && !rows.is_empty() {
        return render_vertical(&grid, &paint);
    }
    render_table(&grid, &fr, &paint, opts, total)
}

fn render_table(grid: &Grid, fr: &Frame, paint: &Paint, opts: &OutputOptions, total: usize) -> String {
    let n = grid.widths.len();
    let per_line = total * 3 + if paint.border.is_empty() { 0 } else { n * 24 + 32 };
    let mut out = String::with_capacity(per_line * (grid.cells.len() / n.max(1) + 4));
    let aligns: Vec<Align> = grid
        .numeric
        .iter()
        .map(|&num| if num && opts.align_numbers { Align::Right } else { Align::Left })
        .collect();
    let header_aligns: Vec<Align> =
        if fr.center_header { vec![Align::Center; n] } else { aligns.clone() };
    let left = paint.border(fr.left);
    let sep = paint.border(fr.sep);
    let right = paint.border(fr.right);
    let trim = fr.right.trim().is_empty();

    if let Some(r) = &fr.top {
        push_rule(&mut out, r, &grid.widths, fr.pad, paint);
    }
    let row = RowWriter { widths: &grid.widths, left: &left, sep: &sep, right: &right, trim, paint };
    row.write(&mut out, &grid.header, &header_aligns, true);
    if fr.markdown {
        push_markdown_rule(&mut out, &grid.widths, &aligns, paint);
    } else if let Some(r) = &fr.head {
        push_rule(&mut out, r, &grid.widths, fr.pad, paint);
    }
    let between = fr.row.as_ref().filter(|_| opts.row_lines);
    for (i, cells) in grid.rows().filter(|c| !c.is_empty()).enumerate() {
        if let (Some(r), true) = (between, i > 0) {
            push_rule(&mut out, r, &grid.widths, fr.pad, paint);
        }
        row.write(&mut out, cells, &aligns, false);
    }
    if let Some(r) = &fr.bottom {
        push_rule(&mut out, r, &grid.widths, fr.pad, paint);
    }
    out
}

fn push_rule(out: &mut String, r: &Rule, widths: &[usize], pad: usize, paint: &Paint) {
    out.push_str(&paint.border);
    out.push_str(r.left);
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            out.push_str(r.cross);
        }
        for _ in 0..w + pad {
            out.push_str(r.fill);
        }
    }
    out.push_str(r.right);
    if !paint.border.is_empty() {
        out.push_str(theme::RESET);
    }
    out.push('\n');
}

fn push_markdown_rule(out: &mut String, widths: &[usize], aligns: &[Align], paint: &Paint) {
    out.push_str(&paint.border);
    out.push('|');
    for (w, a) in widths.iter().zip(aligns) {
        let dashes = (w + 2).max(3);
        if *a == Align::Right {
            out.push_str(&"-".repeat(dashes - 1));
            out.push(':');
        } else {
            out.push_str(&"-".repeat(dashes));
        }
        out.push('|');
    }
    if !paint.border.is_empty() {
        out.push_str(theme::RESET);
    }
    out.push('\n');
}

struct RowWriter<'a> {
    widths: &'a [usize],
    left: &'a str,
    sep: &'a str,
    right: &'a str,
    trim: bool,
    paint: &'a Paint,
}

impl RowWriter<'_> {
    fn write(&self, out: &mut String, cells: &[Cell], aligns: &[Align], header: bool) {
        let height = if cells.iter().any(|c| c.multiline) {
            cells.iter().map(|c| c.text.split('\n').count()).max().unwrap_or(1)
        } else {
            1
        };
        if height == 1 {
            out.push_str(self.left);
            for (i, c) in cells.iter().enumerate() {
                if i > 0 {
                    out.push_str(self.sep);
                }
                self.push_cell(out, &c.text, c.width, i, aligns[i], self.style(c, header));
            }
            self.finish(out);
            return;
        }
        let lines: Vec<Vec<&str>> = cells.iter().map(|c| c.text.split('\n').collect()).collect();
        for k in 0..height {
            out.push_str(self.left);
            for (i, c) in cells.iter().enumerate() {
                if i > 0 {
                    out.push_str(self.sep);
                }
                let line = lines[i].get(k).copied().unwrap_or("");
                self.push_cell(out, line, str_width(line), i, aligns[i], self.style(c, header));
            }
            self.finish(out);
        }
    }

    fn style(&self, c: &Cell, header: bool) -> &str {
        if header { &self.paint.header } else { self.paint.style(c.style) }
    }

    fn finish(&self, out: &mut String) {
        out.push_str(self.right);
        if self.trim {
            let len = out.trim_end_matches(' ').len();
            out.truncate(len);
        }
        out.push('\n');
    }

    fn push_cell(&self, out: &mut String, text: &str, width: usize, col: usize, align: Align, style: &str) {
        let pad = self.widths[col].saturating_sub(width);
        let (before, after) = match align {
            Align::Left => (0, pad),
            Align::Right => (pad, 0),
            Align::Center => (pad / 2, pad - pad / 2),
        };
        push_spaces(out, before);
        if style.is_empty() || text.is_empty() {
            out.push_str(text);
        } else {
            out.push_str(style);
            out.push_str(text);
            out.push_str(theme::RESET);
        }
        push_spaces(out, after);
    }
}

fn push_spaces(out: &mut String, n: usize) {
    const SPACES: &str = "                                                                ";
    let mut n = n;
    while n > 0 {
        let k = n.min(SPACES.len());
        out.push_str(&SPACES[..k]);
        n -= k;
    }
}

fn render_vertical(grid: &Grid, paint: &Paint) -> String {
    let name_w = grid.header.iter().map(|c| c.width).max().unwrap_or(0);
    let bytes: usize = grid.cells.iter().map(|c| c.text.len() + name_w + 4).sum();
    let mut out = String::with_capacity(bytes + grid.cells.len() / grid.header.len().max(1) * 80);
    let stars = "***************************";
    for (r, cells) in grid.rows().filter(|c| !c.is_empty()).enumerate() {
        out.push_str(&paint.border(&format!("{stars} {}. row {stars}", r + 1)));
        out.push('\n');
        for (h, c) in grid.header.iter().zip(cells) {
            push_spaces(&mut out, name_w - h.width);
            if paint.header.is_empty() {
                out.push_str(&h.text);
            } else {
                out.push_str(&paint.header);
                out.push_str(&h.text);
                out.push_str(theme::RESET);
            }
            out.push_str(": ");
            let style = paint.style(c.style);
            for (k, line) in c.text.split('\n').enumerate() {
                if k > 0 {
                    out.push('\n');
                    push_spaces(&mut out, name_w + 2);
                }
                if style.is_empty() || line.is_empty() {
                    out.push_str(line);
                } else {
                    out.push_str(style);
                    out.push_str(line);
                    out.push_str(theme::RESET);
                }
            }
            out.push('\n');
        }
    }
    out
}
