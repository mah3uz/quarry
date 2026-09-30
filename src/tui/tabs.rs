use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use super::widgets::editor::Editor;
use super::widgets::grid::GridState;
use super::widgets::input::Input;
use super::worker::ConnId;
use crate::db::{Backend, Column, PlanNode, Row, Summary, TableDetails, Value};
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Editor,
    Results,
}

#[derive(Clone, Debug)]
pub struct ResultMeta {
    pub index: usize,
    pub sql: String,
    pub columns: Vec<Column>,
    pub summary: Option<Summary>,
    pub elapsed: Option<Duration>,
    pub truncated: bool,
    /// Rows of results not currently shown in the grid.
    pub stash: Option<Vec<Row>>,
    pub row_count: usize,
}

#[derive(Clone, Debug)]
pub enum MessageKind {
    Info,
    Ok,
    Notice,
    Error,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub kind: MessageKind,
    pub text: String,
    pub at: chrono::DateTime<chrono::Local>,
}

pub struct Running {
    pub started: Instant,
    pub total: usize,
    pub current: usize,
}

/// What a query tab works in, when it isn't simply the connection's default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// A MySQL database, switched to with `USE` before each run.
    Database(String),
    /// A PostgreSQL schema, put first on the `search_path` before each run.
    Schema(String),
}

impl Scope {
    pub fn name(&self) -> &str {
        match self {
            Scope::Database(n) | Scope::Schema(n) => n,
        }
    }
}

pub struct QueryTab {
    pub editor: Editor,
    pub scope: Option<Scope>,
    pub grid: GridState,
    pub pane: Pane,
    /// Editor height as a percentage of the tab body.
    pub split: u16,
    pub results: Vec<ResultMeta>,
    /// Index into `results`, or `results.len()` for the Messages view.
    pub shown: usize,
    pub messages: Vec<Message>,
    pub messages_scroll: usize,
    pub running: Option<Running>,
    /// Which model and the question, while the model is writing SQL.
    pub asking: Option<(String, String)>,
    pub last_elapsed: Option<Duration>,
    /// Byte offset of the executed text inside the editor (for error markers).
    pub exec_base: usize,
    pub exec_sqls: Vec<String>,
    /// Editor byte offset where each executed statement starts.
    pub exec_starts: Vec<usize>,
    pub file: Option<std::path::PathBuf>,
    pub search: Option<Input>,
}

impl QueryTab {
    pub fn new(backend: Backend) -> Self {
        QueryTab {
            editor: Editor::new(backend),
            scope: None,
            grid: GridState::new(),
            pane: Pane::Editor,
            split: 42,
            results: Vec::new(),
            shown: 0,
            messages: Vec::new(),
            messages_scroll: 0,
            running: None,
            asking: None,
            last_elapsed: None,
            exec_base: 0,
            exec_sqls: Vec::new(),
            exec_starts: Vec::new(),
            file: None,
            search: None,
        }
    }

    pub fn log(&mut self, kind: MessageKind, text: impl Into<String>) {
        self.messages.push(Message { kind, text: text.into(), at: chrono::Local::now() });
        if self.messages.len() > 2000 {
            self.messages.drain(..500);
        }
    }

    pub fn reset_results(&mut self) {
        self.results.clear();
        self.shown = 0;
        self.grid.clear();
    }

    pub fn showing_messages(&self) -> bool {
        self.shown >= self.results.len()
    }

    /// Switches the grid to another result set, stashing the current rows.
    pub fn show(&mut self, idx: usize) {
        if idx == self.shown {
            return;
        }
        if self.shown < self.results.len() {
            let rows = self.grid.rows().to_vec();
            self.results[self.shown].stash = Some(rows);
        }
        self.shown = idx.min(self.results.len());
        if let Some(r) = self.results.get_mut(self.shown) {
            let rows = r.stash.take().unwrap_or_default();
            self.grid.set_data(r.columns.clone(), rows);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellKey {
    pub row: usize,
    pub col: usize,
}

pub struct TableTab {
    pub schema: String,
    pub name: String,
    pub details: Option<TableDetails>,
    pub grid: GridState,
    pub filter: String,
    pub filter_input: Option<Input>,
    pub order: Option<(usize, bool)>,
    pub page_size: usize,
    pub loaded: usize,
    pub has_more: bool,
    pub loading: bool,
    pub total: Option<u64>,
    pub edits: BTreeMap<CellKey, Value>,
    pub deleted: BTreeSet<usize>,
    /// Row indices (in the grid) of rows added locally and not yet inserted.
    pub inserted: BTreeSet<usize>,
    /// Row values before the first staged edit, so UPDATE ... WHERE matches the stored key.
    pub original: BTreeMap<usize, Row>,
    pub last_elapsed: Option<Duration>,
    pub error: Option<String>,
    /// Generation counter so stale page replies are ignored after a refresh.
    pub generation: u64,
}

impl TableTab {
    pub fn new(schema: &str, name: &str) -> Self {
        TableTab {
            schema: schema.to_string(),
            name: name.to_string(),
            details: None,
            grid: GridState::new(),
            filter: String::new(),
            filter_input: None,
            order: None,
            page_size: 500,
            loaded: 0,
            has_more: true,
            loading: false,
            total: None,
            edits: BTreeMap::new(),
            deleted: BTreeSet::new(),
            inserted: BTreeSet::new(),
            original: BTreeMap::new(),
            last_elapsed: None,
            error: None,
            generation: 0,
        }
    }

    pub fn dirty(&self) -> bool {
        !self.edits.is_empty() || !self.deleted.is_empty() || !self.inserted.is_empty()
    }

    pub fn pending_count(&self) -> usize {
        let edited_rows: BTreeSet<usize> =
            self.edits.keys().map(|k| k.row).filter(|r| !self.inserted.contains(r)).collect();
        edited_rows.len() + self.deleted.len() + self.inserted.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructSection {
    Columns,
    Indexes,
    ForeignKeys,
    References,
    Constraints,
    Triggers,
    Ddl,
}

impl StructSection {
    pub const ALL: [StructSection; 7] = [
        StructSection::Columns,
        StructSection::Indexes,
        StructSection::ForeignKeys,
        StructSection::References,
        StructSection::Constraints,
        StructSection::Triggers,
        StructSection::Ddl,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StructSection::Columns => "Columns",
            StructSection::Indexes => "Indexes",
            StructSection::ForeignKeys => "Foreign keys",
            StructSection::References => "Referenced by",
            StructSection::Constraints => "Constraints",
            StructSection::Triggers => "Triggers",
            StructSection::Ddl => "DDL",
        }
    }
}

pub struct StructureTab {
    pub schema: String,
    pub name: String,
    pub details: Option<TableDetails>,
    pub ddl: Option<String>,
    pub section: StructSection,
    pub grid: GridState,
    pub text_scroll: usize,
    pub error: Option<String>,
}

impl StructureTab {
    pub fn new(schema: &str, name: &str) -> Self {
        StructureTab {
            schema: schema.into(),
            name: name.into(),
            details: None,
            ddl: None,
            section: StructSection::Columns,
            grid: GridState::new(),
            text_scroll: 0,
            error: None,
        }
    }

    pub fn load_section(&mut self) {
        let Some(d) = &self.details else { return };
        let txt = |s: &str| Value::Text(s.to_string());
        let opt = |s: &Option<String>| Value::Text(s.clone().unwrap_or_default());
        let yes = |b: bool| Value::Text(if b { "✓".into() } else { String::new() });
        let (cols, rows): (Vec<&str>, Vec<Row>) = match self.section {
            StructSection::Columns => (
                vec!["name", "type", "nullable", "default", "key", "comment"],
                d.columns
                    .iter()
                    .map(|c| {
                        let key = match (c.primary_key, c.auto) {
                            (true, true) => "PK · auto",
                            (true, false) => "PK",
                            (false, true) => "auto",
                            _ => "",
                        };
                        vec![txt(&c.name), txt(&c.data_type), yes(c.nullable), opt(&c.default), txt(key), opt(&c.comment)]
                    })
                    .collect(),
            ),
            StructSection::Indexes => (
                vec!["name", "columns", "unique", "primary", "method", "definition"],
                d.indexes
                    .iter()
                    .map(|i| vec![txt(&i.name), txt(&i.columns.join(", ")), yes(i.unique), yes(i.primary), opt(&i.method), opt(&i.definition)])
                    .collect(),
            ),
            StructSection::ForeignKeys | StructSection::References => {
                let fks = if self.section == StructSection::ForeignKeys { &d.foreign_keys } else { &d.referenced_by };
                (
                    vec!["name", "table", "columns", "references", "ref columns", "on update", "on delete"],
                    fks.iter()
                        .map(|f| {
                            vec![
                                txt(&f.name),
                                txt(&format!("{}.{}", f.schema, f.table)),
                                txt(&f.columns.join(", ")),
                                txt(&format!("{}.{}", f.ref_schema, f.ref_table)),
                                txt(&f.ref_columns.join(", ")),
                                opt(&f.on_update),
                                opt(&f.on_delete),
                            ]
                        })
                        .collect(),
                )
            }
            StructSection::Constraints => (
                vec!["name", "kind", "definition"],
                d.constraints.iter().map(|c| vec![txt(&c.name), txt(&c.kind), txt(&c.definition)]).collect(),
            ),
            StructSection::Triggers => (
                vec!["name", "event", "definition"],
                d.triggers.iter().map(|t| vec![txt(&t.name), txt(&t.event), txt(&t.definition)]).collect(),
            ),
            StructSection::Ddl => (Vec::new(), Vec::new()),
        };
        let columns = cols.into_iter().map(|c| Column::new(c, "text")).collect();
        self.grid.set_empty_message(format!("No {}", self.section.label().to_lowercase()));
        self.grid.set_data(columns, rows);
    }

    pub fn section_count(&self, s: StructSection) -> Option<usize> {
        let d = self.details.as_ref()?;
        Some(match s {
            StructSection::Columns => d.columns.len(),
            StructSection::Indexes => d.indexes.len(),
            StructSection::ForeignKeys => d.foreign_keys.len(),
            StructSection::References => d.referenced_by.len(),
            StructSection::Constraints => d.constraints.len(),
            StructSection::Triggers => d.triggers.len(),
            StructSection::Ddl => return None,
        })
    }
}

pub struct ActivityTab {
    pub grid: GridState,
    pub paused: bool,
    pub last_refresh: Option<Instant>,
    pub interval: Duration,
    pub error: Option<String>,
    pub in_flight: bool,
}

pub struct TextTab {
    pub text: String,
    pub scroll: usize,
    pub sql: bool,
}

pub struct ExplainTab {
    pub sql: String,
    pub analyze: bool,
    pub plan: Option<PlanNode>,
    pub flat: Vec<(usize, PlanNode)>,
    pub selected: usize,
    pub offset: usize,
    pub elapsed: Option<Duration>,
    pub error: Option<String>,
}

impl ExplainTab {
    pub fn set_plan(&mut self, plan: PlanNode) {
        let mut flat = Vec::new();
        fn walk(n: &PlanNode, depth: usize, out: &mut Vec<(usize, PlanNode)>) {
            let mut shallow = n.clone();
            shallow.children.clear();
            out.push((depth, shallow));
            for c in &n.children {
                walk(c, depth + 1, out);
            }
        }
        walk(&plan, 0, &mut flat);
        self.flat = flat;
        self.plan = Some(plan);
        self.selected = 0;
    }
}

pub struct HistoryEntry {
    pub sql: String,
}

pub struct HistoryTab {
    pub entries: Vec<HistoryEntry>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub offset: usize,
    pub filter: Input,
}

impl HistoryTab {
    pub fn refilter(&mut self) {
        let q = self.filter.value().to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, e)| q.is_empty() || e.sql.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }
}

pub enum TabKind {
    Query(Box<QueryTab>),
    Table(Box<TableTab>),
    Structure(Box<StructureTab>),
    Activity(Box<ActivityTab>),
    Text(Box<TextTab>),
    Explain(Box<ExplainTab>),
    History(Box<HistoryTab>),
}

pub struct Tab {
    pub id: u64,
    pub conn: Option<ConnId>,
    pub title: String,
    pub kind: TabKind,
}

impl Tab {
    pub fn icon(&self) -> &'static str {
        let ic = crate::icons::get();
        match self.kind {
            TabKind::Query(_) => ic.query,
            TabKind::Table(_) => ic.table,
            TabKind::Structure(_) => ic.structure,
            TabKind::Activity(_) => ic.activity,
            TabKind::Text(_) => ic.text,
            TabKind::Explain(_) => ic.explain,
            TabKind::History(_) => ic.history,
        }
    }

    pub fn is_busy(&self) -> bool {
        match &self.kind {
            TabKind::Query(q) => q.running.is_some(),
            TabKind::Table(t) => t.loading,
            TabKind::Activity(a) => a.in_flight,
            _ => false,
        }
    }
}

/// Renders plain or SQL-highlighted text with a line-number gutter.
pub fn render_text(text: &str, sql: Option<Backend>, scroll: usize, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let lines: Vec<&str> = text.lines().collect();
    let gutter = lines.len().to_string().len() as u16 + 2;
    for (i, line) in lines.iter().enumerate().skip(scroll).take(area.height as usize) {
        let y = area.y + (i - scroll) as u16;
        buf.set_string(area.x, y, format!("{:>w$} ", i + 1, w = gutter as usize - 1), Style::default().fg(theme.muted));
        let x = area.x + gutter;
        let width = area.width.saturating_sub(gutter);
        let spans = match sql {
            Some(b) => highlight_spans(line, b, theme),
            None => vec![Span::styled(line.to_string(), Style::default().fg(theme.fg))],
        };
        Line::from(spans).render(Rect { x, y, width, height: 1 }, buf);
    }
}

pub fn highlight_spans(line: &str, backend: Backend, theme: &Theme) -> Vec<Span<'static>> {
    use crate::sql::lexer::{TokenKind, tokenize};
    let tokens = tokenize(line, backend);
    tokens
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let text = t.text(line).replace('\t', "    ");
            let next_paren = tokens[i + 1..].iter().find(|n| n.kind != TokenKind::Whitespace).is_some_and(|n| n.kind == TokenKind::LParen);
            let st = match t.kind {
                TokenKind::Keyword => Style::default().fg(theme.keyword).add_modifier(Modifier::BOLD),
                TokenKind::DataType => Style::default().fg(theme.datatype),
                TokenKind::Builtin => Style::default().fg(theme.function),
                TokenKind::Ident if next_paren => Style::default().fg(theme.function),
                TokenKind::Ident => Style::default().fg(theme.identifier),
                TokenKind::QuotedIdent => Style::default().fg(theme.quoted_ident),
                TokenKind::String => Style::default().fg(theme.string),
                TokenKind::Number => Style::default().fg(theme.number),
                TokenKind::LineComment | TokenKind::BlockComment => Style::default().fg(theme.comment).add_modifier(Modifier::ITALIC),
                TokenKind::Operator => Style::default().fg(theme.operator),
                TokenKind::Parameter | TokenKind::Variable => Style::default().fg(theme.parameter),
                TokenKind::Whitespace => Style::default(),
                _ => Style::default().fg(theme.punctuation),
            };
            Span::styled(text, st)
        })
        .collect()
}

/// Explain view: tree with cost/time bars relative to the root.
/// Returns the rows' area, for clicks.
pub fn render_explain(tab: &mut ExplainTab, area: Rect, buf: &mut Buffer, theme: &Theme, focused: bool) -> Rect {
    if let Some(e) = &tab.error {
        buf.set_stringn(area.x + 1, area.y, format!("{} {e}", crate::icons::get().error), area.width as usize - 1, Style::default().fg(theme.error));
        return Rect::default();
    }
    if tab.flat.is_empty() {
        buf.set_string(area.x + 1, area.y, "Planning…", Style::default().fg(theme.muted));
        return Rect::default();
    }
    let detail_h = (area.height / 3).clamp(3, 12);
    let list = Rect { height: area.height.saturating_sub(detail_h + 1), ..area };
    let metric = |n: &PlanNode| n.actual_time_ms.or(n.total_cost).unwrap_or(0.0);
    let root = tab.flat.first().map(|(_, n)| metric(n)).unwrap_or(0.0).max(f64::EPSILON);
    let h = list.height as usize;
    if tab.selected < tab.offset {
        tab.offset = tab.selected;
    } else if h > 0 && tab.selected >= tab.offset + h {
        tab.offset = tab.selected + 1 - h;
    }
    let bar_w: u16 = 16;
    for (i, (depth, node)) in tab.flat.iter().enumerate().skip(tab.offset).take(h) {
        let y = list.y + (i - tab.offset) as u16;
        let sel = i == tab.selected;
        if sel {
            buf.set_style(Rect { y, height: 1, ..list }, Style::default().bg(if focused { theme.selection } else { theme.highlight }));
        }
        let share = (metric(node) / root).clamp(0.0, 1.0);
        let filled = (share * bar_w as f64).round() as u16;
        let color = if share > 0.6 { theme.error } else if share > 0.25 { theme.warning } else { theme.success };
        for bx in 0..bar_w {
            let sym = if bx < filled { "█" } else { "░" };
            let st = if bx < filled { Style::default().fg(color) } else { Style::default().fg(theme.border) };
            buf.set_string(list.x + 1 + bx, y, sym, st);
        }
        let pct = format!("{:>4.0}%", share * 100.0);
        buf.set_string(list.x + bar_w + 2, y, &pct, Style::default().fg(theme.muted));
        let indent = "  ".repeat(*depth);
        let arrow = if *depth > 0 { "└ " } else { "" };
        let label = format!("{indent}{arrow}{}", node.label);
        let x = list.x + bar_w + 8;
        buf.set_stringn(x, y, &label, list.width.saturating_sub(bar_w + 8) as usize, Style::default().fg(theme.fg).add_modifier(if *depth == 0 { Modifier::BOLD } else { Modifier::empty() }));
        let mut facts = Vec::new();
        if let Some(r) = node.actual_rows.or(node.plan_rows) {
            facts.push(format!("{r:.0} rows"));
        }
        if let Some(t) = node.actual_time_ms {
            facts.push(format!("{t:.2} ms"));
        } else if let Some(c) = node.total_cost {
            facts.push(format!("cost {c:.1}"));
        }
        let f = facts.join(" · ");
        let used = x + label.width() as u16 + 2;
        let fx = list.x + list.width.saturating_sub(f.width() as u16 + 1);
        if fx > used {
            buf.set_string(fx, y, &f, Style::default().fg(theme.muted));
        }
    }
    let sep_y = list.y + list.height;
    for x in area.x..area.x + area.width {
        buf[(x, sep_y)].set_symbol("─").set_style(Style::default().fg(theme.border));
    }
    if let Some((_, node)) = tab.flat.get(tab.selected) {
        let mut y = sep_y + 1;
        let mut kv: Vec<(String, String)> = Vec::new();
        for (k, v) in [
            ("startup cost", node.startup_cost),
            ("total cost", node.total_cost),
            ("plan rows", node.plan_rows),
            ("actual rows", node.actual_rows),
            ("actual time ms", node.actual_time_ms),
            ("loops", node.loops),
        ] {
            if let Some(v) = v {
                kv.push((k.to_string(), format!("{v:.2}")));
            }
        }
        kv.extend(node.details.iter().cloned());
        for (k, v) in kv {
            if y >= area.y + area.height {
                break;
            }
            buf.set_string(area.x + 1, y, format!("{k}:"), Style::default().fg(theme.muted));
            buf.set_stringn(area.x + 18, y, &v, area.width.saturating_sub(19) as usize, Style::default().fg(theme.fg));
            y += 1;
        }
    }
    list
}
