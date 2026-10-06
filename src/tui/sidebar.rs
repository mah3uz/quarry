use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget};
use unicode_width::UnicodeWidthStr;

use super::widgets::input::{Input, InputEvent};
use super::tabs::Scope;
use super::worker::ConnId;
use crate::db::{Backend, Catalog, FunctionKind, RelKind, Relation};
use crate::icons;
use crate::theme::Theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Connection { backend: Backend, mariadb: bool },
    DatabasesGroup,
    Database { name: String, current: bool },
    /// `database` is `Some(current)` where a schema is a whole database (MySQL, SQLite attachments).
    Schema { name: String, database: Option<bool> },
    Group { schema: String, what: GroupKind },
    Relation { schema: String, name: String, kind: RelKind },
    Column { data_type: String, pk: bool, nullable: bool },
    Function { schema: String, name: String, signature: String },
    Message,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroupKind {
    Tables,
    Views,
    MaterializedViews,
    Functions,
    Procedures,
}

impl GroupKind {
    fn label(self) -> &'static str {
        match self {
            GroupKind::Tables => "Tables",
            GroupKind::Views => "Views",
            GroupKind::MaterializedViews => "Materialized views",
            GroupKind::Functions => "Functions",
            GroupKind::Procedures => "Procedures",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub conn: ConnId,
    pub kind: NodeKind,
    pub label: String,
    pub detail: String,
    pub expanded: bool,
    pub children: Vec<Node>,
    /// Children are fetched on first expansion (MySQL non-current databases).
    pub lazy: bool,
    /// Connection roots only: the saved connection's tag color.
    pub color: Option<Color>,
}

impl Node {
    fn new(conn: ConnId, kind: NodeKind, label: impl Into<String>) -> Node {
        Node { conn, kind, label: label.into(), detail: String::new(), expanded: false, children: Vec::new(), lazy: false, color: None }
    }

    fn key(&self, parent: &str) -> String {
        format!("{parent}/{}", self.label)
    }

    fn expandable(&self) -> bool {
        self.lazy || !self.children.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    OpenTable { conn: ConnId, schema: String, name: String },
    OpenStructure { conn: ConnId, schema: String, name: String },
    Generate { conn: ConnId, schema: String, name: String, what: Script },
    InsertText(String),
    SwitchDatabase { conn: ConnId, name: String },
    LoadRelations { conn: ConnId, schema: String },
    Refresh(ConnId),
    ShowFunction { conn: ConnId, schema: String, name: String },
    NewConnection,
    Disconnect(ConnId),
    /// A query tab for the database or schema around the selection (`None`: the connection's own).
    NewConsole { conn: ConnId, scope: Option<Scope>, database: Option<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Select,
    Insert,
    Update,
    Delete,
    Create,
    Drop,
    Count,
}

struct Row {
    depth: usize,
    path: Vec<usize>,
    /// For each ancestor level: whether that ancestor was the last child (draws `│` guides).
    guides: Vec<bool>,
    last: bool,
}

#[derive(Default)]
pub struct Sidebar {
    pub roots: Vec<Node>,
    selected: usize,
    offset: usize,
    rows: Vec<Row>,
    expanded_keys: HashSet<String>,
    pub filter: Option<Input>,
    filter_text: String,
    area: Rect,
    /// Pending `g` prefix for script generation.
    pending_g: bool,
}


impl Sidebar {
    pub fn set_connection(&mut self, conn: ConnId, label: &str, detail: &str, backend: Backend, mariadb: bool) {
        match self.roots.iter_mut().find(|r| r.conn == conn) {
            Some(r) => {
                r.label = label.to_string();
                r.detail = detail.to_string();
            }
            None => {
                let mut n = Node::new(conn, NodeKind::Connection { backend, mariadb }, label);
                n.detail = detail.to_string();
                n.expanded = true;
                n.children.push(Node::new(conn, NodeKind::Message, "loading…"));
                self.expanded_keys.insert(n.key(""));
                self.roots.push(n);
            }
        }
        self.rebuild_rows();
    }

    pub fn set_connection_color(&mut self, conn: ConnId, color: Option<Color>) {
        if let Some(r) = self.roots.iter_mut().find(|r| r.conn == conn) {
            r.color = color;
        }
    }

    pub fn remove_connection(&mut self, conn: ConnId) {
        self.roots.retain(|r| r.conn != conn);
        self.selected = self.selected.min(self.visible_len().saturating_sub(1));
        self.rebuild_rows();
    }

    /// Replaces the connection's subtree from a fresh catalog, keeping expansion state.
    pub fn set_catalog(&mut self, conn: ConnId, cat: &Catalog, show_system: bool) {
        let Some(root_idx) = self.roots.iter().position(|r| r.conn == conn) else {
            return;
        };
        let mut children = Vec::new();
        if cat.backend == Backend::Postgres && !cat.databases.is_empty() {
            let mut g = Node::new(conn, NodeKind::DatabasesGroup, "Databases");
            g.detail = cat.databases.len().to_string();
            for d in &cat.databases {
                let current = cat.current_database.as_deref() == Some(d.as_str());
                let mut n = Node::new(conn, NodeKind::Database { name: d.clone(), current }, d.clone());
                if current {
                    n.detail = "current".into();
                }
                g.children.push(n);
            }
            children.push(g);
        }
        let mut schemas: Vec<&crate::db::SchemaInfo> = cat
            .schemas
            .iter()
            .filter(|s| show_system || !is_system_schema(&s.name, cat.backend))
            .collect();
        schemas.sort_by_key(|s| (!cat.is_on_search_path(&s.name), s.name.clone()));
        for s in schemas {
            let database = (cat.backend != Backend::Postgres).then(|| cat.is_on_search_path(&s.name));
            let mut sn = Node::new(conn, NodeKind::Schema { name: s.name.clone(), database }, s.name.clone());
            let lazy = cat.backend == Backend::MySql && cat.current_database.as_deref() != Some(s.name.as_str())
                && s.relations.is_empty();
            if lazy {
                sn.lazy = true;
            } else {
                // SQLite only reports built-in functions; listing them would bury the tables.
                let funcs: &[crate::db::FunctionInfo] = if cat.backend == Backend::Sqlite { &[] } else { &s.functions };
                sn.children = schema_children(conn, &s.name, &s.relations, funcs);
                sn.detail = s.relations.len().to_string();
            }
            if cat.backend == Backend::MySql && cat.current_database.as_deref() == Some(s.name.as_str()) {
                sn.detail = format!("{} · current", s.relations.len());
            }
            children.push(sn);
        }
        if children.is_empty() {
            children.push(Node::new(conn, NodeKind::Message, "empty"));
        }
        let root = &mut self.roots[root_idx];
        root.children = children;
        let key = root.key("");
        restore_expansion(root, "", &self.expanded_keys);
        if self.expanded_keys.contains(&key) {
            root.expanded = true;
        }
        // auto-expand the obvious default schema the first time
        if root.children.iter().all(|c| !c.expanded) {
            let default = cat.search_path.first().cloned().or(cat.current_database.clone());
            if let Some(d) = default
                && let Some(c) = root.children.iter_mut().find(|c| matches!(&c.kind, NodeKind::Schema { name, .. } if *name == d)) {
                    c.expanded = true;
                    let k = format!("{key}/{}", c.label);
                    self.expanded_keys.insert(k.clone());
                    if let Some(t) = c.children.iter_mut().find(|g| matches!(g.kind, NodeKind::Group { what: GroupKind::Tables, .. })) {
                        t.expanded = true;
                        self.expanded_keys.insert(format!("{k}/{}", t.label));
                    }
                }
        }
        self.rebuild_rows();
    }

    pub fn set_relations(&mut self, conn: ConnId, schema: &str, rels: &[Relation]) {
        if let Some(root) = self.roots.iter_mut().find(|r| r.conn == conn)
            && let Some(sn) = root.children.iter_mut().find(|c| matches!(&c.kind, NodeKind::Schema { name, .. } if name == schema)) {
                sn.lazy = false;
                sn.children = schema_children(conn, schema, rels, &[]);
                sn.detail = rels.len().to_string();
                if sn.children.is_empty() {
                    sn.children.push(Node::new(conn, NodeKind::Message, "empty"));
                }
            }
        self.rebuild_rows();
    }

    fn visible_len(&self) -> usize {
        self.rows.len()
    }

    fn node_at(&self, path: &[usize]) -> Option<&Node> {
        let mut n = self.roots.get(*path.first()?)?;
        for &i in &path[1..] {
            n = n.children.get(i)?;
        }
        Some(n)
    }

    fn node_at_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        let mut n = self.roots.get_mut(*path.first()?)?;
        for &i in &path[1..] {
            n = n.children.get_mut(i)?;
        }
        Some(n)
    }

    fn key_of(&self, path: &[usize]) -> String {
        let mut key = String::new();
        let mut nodes = &self.roots;
        for &i in path {
            let n = &nodes[i];
            key = n.key(&key);
            nodes = &n.children;
        }
        key
    }

    pub fn selected_node(&self) -> Option<&Node> {
        self.rows.get(self.selected).and_then(|r| self.node_at(&r.path))
    }

    pub fn selected_conn(&self) -> Option<ConnId> {
        self.selected_node().map(|n| n.conn)
    }

    /// Where a query console for the selection should run: the database or schema the selected node
    /// is in. `database` is set for another PostgreSQL database, which needs its own connection.
    pub fn console_target(&self) -> Option<Action> {
        let path = self.rows.get(self.selected)?.path.clone();
        let root = self.roots.get(*path.first()?)?;
        let NodeKind::Connection { backend, .. } = root.kind else { return None };
        let mut scope = None;
        let mut database = None;
        let mut nodes = &self.roots;
        for &i in &path {
            let n = nodes.get(i)?;
            match &n.kind {
                NodeKind::Database { name, current: false } => database = Some(name.clone()),
                NodeKind::Schema { name, database: Some(_) } if backend == Backend::MySql => scope = Some(Scope::Database(name.clone())),
                NodeKind::Schema { name, database: None } if backend == Backend::Postgres => scope = Some(Scope::Schema(name.clone())),
                _ => {}
            }
            nodes = &n.children;
        }
        Some(Action::NewConsole { conn: root.conn, scope, database })
    }

    pub fn select_connection(&mut self, conn: ConnId) {
        if let Some(i) = self.rows.iter().position(|r| r.path.len() == 1 && self.roots.get(r.path[0]).is_some_and(|n| n.conn == conn)) {
            self.selected = i;
        }
    }

    fn rebuild_rows(&mut self) {
        let filter = self.filter_text.to_lowercase();
        let mut rows = Vec::new();
        fn walk(nodes: &[Node], depth: usize, path: &mut Vec<usize>, guides: &mut Vec<bool>, filter: &str, out: &mut Vec<Row>) {
            let n_nodes = nodes.len();
            for (i, n) in nodes.iter().enumerate() {
                if !filter.is_empty() && !subtree_matches(n, filter) {
                    continue;
                }
                path.push(i);
                let last = i + 1 == n_nodes;
                out.push(Row { depth, path: path.clone(), guides: guides.clone(), last });
                let open = n.expanded || (!filter.is_empty() && n.children.iter().any(|c| subtree_matches(c, filter)));
                if open {
                    guides.push(last);
                    walk(&n.children, depth + 1, path, guides, filter, out);
                    guides.pop();
                }
                path.pop();
            }
        }
        walk(&self.roots, 0, &mut Vec::new(), &mut Vec::new(), &filter, &mut rows);
        self.rows = rows;
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
    }

    fn toggle(&mut self, open: Option<bool>) -> Option<Action> {
        let path = self.rows.get(self.selected)?.path.clone();
        let key = self.key_of(&path);
        let node = self.node_at_mut(&path)?;
        if !node.expandable() {
            return None;
        }
        let target = open.unwrap_or(!node.expanded);
        node.expanded = target;
        let action = if target && node.lazy {
            if let NodeKind::Schema { name, .. } = &node.kind {
                node.children = vec![Node::new(node.conn, NodeKind::Message, "loading…")];
                Some(Action::LoadRelations { conn: node.conn, schema: name.clone() })
            } else {
                None
            }
        } else {
            None
        };
        if target {
            self.expanded_keys.insert(key);
        } else {
            self.expanded_keys.remove(&key);
        }
        self.rebuild_rows();
        action
    }

    fn move_to_parent(&mut self) {
        if let Some(row) = self.rows.get(self.selected)
            && row.path.len() > 1 {
                let parent = row.path[..row.path.len() - 1].to_vec();
                if let Some(i) = self.rows.iter().position(|r| r.path == parent) {
                    self.selected = i;
                }
            }
    }

    fn activate(&mut self) -> Option<Action> {
        let node = self.selected_node()?.clone();
        match &node.kind {
            NodeKind::Relation { schema, name, .. } => {
                Some(Action::OpenTable { conn: node.conn, schema: schema.clone(), name: name.clone() })
            }
            NodeKind::Database { name, current: false } => Some(Action::SwitchDatabase { conn: node.conn, name: name.clone() }),
            NodeKind::Function { schema, name, .. } => {
                Some(Action::ShowFunction { conn: node.conn, schema: schema.clone(), name: name.clone() })
            }
            NodeKind::Column { .. } => Some(Action::InsertText(node.label.clone())),
            _ => self.toggle(None),
        }
    }

    fn relation_target(&self) -> Option<(ConnId, String, String)> {
        match &self.selected_node()?.kind {
            NodeKind::Relation { schema, name, .. } => Some((self.selected_node()?.conn, schema.clone(), name.clone())),
            _ => None,
        }
    }

    /// After `g` on a table, the next key picks the statement to write.
    pub fn awaiting_script_key(&self) -> bool {
        self.pending_g
    }

    pub fn is_filtering(&self) -> bool {
        self.filter.is_some()
    }

    fn filter_changed(&mut self) {
        self.filter_text = self.filter.as_ref().map(Input::value).unwrap_or_default();
        self.selected = 0;
        self.rebuild_rows();
        self.select_first_match();
    }

    pub fn handle_paste(&mut self, text: &str) {
        if let Some(input) = &mut self.filter {
            input.handle_paste(text);
            self.filter_changed();
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if let Some(input) = &mut self.filter {
            match input.handle_key(key) {
                InputEvent::Changed => self.filter_changed(),
                InputEvent::Submit => {
                    self.filter = None;
                    return self.activate();
                }
                InputEvent::Cancel => {
                    self.filter = None;
                    self.filter_text.clear();
                    self.rebuild_rows();
                }
                InputEvent::Unhandled => match key.code {
                    KeyCode::Down => self.move_by(1),
                    KeyCode::Up => self.move_by(-1),
                    _ => {}
                },
                InputEvent::Moved => {}
            }
            return None;
        }
        if self.pending_g {
            self.pending_g = false;
            let (conn, schema, name) = self.relation_target()?;
            let what = match key.code {
                KeyCode::Char('s') => Script::Select,
                KeyCode::Char('i') => Script::Insert,
                KeyCode::Char('u') => Script::Update,
                KeyCode::Char('d') => Script::Delete,
                KeyCode::Char('c') => Script::Create,
                KeyCode::Char('x') => Script::Drop,
                KeyCode::Char('n') => Script::Count,
                _ => return None,
            };
            return Some(Action::Generate { conn, schema, name, what });
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::PageDown => self.move_by(self.area.height.max(2) as isize - 1),
            KeyCode::PageUp => self.move_by(-(self.area.height.max(2) as isize - 1)),
            KeyCode::Char('d') if ctrl => self.move_by(self.area.height as isize / 2),
            KeyCode::Char('u') if ctrl => self.move_by(-(self.area.height as isize / 2)),
            KeyCode::Home => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.selected = self.rows.len().saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => {
                let expandable = self.selected_node().is_some_and(|n| n.expandable() && !n.expanded);
                if expandable {
                    return self.toggle(Some(true));
                }
                self.move_by(1);
            }
            KeyCode::Left | KeyCode::Char('h') => {
                let open = self.selected_node().is_some_and(|n| n.expanded && n.expandable());
                if open {
                    return self.toggle(Some(false));
                }
                self.move_to_parent();
            }
            KeyCode::Enter => return self.activate(),
            KeyCode::Char(' ') => return self.toggle(None),
            KeyCode::Char('/') => {
                self.filter = Some(Input::new(&self.filter_text).with_placeholder("filter…"));
            }
            KeyCode::Char('g') => {
                if self.relation_target().is_some() {
                    self.pending_g = true;
                } else {
                    self.selected = 0;
                }
            }
            KeyCode::Char('s') => {
                let (conn, schema, name) = self.relation_target()?;
                return Some(Action::OpenStructure { conn, schema, name });
            }
            KeyCode::Char('i') => {
                let n = self.selected_node()?;
                return Some(Action::InsertText(match &n.kind {
                    NodeKind::Relation { schema, name, .. } => format!("{schema}.{name}"),
                    _ => n.label.clone(),
                }));
            }
            KeyCode::Char('r') | KeyCode::F(5) => return self.selected_conn().map(Action::Refresh),
            KeyCode::Char('n') => return Some(Action::NewConnection),
            KeyCode::Char('c') => return self.console_target(),
            KeyCode::Char('x') if ctrl => return self.selected_conn().map(Action::Disconnect),
            KeyCode::Esc if !self.filter_text.is_empty() => {
                self.filter_text.clear();
                self.rebuild_rows();
            }
            _ => {}
        }
        None
    }

    fn select_first_match(&mut self) {
        let f = self.filter_text.to_lowercase();
        if let Some(i) = self.rows.iter().position(|r| self.node_at(&r.path).is_some_and(|n| label_matches(n, &f) && is_leafish(n))) {
            self.selected = i;
        }
    }

    fn move_by(&mut self, d: isize) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        self.selected = (self.selected as isize + d).clamp(0, n - 1) as usize;
    }

    pub fn handle_mouse(&mut self, ev: MouseEvent) -> Option<Action> {
        let a = self.area;
        if ev.column < a.x || ev.column >= a.x + a.width || ev.row < a.y || ev.row >= a.y + a.height {
            return None;
        }
        match ev.kind {
            MouseEventKind::ScrollDown => {
                self.offset = (self.offset + 3).min(self.rows.len().saturating_sub(1));
                None
            }
            MouseEventKind::ScrollUp => {
                self.offset = self.offset.saturating_sub(3);
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let idx = self.offset + (ev.row - a.y) as usize;
                if idx < self.rows.len() {
                    let was = self.selected == idx;
                    self.selected = idx;
                    let expandable = self.selected_node().is_some_and(|n| n.expandable());
                    if expandable {
                        return self.toggle(None);
                    }
                    if was {
                        return self.activate();
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme, focused: bool) {
        let (list_area, filter_area) = if self.filter.is_some() || !self.filter_text.is_empty() {
            (Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area }, Some(Rect { height: 1, ..area }))
        } else {
            (area, None)
        };
        if let Some(fa) = filter_area {
            buf.set_string(fa.x, fa.y, format!("{} ", icons::get().search), Style::default().fg(theme.accent));
            let input_area = Rect { x: fa.x + 2, width: fa.width.saturating_sub(2), ..fa };
            match &mut self.filter {
                Some(input) => {
                    input.render(input_area, buf, theme, focused);
                }
                None => {
                    buf.set_stringn(input_area.x, input_area.y, &self.filter_text, input_area.width as usize, Style::default().fg(theme.muted));
                }
            }
        }
        self.area = list_area;
        let h = list_area.height as usize;
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if h > 0 && self.selected >= self.offset + h {
            self.offset = self.selected + 1 - h;
        }
        let filter = self.filter_text.to_lowercase();
        for (i, row) in self.rows.iter().enumerate().skip(self.offset).take(h) {
            let y = list_area.y + (i - self.offset) as u16;
            let Some(node) = self.node_at(&row.path) else { continue };
            let selected = i == self.selected;
            let line = node_line(node, row, theme, &filter);
            let row_area = Rect { x: list_area.x, y, width: list_area.width.saturating_sub(1), height: 1 };
            if selected {
                let bg = if focused { theme.selection } else { theme.highlight };
                buf.set_style(row_area, Style::default().bg(bg));
            }
            let used = line.width() as u16;
            line.render(row_area, buf);
            if !node.detail.is_empty() {
                let d = &node.detail;
                let w = d.width() as u16;
                if used + w + 2 <= row_area.width {
                    buf.set_string(row_area.x + row_area.width - w, y, d, Style::default().fg(theme.muted));
                }
            }
        }
        if self.rows.len() > h && h > 0 {
            let mut st = ScrollbarState::new(self.rows.len().saturating_sub(h)).position(self.offset);
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some(" "))
                .thumb_symbol(crate::icons::glyph("█"))
                .thumb_style(Style::default().fg(theme.border))
                .render(list_area, buf, &mut st);
        }
    }
}

fn node_line<'a>(node: &'a Node, row: &Row, theme: &Theme, filter: &str) -> Line<'a> {
    let mut spans = Vec::new();
    let guide = Style::default().fg(theme.border);
    let mut prefix = String::from(" ");
    for &ancestor_last in row.guides.iter().skip(1) {
        prefix.push_str(if ancestor_last { "  " } else { "│ " });
    }
    if row.depth > 0 {
        prefix.push_str(if row.last { "╰ " } else { "├ " });
    }
    spans.push(Span::styled(prefix, guide));
    let ic = icons::get();
    let arrow = if node.expandable() { if node.expanded { ic.expanded } else { ic.collapsed } } else { " " };
    spans.push(Span::styled(format!("{arrow} "), Style::default().fg(theme.muted)));
    let (icon, style) = match &node.kind {
        NodeKind::Connection { backend, mariadb } => (
            ic.connection(*backend, *mariadb),
            Style::default().fg(node.color.unwrap_or(theme.accent)).add_modifier(Modifier::BOLD),
        ),
        NodeKind::DatabasesGroup => (ic.databases, Style::default().fg(theme.muted)),
        NodeKind::Database { current: true, .. } => (ic.database_current, Style::default().fg(theme.success)),
        NodeKind::Database { .. } => (ic.database, Style::default().fg(theme.fg)),
        NodeKind::Schema { database: Some(true), .. } => (ic.database_current, Style::default().fg(theme.success)),
        NodeKind::Schema { database: Some(false), .. } => (ic.database, Style::default().fg(theme.accent2)),
        NodeKind::Schema { .. } => (ic.schema, Style::default().fg(theme.accent2)),
        NodeKind::Group { .. } => (ic.group, Style::default().fg(theme.muted)),
        NodeKind::Relation { kind, .. } => match kind {
            RelKind::View => (ic.view, Style::default().fg(theme.info)),
            RelKind::MaterializedView => (ic.matview, Style::default().fg(theme.info)),
            RelKind::ForeignTable => (ic.foreign_table, Style::default().fg(theme.fg)),
            RelKind::SystemTable => (ic.system_table, Style::default().fg(theme.muted)),
            _ => (ic.table, Style::default().fg(theme.fg)),
        },
        NodeKind::Column { pk: true, .. } => (ic.key, Style::default().fg(theme.warning)),
        NodeKind::Column { .. } => (ic.column, Style::default().fg(theme.identifier)),
        NodeKind::Function { .. } => (ic.function, Style::default().fg(theme.function)),
        NodeKind::Message => ("", Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC)),
    };
    let icon = if icon.is_empty() { String::new() } else { format!("{icon} ") };
    let icon_style = match &node.kind {
        NodeKind::Relation { .. } => Style::default().fg(theme.accent),
        _ => style,
    };
    spans.push(Span::styled(icon, icon_style));
    if !filter.is_empty()
        && let Some(pos) = node.label.to_lowercase().find(filter)
            && node.label.is_char_boundary(pos) && node.label.is_char_boundary(pos + filter.len()) {
                spans.push(Span::styled(&node.label[..pos], style));
                spans.push(Span::styled(
                    &node.label[pos..pos + filter.len()],
                    style.fg(theme.warning).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                ));
                spans.push(Span::styled(&node.label[pos + filter.len()..], style));
                return Line::from(spans);
            }
    spans.push(Span::styled(node.label.as_str(), style));
    if let NodeKind::Column { data_type, nullable, .. } = &node.kind {
        spans.push(Span::styled(format!(" {data_type}"), Style::default().fg(theme.datatype).add_modifier(Modifier::DIM)));
        if !nullable {
            spans.push(Span::styled(format!(" {}", ic.not_null), Style::default().fg(theme.muted)));
        }
    }
    Line::from(spans)
}

fn label_matches(n: &Node, filter: &str) -> bool {
    n.label.to_lowercase().contains(filter)
}

fn is_leafish(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Relation { .. } | NodeKind::Function { .. } | NodeKind::Column { .. } | NodeKind::Database { .. })
}

fn subtree_matches(n: &Node, filter: &str) -> bool {
    if matches!(n.kind, NodeKind::Connection { .. }) {
        return true;
    }
    (is_leafish(n) && label_matches(n, filter)) || n.children.iter().any(|c| subtree_matches(c, filter))
}

fn restore_expansion(node: &mut Node, parent: &str, keys: &HashSet<String>) {
    let key = node.key(parent);
    if keys.contains(&key) {
        node.expanded = true;
    }
    for c in &mut node.children {
        restore_expansion(c, &key, keys);
    }
}

pub fn is_system_schema(name: &str, backend: Backend) -> bool {
    match backend {
        Backend::Postgres => name == "pg_catalog" || name == "information_schema" || name.starts_with("pg_toast"),
        Backend::MySql => matches!(name, "information_schema" | "mysql" | "performance_schema" | "sys"),
        Backend::Sqlite => false,
    }
}

fn schema_children(conn: ConnId, schema: &str, rels: &[Relation], funcs: &[crate::db::FunctionInfo]) -> Vec<Node> {
    let mut groups = Vec::new();
    for (what, pred) in [
        (GroupKind::Tables, (|k: RelKind| matches!(k, RelKind::Table | RelKind::PartitionedTable | RelKind::ForeignTable | RelKind::SystemTable)) as fn(RelKind) -> bool),
        (GroupKind::Views, |k| k == RelKind::View),
        (GroupKind::MaterializedViews, |k| k == RelKind::MaterializedView),
    ] {
        let mut items: Vec<&Relation> = rels.iter().filter(|r| pred(r.kind)).collect();
        if items.is_empty() {
            continue;
        }
        items.sort_by(|a, b| a.name.cmp(&b.name));
        let mut g = Node::new(conn, NodeKind::Group { schema: schema.to_string(), what }, what.label());
        g.detail = items.len().to_string();
        for r in items {
            let mut n = Node::new(
                conn,
                NodeKind::Relation { schema: schema.to_string(), name: r.name.clone(), kind: r.kind },
                r.name.clone(),
            );
            if let Some(est) = r.row_estimate.filter(|e| *e > 0) {
                n.detail = compact_count(est);
            }
            for c in &r.columns {
                let mut cn = Node::new(
                    conn,
                    NodeKind::Column { data_type: c.data_type.clone(), pk: c.primary_key, nullable: c.nullable },
                    c.name.clone(),
                );
                cn.detail = String::new();
                n.children.push(cn);
            }
            g.children.push(n);
        }
        groups.push(g);
    }
    for (what, kinds) in [
        (GroupKind::Functions, &[FunctionKind::Function, FunctionKind::Aggregate, FunctionKind::Window, FunctionKind::Trigger][..]),
        (GroupKind::Procedures, &[FunctionKind::Procedure][..]),
    ] {
        let mut fs: Vec<&crate::db::FunctionInfo> = funcs.iter().filter(|f| kinds.contains(&f.kind)).collect();
        if fs.is_empty() {
            continue;
        }
        fs.sort_by(|a, b| a.name.cmp(&b.name));
        let mut g = Node::new(conn, NodeKind::Group { schema: schema.to_string(), what }, what.label());
        g.detail = fs.len().to_string();
        for f in fs {
            let mut n = Node::new(
                conn,
                NodeKind::Function { schema: schema.to_string(), name: f.name.clone(), signature: f.args.clone() },
                f.name.clone(),
            );
            n.detail = f.return_type.clone();
            g.children.push(n);
        }
        groups.push(g);
    }
    groups
}

pub fn compact_count(n: i64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 10_000 => format!("{:.0}k", n as f64 / 1e3),
        n if n >= 1_000 => format!("{:.1}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ColumnInfo, SchemaInfo};

    fn catalog() -> Catalog {
        let col = |n: &str, pk: bool| ColumnInfo {
            name: n.into(),
            data_type: "int".into(),
            nullable: !pk,
            default: None,
            primary_key: pk,
            auto: false,
            comment: None,
        };
        let rel = |n: &str, kind| Relation {
            schema: "public".into(),
            name: n.into(),
            kind,
            columns: vec![col("id", true), col("user_id", false)],
            comment: None,
            row_estimate: Some(1500),
        };
        let mut c = Catalog::empty(Backend::Postgres);
        c.search_path = vec!["public".into()];
        c.schemas = vec![
            SchemaInfo { name: "public".into(), relations: vec![rel("users", RelKind::Table), rel("orders", RelKind::Table), rel("v", RelKind::View)], functions: vec![], types: vec![] },
            SchemaInfo { name: "pg_catalog".into(), relations: vec![], functions: vec![], types: vec![] },
        ];
        c
    }

    fn labels(s: &Sidebar) -> Vec<String> {
        s.rows.iter().map(|r| s.node_at(&r.path).unwrap().label.clone()).collect()
    }

    fn select(s: &mut Sidebar, label: &str) {
        s.selected = labels(s).iter().position(|l| l == label).unwrap_or_else(|| panic!("no {label} in {:?}", labels(s)));
    }

    /// A console opened from the explorer runs where the selection lives: the table's schema on
    /// PostgreSQL, its database on MySQL, and another PostgreSQL database over its own connection.
    #[test]
    fn console_target_follows_the_selection() {
        let mut s = Sidebar::default();
        s.set_connection(0, "pg", "pg", Backend::Postgres, false);
        let mut cat = catalog();
        cat.databases = vec!["app".into(), "other".into()];
        cat.current_database = Some("app".into());
        s.set_catalog(0, &cat, false);
        select(&mut s, "users");
        assert_eq!(s.console_target(), Some(Action::NewConsole { conn: 0, scope: Some(Scope::Schema("public".into())), database: None }));
        select(&mut s, "Databases");
        s.toggle(Some(true));
        select(&mut s, "other");
        assert_eq!(s.console_target(), Some(Action::NewConsole { conn: 0, scope: None, database: Some("other".into()) }));
        select(&mut s, "app");
        assert_eq!(s.console_target(), Some(Action::NewConsole { conn: 0, scope: None, database: None }), "the current database needs nothing");

        let mut my = Sidebar::default();
        my.set_connection(1, "my", "mysql", Backend::MySql, false);
        let mut cat = Catalog::empty(Backend::MySql);
        cat.databases = vec!["shop".into()];
        cat.schemas = vec![SchemaInfo { name: "shop".into(), relations: vec![], functions: vec![], types: vec![] }];
        my.set_catalog(1, &cat, false);
        select(&mut my, "shop");
        assert_eq!(my.console_target(), Some(Action::NewConsole { conn: 1, scope: Some(Scope::Database("shop".into())), database: None }));
    }

    #[test]
    fn default_schema_tables_are_expanded_and_system_schemas_hidden() {
        let mut s = Sidebar::default();
        s.set_connection(0, "local", "pg", Backend::Postgres, false);
        s.set_catalog(0, &catalog(), false);
        assert_eq!(labels(&s), vec!["local", "public", "Tables", "orders", "users", "Views"]);
    }

    #[test]
    fn filter_reveals_matching_leaves_inside_collapsed_groups() {
        let mut s = Sidebar::default();
        s.set_connection(0, "local", "pg", Backend::Postgres, false);
        s.set_catalog(0, &catalog(), false);
        s.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for c in "v".chars() {
            s.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert!(labels(&s).contains(&"v".to_string()));
        assert!(!labels(&s).contains(&"orders".to_string()));
    }

    #[test]
    fn a_paste_filters_like_typing_and_is_ignored_outside_the_filter() {
        let mut s = Sidebar::default();
        s.set_connection(0, "local", "pg", Backend::Postgres, false);
        s.set_catalog(0, &catalog(), false);
        let before = labels(&s);
        s.handle_paste("orders");
        assert_eq!(labels(&s), before);
        s.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        s.handle_paste("orders\n");
        assert!(labels(&s).contains(&"orders".to_string()));
        assert!(!labels(&s).contains(&"users".to_string()));
    }

    #[test]
    fn enter_on_table_opens_it_and_expansion_survives_refresh() {
        let mut s = Sidebar::default();
        s.set_connection(0, "local", "pg", Backend::Postgres, false);
        s.set_catalog(0, &catalog(), false);
        s.selected = 3;
        assert_eq!(
            s.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Action::OpenTable { conn: 0, schema: "public".into(), name: "orders".into() })
        );
        s.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let before = labels(&s);
        s.set_catalog(0, &catalog(), false);
        assert_eq!(labels(&s), before);
        assert!(before.contains(&"user_id".to_string()));
    }

    #[test]
    fn script_generation_prefix() {
        let mut s = Sidebar::default();
        s.set_connection(0, "local", "pg", Backend::Postgres, false);
        s.set_catalog(0, &catalog(), false);
        s.selected = 4;
        s.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        let a = s.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert_eq!(a, Some(Action::Generate { conn: 0, schema: "public".into(), name: "users".into(), what: Script::Select }));
    }

    #[test]
    fn counts_are_compact() {
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1500), "1.5k");
        assert_eq!(compact_count(2_500_000), "2.5M");
    }
}
