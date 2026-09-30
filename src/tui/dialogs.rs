use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
use crate::theme::Theme;

pub fn centered(screen: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(screen.width.saturating_sub(2));
    let h = height.min(screen.height.saturating_sub(2));
    Rect { x: screen.x + (screen.width - w) / 2, y: screen.y + (screen.height - h) / 2, width: w, height: h }
}

pub fn frame(area: Rect, buf: &mut Buffer, theme: &Theme, title: &str, accent: ratatui::style::Color) -> Rect {
    Clear.render(area, buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .title(Span::styled(format!(" {title} "), Style::default().fg(accent).add_modifier(Modifier::BOLD)))
        .style(Style::default().bg(theme.surface).fg(theme.fg));
    let inner = block.inner(area);
    block.render(area, buf);
    inner
}

fn button(buf: &mut Buffer, x: u16, y: u16, label: &str, key: &str, style: Style, theme: &Theme) -> u16 {
    let text = format!(" {label} ");
    buf.set_string(x, y, &text, style);
    let kx = x + text.width() as u16;
    buf.set_string(kx, y, format!(" {key}"), Style::default().fg(theme.muted));
    kx + key.width() as u16 + 3
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

    pub fn render(&self, screen: Rect, buf: &mut Buffer, theme: &Theme) {
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
        let x = button(buf, inner.x + 1, y, &self.yes, "⏎/y", yes_style, theme);
        button(buf, x, y, "Cancel", "esc", Style::default().bg(theme.highlight).fg(theme.fg), theme);
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

    pub fn render(&mut self, screen: Rect, buf: &mut Buffer, theme: &Theme) -> Option<(u16, u16)> {
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
        self.input.render(field, buf, theme, true)
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

    pub fn render(&self, screen: Rect, buf: &mut Buffer, theme: &Theme) {
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

pub struct HelpView {
    pub scroll: usize,
}

pub const HELP: &[(&str, &[(&str, &str)])] = &[
    ("Global", &[
        ("Ctrl+P", "Command palette"),
        ("F1 / ?", "This help"),
        ("Ctrl+O", "Connections"),
        ("Ctrl+T / Ctrl+W", "New query tab / close tab"),
        ("Alt+←/→ · Ctrl+PgUp/PgDn", "Previous / next tab"),
        ("Alt+1…9", "Jump to tab"),
        ("F6 / Shift+F6", "Cycle focus: explorer · editor · results"),
        ("Alt+0", "Focus explorer"),
        ("Ctrl+B", "Toggle explorer"),
        ("Ctrl+G", "Go to table (fuzzy)"),
        ("Ctrl+Y", "Switch theme (live preview)"),
        ("Ctrl+R", "Query history"),
        ("Ctrl+Q", "Quit"),
    ]),
    ("Editor", &[
        ("Ctrl+Enter · Ctrl+E · Alt+Enter", "Run statement under cursor / selection"),
        ("F5 · Ctrl+Shift+Enter", "Run everything in the editor"),
        ("Esc · Ctrl+C (while running)", "Cancel query"),
        ("Ctrl+Space", "Completion (also as you type)"),
        ("F7 / Shift+F7", "Explain / explain analyze"),
        ("Alt+F", "Format SQL"),
        ("Ctrl+S", "Save query as favorite / to file"),
        ("Ctrl+/", "Toggle comment"),
        ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
        ("Ctrl+↑/↓", "Resize editor / results split"),
    ]),
    ("Results grid", &[
        ("hjkl · arrows", "Move"),
        ("Shift+move · v · V", "Select cells / rows"),
        ("Enter", "View cell / row"),
        ("y / Y", "Copy cells / rows"),
        ("/ · n · N", "Search in results"),
        ("< > =", "Narrow / widen / auto-fit column"),
        ("s", "Sort by column (table view)"),
        ("[ ]", "Previous / next result set"),
        ("m", "Messages log"),
        ("Ctrl+X", "Export results to file"),
    ]),
    ("Table view", &[
        ("f", "Filter (WHERE clause)"),
        ("F", "Filter by current cell value"),
        ("e / F2", "Edit cell"),
        ("o", "Insert row"),
        ("D / Delete", "Mark rows for deletion"),
        ("Ctrl+S", "Review & apply pending changes"),
        ("u", "Discard pending changes"),
        ("r / F5", "Reload"),
    ]),
    ("Explorer", &[
        ("Enter · Space · ←/→", "Open / expand / collapse"),
        ("/", "Filter tree"),
        ("s", "Structure of table"),
        ("i", "Insert name into editor"),
        ("g s|i|u|d|c|x|n", "Generate SELECT/INSERT/UPDATE/DELETE/CREATE/DROP/COUNT"),
        ("r", "Refresh"),
        ("n", "New connection"),
    ]),
];

impl HelpView {
    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<()> {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.scroll += 1,
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown => self.scroll += 10,
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            _ => return DialogResult::Close,
        }
        DialogResult::None
    }

    pub fn render(&mut self, screen: Rect, buf: &mut Buffer, theme: &Theme) {
        let area = centered(screen, 96, 40);
        let inner = frame(area, buf, theme, "Keyboard shortcuts", theme.border_focus);
        let mut lines: Vec<Line> = Vec::new();
        for (section, keys) in HELP {
            lines.push(Line::from(Span::styled(*section, Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))));
            for (k, d) in *keys {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {k:<34}"), Style::default().fg(theme.accent2)),
                    Span::styled(*d, Style::default().fg(theme.fg)),
                ]));
            }
            lines.push(Line::from(""));
        }
        let max = lines.len().saturating_sub(inner.height as usize);
        self.scroll = self.scroll.min(max);
        Paragraph::new(lines).scroll((self.scroll as u16, 0)).render(Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner }, buf);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Name,
    Url,
    Backend,
    Host,
    Port,
    User,
    Password,
    Database,
    Ssl,
    ReadOnly,
    Save,
}

const FIELDS: [Field; 11] = [
    Field::Name,
    Field::Url,
    Field::Backend,
    Field::Host,
    Field::Port,
    Field::User,
    Field::Password,
    Field::Database,
    Field::Ssl,
    Field::ReadOnly,
    Field::Save,
];

pub enum ConnectEvent {
    None,
    Close,
    /// Connect with this spec; `save_as` persists it in the config.
    Connect { spec: Box<ConnSpec>, save_as: Option<String>, name: String },
    Delete(String),
}

pub struct ConnectForm {
    pub saved: Vec<(String, SavedConnection)>,
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
    save: bool,
    pub error: Option<String>,
    pub busy: bool,
}

impl ConnectForm {
    pub fn new(saved: Vec<(String, SavedConnection)>) -> Self {
        let in_list = !saved.is_empty();
        ConnectForm {
            saved,
            list_selected: 0,
            in_list,
            field: 1,
            name: Input::new("").with_placeholder("optional — saves the connection"),
            url: Input::new("").with_placeholder("postgres://user@host/db · mysql://… · sqlite:file.db (overrides fields)"),
            backend: Backend::Postgres,
            host: Input::new("").with_placeholder("localhost"),
            port: Input::new("").with_placeholder("default"),
            user: Input::new("").with_placeholder("current user"),
            password: Input::new("").masked().with_placeholder("prompted if needed"),
            database: Input::new(""),
            ssl: SslMode::Prefer,
            readonly: false,
            save: true,
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
        let mut params = Vec::new();
        if spec.backend != Backend::Sqlite && spec.ssl_mode != SslMode::Prefer {
            params.push(format!("sslmode={}", match spec.ssl_mode {
                SslMode::Disable => "disable",
                SslMode::Prefer => "prefer",
                SslMode::Require => "require",
                SslMode::VerifyCa => "verify-ca",
                SslMode::VerifyFull => "verify-full",
            }));
        }
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }
        url
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ConnectEvent {
        self.error = None;
        if key.code == KeyCode::Esc {
            return ConnectEvent::Close;
        }
        if self.in_list {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => self.list_selected = (self.list_selected + 1).min(self.saved.len().saturating_sub(1)),
                KeyCode::Up | KeyCode::Char('k') => self.list_selected = self.list_selected.saturating_sub(1),
                KeyCode::Tab | KeyCode::Right | KeyCode::Char('n') => self.in_list = false,
                KeyCode::Char('d') | KeyCode::Delete => {
                    if let Some((name, _)) = self.saved.get(self.list_selected) {
                        return ConnectEvent::Delete(name.clone());
                    }
                }
                KeyCode::Enter => {
                    if let Some((name, saved)) = self.saved.get(self.list_selected) {
                        return match ConnSpec::parse(&saved.url) {
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
                        };
                    }
                }
                _ => {}
            }
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
                if self.field == 0 && !self.saved.is_empty() {
                    self.in_list = true;
                } else {
                    self.field = (self.field + fields.len() - 1) % fields.len();
                }
                return ConnectEvent::None;
            }
            KeyCode::Enter if !matches!(f, Field::Backend | Field::Ssl | Field::ReadOnly | Field::Save) || ctrl => {
                return self.submit();
            }
            _ => {}
        }
        match f {
            Field::Backend => {
                let order = [Backend::Postgres, Backend::MySql, Backend::Sqlite];
                let i = order.iter().position(|b| *b == self.backend).unwrap_or(0);
                match key.code {
                    KeyCode::Left | KeyCode::Char('h') => self.backend = order[(i + 2) % 3],
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Enter => self.backend = order[(i + 1) % 3],
                    _ => {}
                }
            }
            Field::Ssl => {
                let order = [SslMode::Disable, SslMode::Prefer, SslMode::Require, SslMode::VerifyCa, SslMode::VerifyFull];
                let i = order.iter().position(|m| *m == self.ssl).unwrap_or(1);
                match key.code {
                    KeyCode::Left | KeyCode::Char('h') => self.ssl = order[(i + 4) % 5],
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Enter => self.ssl = order[(i + 1) % 5],
                    _ => {}
                }
            }
            Field::ReadOnly => {
                if matches!(key.code, KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Left | KeyCode::Right) {
                    self.readonly = !self.readonly;
                }
            }
            Field::Save => {
                if matches!(key.code, KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right) {
                    self.save = !self.save;
                } else if key.code == KeyCode::Enter {
                    return self.submit();
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

    fn submit(&mut self) -> ConnectEvent {
        match self.build_spec() {
            Ok(spec) => {
                let name = self.name.value().trim().to_string();
                let label = if name.is_empty() { spec.label() } else { name.clone() };
                let save_as = (self.save && !name.is_empty()).then_some(name);
                ConnectEvent::Connect { spec: Box::new(spec), save_as, name: label }
            }
            Err(e) => {
                self.error = Some(e);
                ConnectEvent::None
            }
        }
    }

    pub fn render(&mut self, screen: Rect, buf: &mut Buffer, theme: &Theme) -> Option<(u16, u16)> {
        let area = centered(screen, 100, 24);
        let inner = frame(area, buf, theme, "Connections", theme.border_focus);
        let list_w = if self.saved.is_empty() { 0 } else { 30.min(inner.width / 3) };
        let mut cursor = None;
        if list_w > 0 {
            buf.set_string(inner.x + 1, inner.y, "Saved", Style::default().fg(theme.muted).add_modifier(Modifier::BOLD));
            for (i, (name, c)) in self.saved.iter().enumerate().take(inner.height.saturating_sub(3) as usize) {
                let y = inner.y + 1 + i as u16;
                let sel = i == self.list_selected;
                let style = if sel && self.in_list {
                    Style::default().bg(theme.selection).fg(theme.fg).add_modifier(Modifier::BOLD)
                } else if sel {
                    Style::default().bg(theme.highlight).fg(theme.fg)
                } else {
                    Style::default().fg(theme.fg)
                };
                let backend = ConnSpec::parse(&c.url).map(|s| match s.backend {
                    Backend::Postgres => "pg",
                    Backend::MySql => "my",
                    Backend::Sqlite => "sq",
                }).unwrap_or("??");
                buf.set_style(Rect { x: inner.x, y, width: list_w, height: 1 }, style);
                buf.set_string(inner.x + 1, y, backend, Style::default().fg(theme.accent2));
                buf.set_stringn(inner.x + 4, y, name, list_w.saturating_sub(6) as usize, style);
                if c.readonly {
                    buf.set_string(inner.x + list_w - 2, y, "ʀ", Style::default().fg(theme.info));
                }
            }
            for y in inner.y..inner.y + inner.height {
                buf.set_string(inner.x + list_w, y, "│", Style::default().fg(theme.border));
            }
        }
        let fx = inner.x + list_w + 2;
        let fw = inner.width.saturating_sub(list_w + 3);
        let label_w = 11u16;
        buf.set_string(fx, inner.y, "New connection", Style::default().fg(theme.muted).add_modifier(Modifier::BOLD));
        let fields = self.visible_fields();
        let focused_field = (!self.in_list).then(|| fields[self.field.min(fields.len() - 1)]);
        let mut y = inner.y + 1;
        for f in fields {
            let label = match f {
                Field::Name => "Name",
                Field::Url => "URL",
                Field::Backend => "Type",
                Field::Host => "Host",
                Field::Port => "Port",
                Field::User => "User",
                Field::Password => "Password",
                Field::Database => if self.backend == Backend::Sqlite { "File" } else { "Database" },
                Field::Ssl => "SSL",
                Field::ReadOnly => "Read-only",
                Field::Save => "Save",
            };
            let focused = focused_field == Some(f);
            let lstyle = if focused { Style::default().fg(theme.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme.muted) };
            buf.set_string(fx, y, format!("{}{label}", if focused { "▸ " } else { "  " }), lstyle);
            let vx = fx + label_w + 2;
            let vw = fw.saturating_sub(label_w + 3);
            let varea = Rect { x: vx, y, width: vw, height: 1 };
            if focused {
                buf.set_style(varea, Style::default().bg(theme.highlight));
            }
            let choice = |buf: &mut Buffer, opts: &[&str], cur: usize| {
                let mut x = vx;
                for (i, o) in opts.iter().enumerate() {
                    let st = if i == cur {
                        Style::default().bg(theme.accent).fg(theme.bg).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.muted)
                    };
                    let t = format!(" {o} ");
                    buf.set_string(x, y, &t, st);
                    x += t.width() as u16 + 1;
                }
            };
            match f {
                Field::Backend => choice(buf, &["PostgreSQL", "MySQL/MariaDB", "SQLite"], match self.backend {
                    Backend::Postgres => 0,
                    Backend::MySql => 1,
                    Backend::Sqlite => 2,
                }),
                Field::Ssl => choice(buf, &["disable", "prefer", "require", "verify-ca", "verify-full"], match self.ssl {
                    SslMode::Disable => 0,
                    SslMode::Prefer => 1,
                    SslMode::Require => 2,
                    SslMode::VerifyCa => 3,
                    SslMode::VerifyFull => 4,
                }),
                Field::ReadOnly => choice(buf, &["off", "on"], self.readonly as usize),
                Field::Save => choice(buf, &["no", "yes"], self.save as usize),
                other => {
                    if let Some(input) = self.input_mut(other) {
                        let c = input.render(varea, buf, theme, focused);
                        if focused {
                            cursor = c;
                        }
                    }
                }
            }
            y += 1;
        }
        let msg_y = inner.y + inner.height - 2;
        if let Some(e) = &self.error {
            buf.set_stringn(fx, msg_y, format!("✗ {e}"), fw as usize, Style::default().fg(theme.error));
        } else if self.busy {
            buf.set_string(fx, msg_y, "Connecting…", Style::default().fg(theme.info));
        }
        let hint = if self.in_list {
            "⏎ connect · d delete · Tab new connection · Esc close"
        } else {
            "Tab/↑↓ fields · ←→ choose · ⏎ connect · Esc close"
        };
        buf.set_stringn(fx, inner.y + inner.height - 1, hint, fw as usize, Style::default().fg(theme.muted));
        cursor
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

    #[test]
    fn sqlite_requires_a_file() {
        let mut f = ConnectForm::new(Vec::new());
        f.field = 2;
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
