use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use super::widgets::input::{Input, InputEvent};
use crate::config::SavedConnection;
use crate::conn::{ConnSpec, SslMode};
use crate::db::Backend;
use crate::icons;
use crate::theme::Theme;

pub fn centered(screen: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(screen.width.saturating_sub(2));
    let h = height.min(screen.height.saturating_sub(2));
    Rect { x: screen.x + (screen.width - w) / 2, y: screen.y + (screen.height - h) / 2, width: w, height: h }
}

pub fn frame(area: Rect, buf: &mut Buffer, theme: &Theme, title: &str, accent: ratatui::style::Color) -> Rect {
    let title = Span::styled(format!(" {title} "), Style::default().fg(accent).add_modifier(Modifier::BOLD));
    modal(area, buf, theme, Some(title), accent)
}

/// A floating box: the border sits on the app background so its rounded corners read as round,
/// and only the inside takes the popup colour.
pub fn modal(area: Rect, buf: &mut Buffer, theme: &Theme, title: Option<Span<'_>>, border: ratatui::style::Color) -> Rect {
    Clear.render(area, buf);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border).bg(theme.bg))
        .style(Style::default().bg(theme.bg).fg(theme.fg));
    if let Some(t) = title {
        block = block.title(t);
    }
    let inner = block.inner(area);
    block.render(area, buf);
    buf.set_style(inner, Style::default().bg(theme.surface).fg(theme.fg));
    inner
}

fn button(buf: &mut Buffer, x: u16, y: u16, label: &str, key: &str, style: Style, theme: &Theme) -> u16 {
    let text = format!(" {label} ");
    buf.set_string(x, y, &text, style);
    let kx = x + text.width() as u16;
    buf.set_string(kx, y, format!(" {key}"), Style::default().fg(theme.muted));
    kx + key.width() as u16 + 3
}

/// Where an overlay was drawn, for mouse handling: clicks outside close it, buttons answer it.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModalLayout {
    pub area: Rect,
    pub yes: Option<Rect>,
    pub cancel: Option<Rect>,
}

impl ModalLayout {
    fn at(area: Rect) -> Self {
        ModalLayout { area, ..Default::default() }
    }
}

pub enum DialogResult<T> {
    None,
    Close,
    Emit(T),
}

pub struct Confirm<T> {
    pub title: String,
    pub lines: Vec<Line<'static>>,
    pub yes: String,
    pub danger: bool,
    pub on_yes: T,
    pub scroll: u16,
}

impl<T: Clone> Confirm<T> {
    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<T> {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => DialogResult::Emit(self.on_yes.clone()),
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') => DialogResult::Close,
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = self.scroll.saturating_add(1);
                DialogResult::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll = self.scroll.saturating_sub(1);
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }

    pub fn render(&self, screen: Rect, buf: &mut Buffer, theme: &Theme) -> ModalLayout {
        let content_w = self.lines.iter().map(|l| l.width()).max().unwrap_or(20) as u16;
        let width = (content_w + 6).clamp(44, screen.width.saturating_sub(4).max(44));
        let height = (self.lines.len() as u16 + 5).clamp(7, screen.height.saturating_sub(2));
        let area = centered(screen, width, height);
        let accent = if self.danger { theme.error } else { theme.border_focus };
        let inner = frame(area, buf, theme, &self.title, accent);
        let body = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), height: inner.height.saturating_sub(2), ..inner };
        Paragraph::new(self.lines.clone()).wrap(Wrap { trim: false }).scroll((self.scroll, 0)).render(body, buf);
        let y = inner.y + inner.height - 1;
        let yes_style = if self.danger {
            Style::default().bg(theme.error).fg(theme.bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(theme.accent).fg(theme.bg).add_modifier(Modifier::BOLD)
        };
        let x = button(buf, inner.x + 1, y, &self.yes, &format!("{}/y", crate::icons::enter()), yes_style, theme);
        let end = button(buf, x, y, "Cancel", "esc", Style::default().bg(theme.highlight).fg(theme.fg), theme);
        ModalLayout {
            area,
            yes: Some(Rect { x: inner.x + 1, y, width: x.saturating_sub(inner.x + 1), height: 1 }),
            cancel: Some(Rect { x, y, width: end.saturating_sub(x), height: 1 }),
        }
    }
}

pub struct Prompt<T> {
    pub title: String,
    pub hint: String,
    pub input: Input,
    pub purpose: T,
}

impl<T: Clone> Prompt<T> {
    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<(T, String)> {
        match self.input.handle_key(key) {
            InputEvent::Submit => DialogResult::Emit((self.purpose.clone(), self.input.value())),
            InputEvent::Cancel => DialogResult::Close,
            _ => DialogResult::None,
        }
    }

    pub fn render(&mut self, screen: Rect, buf: &mut Buffer, theme: &Theme) -> (ModalLayout, Option<(u16, u16)>) {
        let width = (screen.width * 3 / 5).clamp(40, 100);
        let hint_lines = if self.hint.is_empty() { 0 } else { 1 + self.hint.width() as u16 / width.max(1) };
        let area = centered(screen, width, 5 + hint_lines);
        let inner = frame(area, buf, theme, &self.title, theme.border_focus);
        let mut y = inner.y;
        if !self.hint.is_empty() {
            let r = Rect { x: inner.x + 1, y, width: inner.width.saturating_sub(2), height: hint_lines };
            Paragraph::new(self.hint.clone()).style(Style::default().fg(theme.muted)).wrap(Wrap { trim: true }).render(r, buf);
            y += hint_lines;
        }
        let field = Rect { x: inner.x + 1, y: y + 1, width: inner.width.saturating_sub(2), height: 1 };
        buf.set_style(field, Style::default().bg(theme.highlight));
        (ModalLayout::at(area), self.input.render(field, buf, theme, true))
    }
}

pub struct TextView {
    pub title: String,
    pub text: String,
    pub scroll: usize,
    pub hscroll: usize,
    pub footer: String,
}

impl TextView {
    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<()> {
        let lines = self.text.lines().count();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => return DialogResult::Close,
            KeyCode::Down | KeyCode::Char('j') => self.scroll = (self.scroll + 1).min(lines.saturating_sub(1)),
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll = (self.scroll + 20).min(lines.saturating_sub(1)),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(20),
            KeyCode::Right | KeyCode::Char('l') => self.hscroll += 4,
            KeyCode::Left | KeyCode::Char('h') => self.hscroll = self.hscroll.saturating_sub(4),
            KeyCode::Home | KeyCode::Char('g') => self.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.scroll = lines.saturating_sub(1),
            KeyCode::Char('y') => return DialogResult::Emit(()),
            _ => {}
        }
        DialogResult::None
    }

    pub fn render(&self, screen: Rect, buf: &mut Buffer, theme: &Theme) -> ModalLayout {
        let longest = self.text.lines().map(|l| l.width()).max().unwrap_or(10) as u16;
        let width = (longest + 8).clamp(50, screen.width.saturating_sub(4).max(50));
        let height = (self.text.lines().count() as u16 + 4).clamp(8, screen.height.saturating_sub(4).max(8));
        let area = centered(screen, width, height);
        let inner = frame(area, buf, theme, &self.title, theme.border_focus);
        let body = Rect { height: inner.height.saturating_sub(1), ..inner };
        for (i, line) in self.text.lines().skip(self.scroll).take(body.height as usize).enumerate() {
            let shown: String = line.chars().skip(self.hscroll).collect();
            let spans = json_spans(&shown, theme);
            Line::from(spans).render(Rect { x: body.x + 1, y: body.y + i as u16, width: body.width.saturating_sub(2), height: 1 }, buf);
        }
        let fy = inner.y + inner.height - 1;
        buf.set_stringn(inner.x + 1, fy, &self.footer, inner.width.saturating_sub(2) as usize, Style::default().fg(theme.muted));
        ModalLayout::at(area)
    }
}

/// Light JSON-ish coloring for the value viewer (keys, strings, numbers, literals).
fn json_spans(line: &str, theme: &Theme) -> Vec<Span<'static>> {
    let trimmed = line.trim_start();
    if !(trimmed.starts_with('"') || trimmed.starts_with('{') || trimmed.starts_with('[') || trimmed.starts_with('}') || trimmed.starts_with(']')) {
        return vec![Span::styled(line.to_string(), Style::default().fg(theme.fg))];
    }
    let mut spans = Vec::new();
    let mut chars = line.char_indices().peekable();
    let mut buf = String::new();
    while let Some((_, c)) = chars.next() {
        if c == '"' {
            if !buf.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut buf), Style::default().fg(theme.punctuation)));
            }
            let mut s = String::from('"');
            let mut escaped = false;
            for (_, c2) in chars.by_ref() {
                s.push(c2);
                if escaped {
                    escaped = false;
                } else if c2 == '\\' {
                    escaped = true;
                } else if c2 == '"' {
                    break;
                }
            }
            let is_key = chars.peek().is_some_and(|(_, n)| *n == ':');
            let color = if is_key { theme.accent } else { theme.string };
            spans.push(Span::styled(s, Style::default().fg(color)));
        } else if c.is_ascii_digit() || c == '-' {
            if !buf.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut buf), Style::default().fg(theme.punctuation)));
            }
            let mut s = String::from(c);
            while let Some((_, n)) = chars.peek() {
                if n.is_ascii_digit() || matches!(n, '.' | 'e' | 'E' | '+' | '-') {
                    s.push(*n);
                    chars.next();
                } else {
                    break;
                }
            }
            spans.push(Span::styled(s, Style::default().fg(theme.number)));
        } else if c.is_ascii_alphabetic() {
            if !buf.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut buf), Style::default().fg(theme.punctuation)));
            }
            let mut s = String::from(c);
            while let Some((_, n)) = chars.peek() {
                if n.is_ascii_alphabetic() {
                    s.push(*n);
                    chars.next();
                } else {
                    break;
                }
            }
            let color = if s == "null" { theme.null } else { theme.boolean };
            spans.push(Span::styled(s, Style::default().fg(color)));
        } else {
            buf.push(c);
        }
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, Style::default().fg(theme.punctuation)));
    }
    spans
}

fn truncate_to(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + c.to_string().width() + 1 > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// One line of the shortcuts list.
#[derive(Clone, Debug)]
pub struct HelpRow {
    pub section: String,
    pub keys: String,
    pub what: String,
    /// The `[keys]` name of a rebindable action; empty for fixed keys.
    pub action: String,
}

/// The keyboard shortcuts, filtered as you type.
pub struct HelpView {
    rows: Vec<HelpRow>,
    filter: Input,
    scroll: usize,
}

/// Keys that can't be rebound: the ones a pane handles beyond its `[keys]` actions.
pub const FIXED_KEYS: &[(&str, &[(&str, &str)])] = &[
    ("Global", &[
        ("Alt+1…9", "Jump to tab"),
    ]),
    ("Editor", &[
        ("Tab / Shift+Tab", "Indent / dedent lines"),
        ("Esc", "Go to the results"),
    ]),
    ("Vim (vi = true)", &[
        ("i a I A o O", "Insert before / after / at line start / at line end / new line below / above"),
        ("Esc", "Back to normal mode (again: go to the results)"),
        ("h j k l · w b e · 0 ^ $ · gg G", "Move; a count repeats: 3j, 2w, 10G"),
        ("d c y > <  + motion", "Delete, change, yank, indent, dedent: dw, c$, y2j, >G"),
        ("dd cc yy >> <<", "The whole line (with a count: 3dd)"),
        ("ciw diw yaw", "Change, delete, yank a word"),
        ("x X s S D C Y r J", "Delete char, before, substitute, line, to end; replace char; join lines"),
        ("p P", "Paste after / before (lines go below / above)"),
        ("u · Ctrl+R", "Undo / redo"),
        ("v · V", "Visual / visual-line selection, then d c y > <"),
        ("Ctrl+D / Ctrl+U", "Half a page down / up"),
        ("/ · n · N", "Search for text, then next / previous match"),
    ]),
    ("Results grid", &[
        ("Shift+arrows", "Select cells"),
        ("Esc", "Clear the selection, then back to the editor"),
    ]),
    ("Table view", &[
        ("Esc", "Clear the filter"),
    ]),
    ("Explorer", &[
        ("s i u d c x n, after the script key", "Write a SELECT / INSERT / UPDATE / DELETE / CREATE / DROP / COUNT"),
    ]),
    ("Structure tab", &[
        ("1 … 7", "Jump to a section"),
    ]),
    ("Activity tab", &[
        ("Enter", "Session details"),
    ]),
    ("Definition and DDL", &[
        ("j k · PgUp PgDn · g G", "Scroll"),
    ]),
    ("Mouse", &[
        ("Click", "Focus a pane, pick a tab, a row, a button or a list item"),
        ("Middle click on a tab", "Close it"),
        ("Drag a border", "Resize the explorer or the editor / results split"),
        ("Wheel", "Scroll; in lists, move the selection"),
        ("Click outside a dialog", "Close it"),
    ]),
];

impl HelpView {
    /// `configurable` lists the rebindable actions with their current keys; the fixed keys follow.
    pub fn new(configurable: Vec<HelpRow>) -> Self {
        let mut rows = configurable;
        for (section, keys) in FIXED_KEYS {
            for (k, what) in *keys {
                rows.push(HelpRow { section: section.to_string(), keys: k.to_string(), what: what.to_string(), action: String::new() });
            }
        }
        // keep sections together, in the order they first appear
        let order: Vec<String> = rows.iter().fold(Vec::new(), |mut v, r| {
            if !v.contains(&r.section) {
                v.push(r.section.clone());
            }
            v
        });
        rows.sort_by_key(|r| order.iter().position(|s| *s == r.section));
        HelpView { rows, filter: Input::new("").with_placeholder("type to filter, e.g. run, ctrl, vim"), scroll: 0 }
    }

    fn visible(&self) -> Vec<&HelpRow> {
        let q = self.filter.value().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        self.rows
            .iter()
            .filter(|r| {
                let hay = format!("{} {} {} {}", r.section, r.keys, r.what, r.action).to_lowercase();
                words.iter().all(|w| hay.contains(w))
            })
            .collect()
    }

    pub fn handle_paste(&mut self, text: &str) {
        self.filter.handle_paste(text);
        self.scroll = 0;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<()> {
        match key.code {
            KeyCode::Down => self.scroll += 1,
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown => self.scroll += 10,
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.set_value("");
                self.scroll = 0;
            }
            KeyCode::Esc | KeyCode::F(1) => return DialogResult::Close,
            _ => {
                if let InputEvent::Changed = self.filter.handle_key(key) {
                    self.scroll = 0;
                }
            }
        }
        DialogResult::None
    }

    pub fn render(&mut self, screen: Rect, buf: &mut Buffer, theme: &Theme) -> (ModalLayout, Option<(u16, u16)>) {
        let area = centered(screen, 100, 40);
        let inner = frame(area, buf, theme, "Keyboard shortcuts", theme.border_focus);
        buf.set_string(inner.x + 1, inner.y, format!("{} ", icons::get().search), Style::default().fg(theme.accent));
        let field = Rect { x: inner.x + 3, y: inner.y, width: inner.width.saturating_sub(4), height: 1 };
        let cursor = self.filter.render(field, buf, theme, true);
        let rows = self.visible();
        let count = format!("{}/{}", rows.len(), self.rows.len());
        buf.set_string(inner.x + inner.width - count.width() as u16 - 1, inner.y, &count, Style::default().fg(theme.muted));
        let key_w = rows.iter().map(|r| r.keys.width()).max().unwrap_or(10).clamp(10, 36);
        let mut lines: Vec<Line> = Vec::new();
        let mut section = "";
        for r in &rows {
            if r.section != section {
                if !section.is_empty() {
                    lines.push(Line::from(""));
                }
                section = &r.section;
                lines.push(Line::from(Span::styled(section.to_string(), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))));
            }
            let keys = if r.keys.is_empty() { "(unbound)".to_string() } else { r.keys.clone() };
            let lead = format!("  {keys:<key_w$}  ");
            let name_w = r.action.width();
            let room = (inner.width as usize).saturating_sub(2 + lead.width() + name_w + 2);
            let what = truncate_to(&r.what, room);
            let gap = room.saturating_sub(what.width()) + 2;
            let mut spans = vec![
                Span::styled(lead, Style::default().fg(if r.keys.is_empty() { theme.muted } else { theme.accent2 })),
                Span::styled(what, Style::default().fg(theme.fg)),
            ];
            if !r.action.is_empty() {
                spans.push(Span::styled(format!("{}{}", " ".repeat(gap), r.action), Style::default().fg(theme.muted)));
            }
            lines.push(Line::from(spans));
        }
        if rows.is_empty() {
            lines.push(Line::from(Span::styled("  No shortcut matches", Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC))));
        }
        let body = Rect { x: inner.x + 1, y: inner.y + 2, width: inner.width.saturating_sub(2), height: inner.height.saturating_sub(3) };
        let max = lines.len().saturating_sub(body.height as usize);
        self.scroll = self.scroll.min(max);
        Paragraph::new(lines).scroll((self.scroll as u16, 0)).render(body, buf);
        let hint = "Rebind an action by its name under [keys] in the config, e.g. run_all = \"f9\"";
        buf.set_stringn(inner.x + 1, inner.y + inner.height - 1, hint, inner.width.saturating_sub(2) as usize, Style::default().fg(theme.muted));
        (ModalLayout::at(area), cursor)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Backend,
    Url,
    Host,
    Port,
    User,
    Password,
    Database,
    Ssl,
    ReadOnly,
    Name,
}

const FIELDS: [Field; 10] = [
    Field::Backend,
    Field::Url,
    Field::Host,
    Field::Port,
    Field::User,
    Field::Password,
    Field::Database,
    Field::Ssl,
    Field::ReadOnly,
    Field::Name,
];

const BACKENDS: [(Backend, &str); 3] = [(Backend::Postgres, "PostgreSQL"), (Backend::MySql, "MySQL / MariaDB"), (Backend::Sqlite, "SQLite")];
const SSL_MODES: [(SslMode, &str); 5] = [
    (SslMode::Disable, "disable"),
    (SslMode::Prefer, "prefer"),
    (SslMode::Require, "require"),
    (SslMode::VerifyCa, "verify-ca"),
    (SslMode::VerifyFull, "verify-full"),
];

pub enum ConnectEvent {
    None,
    Close,
    /// Connect with this spec; `save_as` persists it in the config.
    Connect { spec: Box<ConnSpec>, save_as: Option<String>, name: String },
    Delete(String),
}

/// What a click at a spot in the dialog means; filled in while rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Saved(usize),
    NewConnection,
    Field(usize),
    Backend(Backend),
    SslPrev,
    SslNext,
    Connect,
    Back,
}

pub struct ConnectForm {
    pub saved: Vec<(String, SavedConnection)>,
    /// Names of the connections already open, marked in the list.
    pub open: Vec<String>,
    list_selected: usize,
    in_list: bool,
    field: usize,
    name: Input,
    url: Input,
    backend: Backend,
    host: Input,
    port: Input,
    user: Input,
    password: Input,
    database: Input,
    ssl: SslMode,
    readonly: bool,
    hits: Vec<(Rect, Hit)>,
    pub error: Option<String>,
    pub busy: bool,
}

impl ConnectForm {
    pub fn new(saved: Vec<(String, SavedConnection)>) -> Self {
        let in_list = !saved.is_empty();
        ConnectForm {
            saved,
            open: Vec::new(),
            list_selected: 0,
            in_list,
            field: 1,
            name: Input::new("").with_placeholder("optional; saves the connection under this name"),
            url: Input::new("").with_placeholder("paste a URL, or fill in the fields below"),
            backend: Backend::Postgres,
            host: Input::new("").with_placeholder("localhost"),
            port: Input::new("").with_placeholder("default"),
            user: Input::new("").with_placeholder("current user"),
            password: Input::new("").masked().with_placeholder("asked for if needed"),
            database: Input::new(""),
            ssl: SslMode::Prefer,
            readonly: false,
            hits: Vec::new(),
            error: None,
            busy: false,
        }
    }

    fn input_mut(&mut self, f: Field) -> Option<&mut Input> {
        Some(match f {
            Field::Name => &mut self.name,
            Field::Url => &mut self.url,
            Field::Host => &mut self.host,
            Field::Port => &mut self.port,
            Field::User => &mut self.user,
            Field::Password => &mut self.password,
            Field::Database => &mut self.database,
            _ => return None,
        })
    }

    fn visible_fields(&self) -> Vec<Field> {
        FIELDS
            .iter()
            .copied()
            .filter(|f| {
                self.backend != Backend::Sqlite
                    || !matches!(f, Field::Host | Field::Port | Field::User | Field::Password | Field::Ssl)
            })
            .collect()
    }

    pub fn build_spec(&self) -> Result<ConnSpec, String> {
        let url = self.url.value();
        let mut spec = if !url.trim().is_empty() {
            ConnSpec::parse(url.trim())?
        } else if self.backend == Backend::Sqlite {
            let path = self.database.value();
            if path.trim().is_empty() {
                return Err("SQLite needs a database file (or :memory:)".into());
            }
            ConnSpec::sqlite(crate::conn::url::expand_tilde(path.trim()))
        } else {
            let mut s = ConnSpec::new(self.backend);
            let v = |i: &Input| Some(i.value()).filter(|x| !x.trim().is_empty());
            s.host = v(&self.host);
            s.port = match v(&self.port) {
                Some(p) => Some(p.trim().parse().map_err(|_| format!("invalid port '{p}'"))?),
                None => None,
            };
            s.user = v(&self.user);
            s.database = v(&self.database);
            s.ssl_mode = self.ssl;
            s
        };
        if !self.password.is_empty() {
            spec.password = Some(self.password.value());
        }
        spec.readonly |= self.readonly;
        Ok(spec)
    }

    /// URL stored in the config: never contains the password.
    pub fn url_for_saving(spec: &ConnSpec) -> String {
        let mut url = spec.display_url();
        if spec.backend != Backend::Sqlite && spec.ssl_mode != SslMode::Prefer {
            let mode = SSL_MODES.iter().find(|(m, _)| *m == spec.ssl_mode).map_or("prefer", |(_, n)| n);
            url.push_str(&format!("?sslmode={mode}"));
        }
        url
    }

    fn connect_saved(&mut self, i: usize) -> ConnectEvent {
        let Some((name, saved)) = self.saved.get(i) else { return ConnectEvent::None };
        match ConnSpec::parse(&saved.url) {
            Ok(mut spec) => {
                spec.readonly |= saved.readonly;
                if let Some(ssh) = &saved.ssh {
                    spec.ssh = crate::conn::SshSpec::parse(ssh);
                }
                spec.init_commands.extend(saved.init_commands.iter().cloned());
                ConnectEvent::Connect { spec: Box::new(spec), save_as: None, name: name.clone() }
            }
            Err(e) => {
                self.error = Some(e);
                ConnectEvent::None
            }
        }
    }

    fn cycle_backend(&mut self, forward: bool) {
        let i = BACKENDS.iter().position(|(b, _)| *b == self.backend).unwrap_or(0);
        self.backend = BACKENDS[if forward { (i + 1) % 3 } else { (i + 2) % 3 }].0;
    }

    fn cycle_ssl(&mut self, forward: bool) {
        let n = SSL_MODES.len();
        let i = SSL_MODES.iter().position(|(m, _)| *m == self.ssl).unwrap_or(1);
        self.ssl = SSL_MODES[if forward { (i + 1) % n } else { (i + n - 1) % n }].0;
    }

    pub fn handle_paste(&mut self, text: &str) {
        if self.in_list {
            return;
        }
        let fields = self.visible_fields();
        let f = fields[self.field.min(fields.len() - 1)];
        if let Some(input) = self.input_mut(f) {
            input.handle_paste(text);
            self.error = None;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ConnectEvent {
        self.error = None;
        if self.in_list {
            match key.code {
                KeyCode::Esc => return ConnectEvent::Close,
                KeyCode::Down | KeyCode::Char('j') => self.list_selected = (self.list_selected + 1).min(self.saved.len().saturating_sub(1)),
                KeyCode::Up | KeyCode::Char('k') => self.list_selected = self.list_selected.saturating_sub(1),
                KeyCode::Tab | KeyCode::Char('n') => self.in_list = false,
                KeyCode::Char('d') | KeyCode::Delete => {
                    if let Some((name, _)) = self.saved.get(self.list_selected) {
                        return ConnectEvent::Delete(name.clone());
                    }
                }
                KeyCode::Enter => return self.connect_saved(self.list_selected),
                _ => {}
            }
            return ConnectEvent::None;
        }
        if key.code == KeyCode::Esc {
            if self.saved.is_empty() {
                return ConnectEvent::Close;
            }
            self.in_list = true;
            return ConnectEvent::None;
        }
        let fields = self.visible_fields();
        self.field = self.field.min(fields.len() - 1);
        let f = fields[self.field];
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.field = (self.field + 1) % fields.len();
                return ConnectEvent::None;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.field = (self.field + fields.len() - 1) % fields.len();
                return ConnectEvent::None;
            }
            KeyCode::Enter if !matches!(f, Field::Backend | Field::Ssl | Field::ReadOnly) || ctrl => {
                return self.submit();
            }
            _ => {}
        }
        match f {
            Field::Backend => match key.code {
                KeyCode::Left | KeyCode::Char('h') => self.cycle_backend(false),
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Enter => self.cycle_backend(true),
                _ => {}
            },
            Field::Ssl => match key.code {
                KeyCode::Left | KeyCode::Char('h') => self.cycle_ssl(false),
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Enter => self.cycle_ssl(true),
                _ => {}
            },
            Field::ReadOnly => {
                if matches!(key.code, KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Left | KeyCode::Right) {
                    self.readonly = !self.readonly;
                }
            }
            other => {
                if let Some(input) = self.input_mut(other) {
                    input.handle_key(key);
                }
            }
        }
        ConnectEvent::None
    }

    pub fn handle_mouse(&mut self, m: MouseEvent) -> ConnectEvent {
        match m.kind {
            MouseEventKind::ScrollDown if self.in_list => {
                self.list_selected = (self.list_selected + 1).min(self.saved.len().saturating_sub(1));
            }
            MouseEventKind::ScrollUp if self.in_list => self.list_selected = self.list_selected.saturating_sub(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let at = |r: &Rect| m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height;
                let Some(&(_, hit)) = self.hits.iter().find(|(r, _)| at(r)) else { return ConnectEvent::None };
                self.error = None;
                match hit {
                    Hit::Saved(i) => {
                        self.list_selected = i;
                        return self.connect_saved(i);
                    }
                    Hit::NewConnection => self.in_list = false,
                    Hit::Field(i) => {
                        self.field = i;
                        if self.visible_fields().get(i) == Some(&Field::ReadOnly) {
                            self.readonly = !self.readonly;
                        }
                    }
                    Hit::Backend(b) => {
                        self.backend = b;
                        self.field = 0;
                    }
                    Hit::SslPrev => self.cycle_ssl(false),
                    Hit::SslNext => self.cycle_ssl(true),
                    Hit::Connect => return self.submit(),
                    Hit::Back => {
                        if self.saved.is_empty() {
                            return ConnectEvent::Close;
                        }
                        self.in_list = true;
                    }
                }
            }
            _ => {}
        }
        ConnectEvent::None
    }

    fn submit(&mut self) -> ConnectEvent {
        match self.build_spec() {
            Ok(spec) => {
                let name = self.name.value().trim().to_string();
                let label = if name.is_empty() { spec.label() } else { name.clone() };
                let save_as = (!name.is_empty()).then_some(name);
                ConnectEvent::Connect { spec: Box::new(spec), save_as, name: label }
            }
            Err(e) => {
                self.error = Some(e);
                ConnectEvent::None
            }
        }
    }

    pub fn render(&mut self, screen: Rect, buf: &mut Buffer, theme: &Theme) -> (ModalLayout, Option<(u16, u16)>) {
        self.hits.clear();
        if self.in_list {
            (ModalLayout::at(self.render_list(screen, buf, theme)), None)
        } else {
            let (area, cursor) = self.render_form(screen, buf, theme);
            (ModalLayout::at(area), cursor)
        }
    }

    fn footer(&self, buf: &mut Buffer, inner: Rect, theme: &Theme, hint: &str) {
        let x = inner.x + 2;
        let w = inner.width.saturating_sub(4) as usize;
        let y = inner.y + inner.height - 1;
        if let Some(e) = &self.error {
            buf.set_stringn(x, y - 1, format!("{} {e}", icons::get().error), w, Style::default().fg(theme.error));
        } else if self.busy {
            buf.set_string(x, y - 1, "Connecting…", Style::default().fg(theme.info));
        }
        buf.set_stringn(x, y, hint, w, Style::default().fg(theme.muted));
    }

    fn render_list(&mut self, screen: Rect, buf: &mut Buffer, theme: &Theme) -> Rect {
        let ic = icons::get();
        let rows = self.saved.len() as u16 + 7;
        let area = centered(screen, 72, rows.min(screen.height.saturating_sub(2)));
        let inner = frame(area, buf, theme, "Connections", theme.border_focus);
        let x = inner.x + 1;
        let w = inner.width.saturating_sub(2);
        let list_h = inner.height.saturating_sub(5) as usize;
        let offset = self.list_selected.saturating_sub(list_h.saturating_sub(1));
        let name_w = self.saved.iter().map(|(n, _)| n.width()).max().unwrap_or(0).clamp(8, 28) as u16;
        for (row, (i, (name, c))) in self.saved.iter().enumerate().skip(offset).take(list_h).enumerate() {
            let y = inner.y + 1 + row as u16;
            let sel = i == self.list_selected;
            let line = Rect { x, y, width: w, height: 1 };
            let bg = if sel { theme.selection } else { theme.surface };
            buf.set_style(line, Style::default().bg(bg));
            let spec = ConnSpec::parse(&c.url).ok();
            let icon = spec.as_ref().map_or(ic.connection, |s| ic.connection(s.backend, false));
            buf.set_string(x + 1, y, icon, Style::default().fg(theme.accent2).bg(bg));
            let mut st = Style::default().fg(theme.fg).bg(bg);
            if sel {
                st = st.add_modifier(Modifier::BOLD);
            }
            buf.set_stringn(x + 3, y, name, name_w as usize, st);
            let target = spec.map(|s| {
                let url = s.display_url();
                url.split_once("://").map_or(url.clone(), |(_, rest)| rest.trim_end_matches('/').to_string())
            });
            let tx = x + 3 + name_w + 2;
            let marks_w = 4u16;
            let target_w = (x + w).saturating_sub(tx + marks_w) as usize;
            buf.set_stringn(tx, y, target.unwrap_or_else(|| c.url.clone()), target_w, Style::default().fg(theme.muted).bg(bg));
            let mut mx = x + w - marks_w + 1;
            if c.readonly {
                buf.set_string(mx, y, ic.readonly_mark, Style::default().fg(theme.info).bg(bg));
            }
            mx += 2;
            if self.open.contains(name) {
                buf.set_string(mx, y, ic.ok, Style::default().fg(theme.success).bg(bg));
            }
            self.hits.push((line, Hit::Saved(i)));
        }
        let ny = inner.y + 1 + list_h.min(self.saved.len()) as u16 + 1;
        let label = format!("{} New connection", ic.add);
        buf.set_string(x + 1, ny, &label, Style::default().fg(theme.accent));
        self.hits.push((Rect { x, y: ny, width: label.width() as u16 + 2, height: 1 }, Hit::NewConnection));
        self.footer(buf, inner, theme, &format!("{} connect   n new   d delete   esc close", crate::icons::enter()));
        area
    }

    fn render_form(&mut self, screen: Rect, buf: &mut Buffer, theme: &Theme) -> (Rect, Option<(u16, u16)>) {
        let ic = icons::get();
        let fields = self.visible_fields();
        let area = centered(screen, 72, fields.len() as u16 + 7);
        let inner = frame(area, buf, theme, "New connection", theme.border_focus);
        let focused = fields[self.field.min(fields.len() - 1)];
        let label_w = 10u16;
        let x = inner.x + 2;
        let vx = x + label_w;
        let vw = inner.width.saturating_sub(label_w + 4);
        let mut cursor = None;
        let mut y = inner.y + 1;
        for (i, f) in fields.iter().copied().enumerate() {
            let label = match f {
                Field::Backend => "Type",
                Field::Url => "URL",
                Field::Host => "Host",
                Field::Port => "Port",
                Field::User => "User",
                Field::Password => "Password",
                Field::Database if self.backend == Backend::Sqlite => "File",
                Field::Database => "Database",
                Field::Ssl => "TLS",
                Field::ReadOnly => "",
                Field::Name => "Save as",
            };
            if f == Field::Name {
                y += 1;
            }
            let is_focus = f == focused;
            let lstyle = if is_focus { Style::default().fg(theme.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme.muted) };
            buf.set_string(x, y, label, lstyle);
            let varea = Rect { x: vx, y, width: vw, height: 1 };
            self.hits.push((Rect { x, y, width: label_w + vw, height: 1 }, Hit::Field(i)));
            match f {
                Field::Backend => {
                    let mut bx = vx;
                    for (b, name) in BACKENDS {
                        let on = b == self.backend;
                        let st = match (on, is_focus) {
                            (true, _) => Style::default().bg(theme.accent).fg(theme.bg).add_modifier(Modifier::BOLD),
                            (false, true) => Style::default().fg(theme.fg),
                            (false, false) => Style::default().fg(theme.muted),
                        };
                        let t = format!(" {name} ");
                        buf.set_string(bx, y, &t, st);
                        self.hits.push((Rect { x: bx, y, width: t.width() as u16, height: 1 }, Hit::Backend(b)));
                        bx += t.width() as u16 + 1;
                    }
                }
                Field::Ssl => {
                    let name = SSL_MODES.iter().find(|(m, _)| *m == self.ssl).map_or("prefer", |(_, n)| n);
                    let st = if is_focus { Style::default().fg(theme.fg).bg(theme.highlight) } else { Style::default().fg(theme.fg) };
                    let arrow = Style::default().fg(theme.accent);
                    buf.set_string(vx, y, ic.more_left, arrow);
                    buf.set_string(vx + 2, y, format!("{name:<11}"), st);
                    buf.set_string(vx + 14, y, ic.more_right, arrow);
                    self.hits.push((Rect { x: vx, y, width: 2, height: 1 }, Hit::SslPrev));
                    self.hits.push((Rect { x: vx + 13, y, width: 2, height: 1 }, Hit::SslNext));
                }
                Field::ReadOnly => {
                    let mark = if self.readonly { "[x]" } else { "[ ]" };
                    let st = if is_focus { Style::default().fg(theme.fg).bg(theme.highlight) } else { Style::default().fg(theme.fg) };
                    buf.set_string(vx, y, format!("{mark} read-only"), st);
                }
                other => {
                    if is_focus {
                        buf.set_style(varea, Style::default().bg(theme.highlight));
                    }
                    if let Some(input) = self.input_mut(other) {
                        let c = input.render(varea, buf, theme, is_focus);
                        if is_focus {
                            cursor = c;
                        }
                    }
                }
            }
            y += 1;
        }
        let by = inner.y + inner.height - 1;
        let connect = " Connect ";
        let back = if self.saved.is_empty() { " Cancel " } else { " Back " };
        let cx = inner.x + inner.width - connect.width() as u16 - 2;
        let bx = cx - back.width() as u16 - 1;
        buf.set_string(cx, by, connect, Style::default().bg(theme.accent).fg(theme.bg).add_modifier(Modifier::BOLD));
        buf.set_string(bx, by, back, Style::default().bg(theme.highlight).fg(theme.fg));
        self.hits.push((Rect { x: cx, y: by, width: connect.width() as u16, height: 1 }, Hit::Connect));
        self.hits.push((Rect { x: bx, y: by, width: back.width() as u16, height: 1 }, Hit::Back));
        self.footer(buf, Rect { width: inner.width.saturating_sub(connect.width() as u16 + back.width() as u16 + 3), ..inner }, theme, &format!("{} connect  tab next  esc back", crate::icons::enter()));
        (area, cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_str(f: &mut ConnectForm, s: &str) {
        for c in s.chars() {
            f.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    #[test]
    fn url_field_wins_and_password_is_never_saved() {
        let mut f = ConnectForm::new(Vec::new());
        type_str(&mut f, "mysql://root:secret@127.0.0.1/shop");
        match f.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            ConnectEvent::Connect { spec, .. } => {
                assert_eq!(spec.backend, Backend::MySql);
                assert_eq!(spec.password.as_deref(), Some("secret"));
                assert!(!ConnectForm::url_for_saving(&spec).contains("secret"));
            }
            _ => panic!("expected connect"),
        }
    }

    /// Terminals deliver a paste as one event, not as keys, so the form has to take it itself.
    #[test]
    fn a_paste_goes_into_the_focused_field_only() {
        let mut f = ConnectForm::new(Vec::new());
        f.handle_paste("sqlite::memory:\n");
        assert_eq!(f.url.value(), "sqlite::memory:");
        f.field = 0;
        f.handle_paste("ignored by the type switch");
        f.field = FIELDS.iter().position(|x| *x == Field::Password).unwrap();
        f.handle_paste("s3cret\r");
        assert_eq!(f.password.value(), "s3cret");
        assert_eq!(f.url.value(), "sqlite::memory:");
        assert!(f.host.is_empty() && f.name.is_empty());
    }

    /// In the saved list, letters are commands (`d` deletes); pasted text must not reach anything.
    #[test]
    fn a_paste_over_the_saved_list_changes_nothing() {
        let mut f = ConnectForm::new(vec![("local".into(), SavedConnection { url: "sqlite::memory:".into(), ..Default::default() })]);
        f.handle_paste("dn");
        assert!(f.in_list && f.url.is_empty() && f.saved.len() == 1);
    }

    /// There is no separate "save" switch any more: a name is what saves a connection.
    #[test]
    fn a_name_saves_the_connection_and_no_name_does_not() {
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        let mut f = ConnectForm::new(Vec::new());
        type_str(&mut f, "sqlite::memory:");
        match f.handle_key(key(KeyCode::Enter)) {
            ConnectEvent::Connect { save_as, .. } => assert_eq!(save_as, None),
            _ => panic!("expected connect"),
        }
        f.field = FIELDS.iter().position(|x| *x == Field::Name).unwrap();
        type_str(&mut f, "scratch");
        match f.handle_key(key(KeyCode::Enter)) {
            ConnectEvent::Connect { save_as, name, .. } => {
                assert_eq!(save_as.as_deref(), Some("scratch"));
                assert_eq!(name, "scratch");
            }
            _ => panic!("expected connect"),
        }
    }

    #[test]
    fn escape_in_the_form_goes_back_to_the_saved_list_before_closing() {
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        let mut f = ConnectForm::new(vec![("local".into(), SavedConnection { url: "sqlite::memory:".into(), ..Default::default() })]);
        f.handle_key(key(KeyCode::Char('n')));
        assert!(matches!(f.handle_key(key(KeyCode::Esc)), ConnectEvent::None));
        assert!(matches!(f.handle_key(key(KeyCode::Esc)), ConnectEvent::Close));
    }

    #[test]
    fn sqlite_requires_a_file() {
        let mut f = ConnectForm::new(Vec::new());
        f.field = 0;
        f.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(f.backend, Backend::Sqlite);
        assert!(f.build_spec().is_err());
    }

    #[test]
    fn json_viewer_colors_keys_differently_from_values() {
        let t = Theme::default();
        let spans = json_spans(r#"  "id": "x","#, &t);
        let key = spans.iter().find(|s| s.content == "\"id\"").unwrap();
        let val = spans.iter().find(|s| s.content == "\"x\"").unwrap();
        assert_ne!(key.style.fg, val.style.fg);
    }
}
