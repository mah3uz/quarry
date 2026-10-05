use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event as CEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use tokio::runtime::Handle;

use super::dialogs::{Confirm, ConnectEvent, ConnectForm, DialogResult, HelpRow, HelpView, Prompt, TextView};
use super::keymap::{Action, Keymap};
use super::palette::{Item, Palette, PaletteEvent};
use super::sidebar::{self, Script, Sidebar};
use super::tabs::*;
use super::widgets::editor::{EditorEvent, VimMode};
use super::widgets::grid::{CopyKind, GridEvent, GridState};
use super::widgets::input::{Input, InputEvent};
use super::worker::{AppEvent, AppSender, ConnId, ConnectError, Connected, Reply, Request, Setup, Tag, Worker};
use super::Event;
use crate::cli::Opened;
use crate::complete::{CompleteOptions, Completer, Extras, KeywordCasing, Suggestion};
use crate::config::{Config, SavedConnection};
use crate::conn::ConnSpec;
use crate::db::{Backend, Catalog, Connection, DbError, ExecEvent, ServerInfo, Value, qualified, quote_ident};
use crate::output::{self, OutputOptions, TableFormat};
use crate::special;
use crate::sql::{classify, split};
use crate::theme::{ColorDepth, Theme};

pub struct ConnEntry {
    pub id: ConnId,
    pub name: String,
    pub spec: ConnSpec,
    pub main: Worker,
    pub meta_worker: Option<Worker>,
    pub info: ServerInfo,
    pub in_tx: bool,
    pub readonly: bool,
    pub catalog: Option<Arc<Catalog>>,
    pub completer: Option<Arc<Completer>>,
    pub _tunnel: Option<crate::conn::ssh::Tunnel>,
    pub loading_catalog: bool,
    /// From the saved connection's `color`, e.g. red for production.
    pub color: Option<Color>,
    /// Databases whose tables were requested for completion (MySQL loads only the current one up front).
    pub requested_schemas: std::collections::HashSet<String>,
    /// The schema a scoped tab last put first on the PostgreSQL search_path (reset for other tabs).
    pub session_schema: Option<String>,
    /// Completers that resolve unqualified names in a tab's database or schema, by scope name.
    pub scoped_completers: HashMap<String, Arc<Completer>>,
}

impl ConnEntry {
    pub fn meta(&self) -> &Worker {
        self.meta_worker.as_ref().unwrap_or(&self.main)
    }

    pub fn backend(&self) -> Backend {
        self.spec.backend
    }

    pub fn short_label(&self) -> String {
        let db = self.info.database.clone().or(self.spec.database.clone()).unwrap_or_default();
        match self.spec.backend {
            Backend::Sqlite => self.name.clone(),
            _ if db.is_empty() => self.name.clone(),
            _ => format!("{} ▸ {db}", self.name),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    Split,
    Sidebar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Main,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

pub struct Toast {
    pub text: String,
    pub level: Level,
    pub at: Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    NewQuery,
    CloseTab,
    CloseTabAt(usize),
    Commands,
    ToggleTransparent,
    NextTab,
    PrevTab,
    ToggleSidebar,
    FocusSidebar,
    FocusEditor,
    FocusResults,
    RunStatement,
    RunAll,
    Cancel,
    Explain(bool),
    FormatSql,
    Commit,
    Rollback,
    Activity,
    History,
    Themes,
    Connections,
    GoToTable,
    Refresh,
    Export,
    Copy(TableFormat),
    SaveFavorite,
    Favorites,
    OpenFavorite(String),
    OpenTable(ConnId, String, String),
    Structure,
    ToggleReadonly,
    SaveFile,
    OpenFile,
    Help,
    Quit,
    ForceQuit,
    RunConfirmed(u64, Vec<String>),
    ApplyEdits(u64),
    AskLlm,
    Kill(u64, String),
    DropFavorite(String),
    SwitchDatabase(ConnId, String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PromptPurpose {
    SaveFavorite,
    EditCell { tab: u64, row: usize, col: usize },
    Filter { tab: u64 },
    ExportPath,
    SaveFile,
    OpenFile,
    Password { conn: ConnId },
    GridSearch { tab: u64 },
    Llm { tab: u64 },
}

#[allow(clippy::large_enum_variant)]
pub enum Overlay {
    Commands(Palette<Command>),
    Themes(Palette<String>, Box<Theme>),
    Help(HelpView),
    Confirm(Confirm<Command>),
    Prompt(Prompt<PromptPurpose>),
    Text(TextView),
    Connect(ConnectForm),
}

pub struct CompletionPopup {
    pub items: Vec<Suggestion>,
    pub selected: usize,
    pub offset: usize,
    pub replace_start: usize,
}

/// Areas recorded during draw for mouse hit-testing.
#[derive(Default, Clone)]
pub struct Areas {
    pub header_tabs: Vec<(Rect, usize)>,
    pub sidebar: Rect,
    pub main: Rect,
    pub editor: Rect,
    pub result_tabs: Vec<(Rect, usize)>,
    pub grid: Rect,
    pub struct_tabs: Vec<(Rect, usize)>,
    /// Anything clickable that runs a command: tab close, new tab, run, status-bar pills.
    pub buttons: Vec<(Rect, Command)>,
    pub modal: super::dialogs::ModalLayout,
    /// The editor's bottom border and the results' top border: drag to resize the split.
    pub split: Rect,
    /// Rows of the History or Explain list.
    pub list: Rect,
    /// Rows of the completion popup.
    pub completion: Rect,
}

pub struct App {
    pub rt: Handle,
    pub config: Config,
    pub theme: Theme,
    pub depth: ColorDepth,
    pub tx: AppSender,
    pub conns: Vec<Option<ConnEntry>>,
    pub sidebar: Sidebar,
    pub sidebar_visible: bool,
    pub sidebar_width: u16,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub focus: Focus,
    pub overlay: Option<Overlay>,
    pub toasts: Vec<Toast>,
    pub completion: Option<CompletionPopup>,
    pub areas: Areas,
    pub spinner: usize,
    pub favorites: special::favorites::Favorites,
    pub keymap: Keymap,
    quit: bool,
    dragging: Option<Drag>,
    next_id: u64,
    pending_connects: HashMap<ConnId, (String, Box<ConnSpec>, Option<String>)>,
    /// Connections opened for a query console on another PostgreSQL database; they get a tab when ready.
    pending_consoles: std::collections::HashSet<String>,
    pub max_rows: usize,
}

use crate::theme::SPINNER;

impl App {
    pub fn new(rt: Handle, config: Config, tx: AppSender, overrides: super::Overrides) -> App {
        let depth = if overrides.no_color { ColorDepth::None } else { ColorDepth::detect() };
        let theme_name = overrides.theme.or_else(|| load_ui_state().theme).unwrap_or_else(|| config.main.theme.clone());
        let theme = crate::theme::load(&theme_name, &config.themes_dir()).unwrap_or_default();
        let favorites = special::favorites::Favorites::load(config.favorites_path()).unwrap_or_default();
        let mut warnings = config.warnings.clone();
        let (keymap, key_warnings) = Keymap::new(&config.keys);
        warnings.extend(key_warnings);
        let mut app = App {
            keymap,
            rt,
            theme: theme.adapted(depth),
            depth,
            tx,
            conns: Vec::new(),
            sidebar: Sidebar::default(),
            sidebar_visible: true,
            sidebar_width: 34,
            dragging: None,
            tabs: Vec::new(),
            active: 0,
            focus: Focus::Main,
            overlay: None,
            toasts: Vec::new(),
            completion: None,
            areas: Areas::default(),
            spinner: 0,
            favorites,
            quit: false,
            next_id: 1,
            pending_connects: HashMap::new(),
            pending_consoles: Default::default(),
            max_rows: 200_000,
            config,
        };
        for w in warnings {
            app.toast(Level::Warning, w);
        }
        app
    }

    /// The editor's vim mode while a query editor has focus.
    pub fn vim_mode(&self) -> Option<VimMode> {
        match self.active_tab().map(|t| &t.kind) {
            Some(TabKind::Query(q)) if self.editor_focused() && self.overlay.is_none() => q.editor.vim_mode(),
            _ => None,
        }
    }

    pub fn spinner(&self) -> &'static str {
        SPINNER[self.spinner % SPINNER.len()]
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn is_animating(&self) -> bool {
        self.tabs.iter().any(|t| t.is_busy())
            || !self.toasts.is_empty()
            || self.conns.iter().flatten().any(|c| c.loading_catalog)
            || !self.pending_connects.is_empty()
            || self.tabs.iter().any(|t| matches!(&t.kind, TabKind::Activity(a) if !a.paused))
    }

    pub fn shutdown(&mut self) {
        for c in self.conns.iter().flatten() {
            if c.main.is_busy() {
                c.main.cancel();
            }
        }
    }

    pub fn toast(&mut self, level: Level, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), level, at: Instant::now() });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn next_tab_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn conn(&self, id: ConnId) -> Option<&ConnEntry> {
        self.conns.get(id).and_then(|c| c.as_ref())
    }

    fn conn_mut(&mut self, id: ConnId) -> Option<&mut ConnEntry> {
        self.conns.get_mut(id).and_then(|c| c.as_mut())
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    fn active_conn_id(&self) -> Option<ConnId> {
        self.active_tab()
            .and_then(|t| t.conn)
            .or_else(|| self.sidebar.selected_conn())
            .or_else(|| self.conns.iter().position(|c| c.is_some()))
    }

    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    // ---------------------------------------------------------------- connections

    fn saved_color(&mut self, name: &str) -> Option<Color> {
        let text = self.config.connections.get(name)?.color.clone()?;
        match crate::theme::parse_color(&text) {
            Some(c) => Some(crate::theme::adapt(c, self.depth)),
            None => {
                self.toast(Level::Warning, format!("Connection '{name}': unknown color '{text}' (use \"#rrggbb\" or a name like red)"));
                None
            }
        }
    }

    pub fn adopt_connection(&mut self, opened: Opened, name: Option<String>, meta: Option<Connection>) {
        let id = self.conns.len();
        let name = name.unwrap_or_else(|| default_conn_name(&opened.spec));
        let needs_shared = opened.spec.backend == Backend::Sqlite;
        let spec = opened.spec.clone();
        let info = opened.conn.info().clone();
        let color = self.saved_color(&name);
        let main = Worker::spawn(&self.rt, id, opened.conn, self.tx.clone(), true);
        let entry = ConnEntry {
            id,
            name: name.clone(),
            readonly: spec.readonly,
            spec: spec.clone(),
            main,
            meta_worker: None,
            info: info.clone(),
            in_tx: false,
            catalog: None,
            completer: None,
            _tunnel: opened.tunnel,
            loading_catalog: true,
            color,
            requested_schemas: Default::default(),
            session_schema: None,
            scoped_completers: HashMap::new(),
        };
        self.conns.push(Some(entry));
        self.sidebar.set_connection(id, &name, &info.version, spec.backend, info.is_mariadb);
        self.sidebar.set_connection_color(id, color);
        if let Some(m) = meta {
            let w = Worker::spawn(&self.rt, id, m, self.tx.clone(), false);
            if let Some(e) = self.conn_mut(id) {
                e.meta_worker = Some(w);
            }
        } else if !needs_shared {
            let tx = self.tx.clone();
            let meta_spec = spec.clone();
            self.rt.spawn(async move {
                if let Ok(c) = Connection::connect(&meta_spec).await {
                    let _ = tx.send(Event::App(AppEvent::MetaReady { conn: id, connection: Box::new(c) }));
                }
            });
        }
        self.load_catalog(id);
        let has_query = self.tabs.iter().any(|t| matches!(t.kind, TabKind::Query(_)));
        if !has_query {
            self.new_query_tab(Some(id), None);
        } else if let Some(t) = self.tabs.get_mut(self.active)
            && t.conn.is_none() {
                t.conn = Some(id);
            }
        for t in &mut self.tabs {
            if t.conn.is_none() {
                t.conn = Some(id);
                if let TabKind::Query(q) = &mut t.kind {
                    q.editor.set_backend(spec.backend);
                }
            }
        }
        self.toast(Level::Success, format!("Connected to {name} · {}", info.version));
    }

    fn load_catalog(&mut self, id: ConnId) {
        if let Some(c) = self.conn_mut(id) {
            c.loading_catalog = true;
            c.meta().send(Tag::Catalog, Request::LoadCatalog);
        }
    }

    fn focus_connection(&mut self, id: ConnId) {
        self.sidebar.select_connection(id);
        match self.tabs.iter().position(|t| t.conn == Some(id) && matches!(t.kind, TabKind::Query(_))) {
            Some(i) => {
                self.active = i;
                self.focus = Focus::Main;
            }
            None => {
                self.new_query_tab(Some(id), None);
            }
        }
        self.completion = None;
    }

    /// A query tab for a database or schema from the explorer. Another PostgreSQL database needs its
    /// own connection, opened (or reused) under the name `connection/database`.
    fn open_console(&mut self, conn: ConnId, scope: Option<Scope>, database: Option<String>) {
        let Some(c) = self.conn(conn) else { return };
        if let Some(db) = database {
            let name = format!("{}/{db}", c.name);
            if let Some(id) = open_conn_named(&self.conns, &name, c.backend()) {
                self.new_query_tab(Some(id), None);
                return;
            }
            let mut spec = c.spec.clone();
            spec.database = Some(db);
            self.pending_consoles.insert(name.clone());
            self.toast(Level::Info, format!("Connecting to {name}…"));
            self.start_connect(name, spec, None);
            return;
        }
        let idx = self.new_query_tab(Some(conn), None);
        if let Some(scope) = scope {
            let n = self.tabs.iter().filter(|t| matches!(&t.kind, TabKind::Query(q) if q.scope.as_ref() == Some(&scope))).count();
            let title = if n == 0 { scope.name().to_string() } else { format!("{} ({})", scope.name(), n + 1) };
            let tab = &mut self.tabs[idx];
            tab.title = title;
            if let TabKind::Query(q) = &mut tab.kind {
                q.scope = Some(scope.clone());
            }
            if let Scope::Database(db) = &scope {
                self.request_schema(conn, db);
            }
        }
    }

    /// Loads a database's tables for completion if the catalog doesn't have them yet.
    fn request_schema(&mut self, conn: ConnId, schema: &str) {
        let Some(c) = self.conn_mut(conn) else { return };
        let Some(cat) = &c.catalog else { return };
        let unloaded = cat.schemas.iter().any(|s| s.name == schema && s.relations.is_empty());
        if unloaded && c.requested_schemas.insert(schema.to_string()) {
            c.meta().send(Tag::Sidebar, Request::ListRelations(schema.to_string()));
        }
    }

    /// What a tab must run first so the shared session is in the tab's database or schema; `None`
    /// when it already is. The connection's own tabs put back the default search_path.
    fn session_setup(&mut self, conn: ConnId, scope: Option<Scope>) -> Option<Setup> {
        let c = self.conn_mut(conn)?;
        match (c.backend(), scope) {
            (Backend::MySql, Some(Scope::Database(db))) if c.info.database.as_deref() != Some(db.as_str()) => {
                c.info.database = Some(db.clone());
                Some(Setup::Database(db))
            }
            (Backend::Postgres, Some(Scope::Schema(s))) if c.session_schema.as_deref() != Some(s.as_str()) => {
                let sql = format!("SET search_path TO {}, public", quote_ident(&s, Backend::Postgres));
                c.session_schema = Some(s);
                Some(Setup::Sql(sql))
            }
            (Backend::Postgres, None) if c.session_schema.is_some() => {
                c.session_schema = None;
                Some(Setup::Sql("RESET search_path".into()))
            }
            _ => None,
        }
    }

    /// `connection ▸ database` for a tab, showing the tab's own database or schema when it has one.
    pub fn tab_conn_label(&self, tab: &Tab) -> String {
        let Some(c) = tab.conn.and_then(|id| self.conn(id)) else { return String::new() };
        match &tab.kind {
            TabKind::Query(q) if q.scope.is_some() => format!("{} ▸ {}", c.name, q.scope.as_ref().map_or("", |s| s.name())),
            _ => c.short_label(),
        }
    }

    pub fn open_connection_manager(&mut self) {
        let saved: Vec<(String, SavedConnection)> =
            self.config.connections.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut form = ConnectForm::new(saved);
        form.open = self.conns.iter().flatten().map(|c| c.name.clone()).collect();
        self.overlay = Some(Overlay::Connect(form));
    }

    fn start_connect(&mut self, name: String, spec: ConnSpec, save_as: Option<String>) {
        let id = self.conns.len() + self.pending_connects.len();
        self.pending_connects.insert(id, (name.clone(), Box::new(spec.clone()), save_as.clone()));
        let tx = self.tx.clone();
        let pw_cmd = self.config.connections.get(&name).and_then(|c| c.password_command.clone());
        self.rt.spawn(async move {
            let mut spec_for_open = spec.clone();
            if spec_for_open.backend != Backend::Sqlite {
                crate::conn::passfile::apply_defaults(&mut spec_for_open);
            }
            let result = match crate::cli::open(spec_for_open, pw_cmd.as_deref(), false, false).await {
                Ok(opened) => {
                    let meta = if opened.spec.backend == Backend::Sqlite {
                        None
                    } else {
                        Connection::connect(&opened.spec).await.ok()
                    };
                    Ok(Box::new(Connected { opened, meta }))
                }
                Err(e) => {
                    let auth = e.downcast_ref::<DbError>().is_some_and(|d| d.kind == crate::db::ErrorKind::Auth);
                    let msg = format!("{e:#}");
                    Err(if auth { ConnectError::Auth(msg) } else { ConnectError::Other(msg) })
                }
            };
            let _ = tx.send(Event::App(AppEvent::Connected { conn: id, name, spec: Box::new(spec), save_as, result }));
        });
    }

    fn on_connected(&mut self, id: ConnId, name: String, spec: ConnSpec, save_as: Option<String>, result: Result<Box<Connected>, ConnectError>) {
        self.pending_connects.remove(&id);
        match result {
            Ok(c) => {
                let c = *c;
                if let Some(Overlay::Connect(_)) = self.overlay {
                    self.overlay = None;
                }
                if let Some(save) = save_as {
                    self.config.connections.insert(
                        save.clone(),
                        SavedConnection { url: ConnectForm::url_for_saving(&c.opened.spec), readonly: spec.readonly, ..Default::default() },
                    );
                    match self.config.save() {
                        Ok(()) => self.toast(Level::Info, format!("Saved connection '{save}'")),
                        Err(e) => self.toast(Level::Error, format!("Could not save connection: {e}")),
                    }
                }
                let console = self.pending_consoles.remove(&name);
                self.adopt_connection(c.opened, Some(name), c.meta);
                if console {
                    let id = self.conns.len() - 1;
                    self.new_query_tab(Some(id), None);
                }
            }
            Err(ConnectError::Auth(msg)) => {
                self.pending_connects.insert(id, (name, Box::new(spec), save_as));
                let mut input = Input::new("").masked();
                input.placeholder = "password".into();
                self.overlay = Some(Overlay::Prompt(Prompt {
                    title: "Password required".into(),
                    hint: msg,
                    input,
                    purpose: PromptPurpose::Password { conn: id },
                }));
            }
            Err(ConnectError::Other(msg)) => {
                if let Some(Overlay::Connect(form)) = &mut self.overlay {
                    form.busy = false;
                    form.error = Some(msg.clone());
                } else {
                    self.toast(Level::Error, msg);
                }
            }
        }
    }

    // ---------------------------------------------------------------- tabs

    pub fn new_query_tab(&mut self, conn: Option<ConnId>, text: Option<String>) -> usize {
        let conn = conn.or_else(|| self.active_conn_id());
        let backend = conn.and_then(|c| self.conn(c)).map(|c| c.backend()).unwrap_or(Backend::Postgres);
        // a new tab works where the current one does; on MySQL, at least in the connection's database
        let inherited = match self.active_tab() {
            Some(Tab { conn: c, kind: TabKind::Query(q), .. }) if *c == conn => q.scope.clone(),
            _ => None,
        };
        let scope = inherited.or_else(|| {
            let c = self.conn(conn?)?;
            (c.backend() == Backend::MySql).then(|| c.info.database.clone().map(Scope::Database)).flatten()
        });
        let mut q = QueryTab::new(backend);
        q.scope = scope;
        q.editor.set_vim(self.config.main.vi);
        if let Some(t) = text {
            q.editor.set_text(&t);
        }
        let n = self.tabs.iter().filter(|t| matches!(t.kind, TabKind::Query(_))).count() + 1;
        let id = self.next_tab_id();
        self.tabs.push(Tab { id, conn, title: format!("Query {n}"), kind: TabKind::Query(Box::new(q)) });
        self.active = self.tabs.len() - 1;
        self.focus = Focus::Main;
        self.active
    }

    fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        if let TabKind::Table(t) = &self.tabs[idx].kind
            && t.dirty() {
                let id = self.tabs[idx].id;
                self.overlay = Some(Overlay::Confirm(Confirm {
                    title: "Discard changes?".into(),
                    lines: vec![Line::from(format!("{} pending change(s) in {} will be lost.", t.pending_count(), t.name))],
                    yes: "Discard & close".into(),
                    danger: true,
                    on_yes: Command::ApplyEdits(u64::MAX - id),
                    scroll: 0,
                }));
                return;
            }
        if self.tabs[idx].is_busy()
            && let Some(c) = self.tabs[idx].conn.and_then(|c| self.conn(c)) {
                c.main.cancel();
            }
        self.tabs.remove(idx);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        self.completion = None;
    }

    fn open_table(&mut self, conn: ConnId, schema: String, name: String) {
        if let Some(i) = self.tabs.iter().position(|t| t.conn == Some(conn) && matches!(&t.kind, TabKind::Table(tt) if tt.schema == schema && tt.name == name)) {
            self.active = i;
            self.focus = Focus::Main;
            return;
        }
        let id = self.next_tab_id();
        let mut tt = TableTab::new(&schema, &name);
        tt.grid.null_text = self.config.main.null_string.clone();
        self.tabs.push(Tab { id, conn: Some(conn), title: name.clone(), kind: TabKind::Table(Box::new(tt)) });
        self.active = self.tabs.len() - 1;
        self.focus = Focus::Main;
        if let Some(c) = self.conn(conn) {
            c.meta().send(Tag::Tab(id, 0), Request::TableDetails { schema: Some(schema), table: name });
        }
    }

    fn open_structure(&mut self, conn: ConnId, schema: String, name: String) {
        let id = self.next_tab_id();
        self.tabs.push(Tab {
            id,
            conn: Some(conn),
            title: format!("{name} structure"),
            kind: TabKind::Structure(Box::new(StructureTab::new(&schema, &name))),
        });
        self.active = self.tabs.len() - 1;
        self.focus = Focus::Main;
        if let Some(c) = self.conn(conn) {
            c.meta().send(Tag::Tab(id, 0), Request::TableDetails { schema: Some(schema.clone()), table: name.clone() });
            c.meta().send(Tag::Tab(id, 1), Request::ObjectDdl { schema: Some(schema), name, kind: "table".into() });
        }
    }

    fn open_text_tab(&mut self, conn: Option<ConnId>, title: String, text: String, sql: bool) {
        let id = self.next_tab_id();
        self.tabs.push(Tab { id, conn, title, kind: TabKind::Text(Box::new(TextTab { text, scroll: 0, sql })) });
        self.active = self.tabs.len() - 1;
        self.focus = Focus::Main;
    }

    fn open_activity(&mut self) {
        let Some(conn) = self.active_conn_id() else {
            self.toast(Level::Warning, "No connection");
            return;
        };
        if self.conn(conn).is_some_and(|c| c.backend() == Backend::Sqlite) {
            self.toast(Level::Info, "SQLite has no server sessions to show");
            return;
        }
        let id = self.next_tab_id();
        let mut grid = GridState::new();
        grid.set_empty_message("Loading sessions…");
        self.tabs.push(Tab {
            id,
            conn: Some(conn),
            title: "Activity".into(),
            kind: TabKind::Activity(Box::new(ActivityTab {
                grid,
                paused: false,
                last_refresh: None,
                interval: Duration::from_secs(2),
                error: None,
                in_flight: false,
            })),
        });
        self.active = self.tabs.len() - 1;
        self.focus = Focus::Main;
    }

    fn open_history(&mut self) {
        let entries = read_history(&self.config.history_path());
        let id = self.next_tab_id();
        let mut h = HistoryTab {
            entries: entries.into_iter().map(|sql| HistoryEntry { sql }).collect(),
            filtered: Vec::new(),
            selected: 0,
            offset: 0,
            filter: Input::new("").with_placeholder("type to filter history…"),
        };
        h.refilter();
        let conn = self.active_conn_id();
        self.tabs.push(Tab { id, conn, title: "History".into(), kind: TabKind::History(Box::new(h)) });
        self.active = self.tabs.len() - 1;
        self.focus = Focus::Main;
    }

    fn open_explain(&mut self, analyze: bool) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let TabKind::Query(q) = &tab.kind else {
            self.toast(Level::Info, "Explain works from a query tab");
            return;
        };
        let Some(conn) = tab.conn else { return };
        let sql = selected_or_statement(q);
        if sql.trim().is_empty() {
            self.toast(Level::Warning, "Nothing to explain");
            return;
        }
        let id = self.next_tab_id();
        let title = if analyze { "Explain analyze" } else { "Explain" };
        self.tabs.push(Tab {
            id,
            conn: Some(conn),
            title: title.into(),
            kind: TabKind::Explain(Box::new(ExplainTab {
                sql: sql.clone(),
                analyze,
                plan: None,
                flat: Vec::new(),
                selected: 0,
                offset: 0,
                elapsed: None,
                error: None,
            })),
        });
        self.active = self.tabs.len() - 1;
        if let Some(c) = self.conn(conn) {
            c.main.send(Tag::Tab(id, 0), Request::Explain { sql, analyze });
        }
    }

    // ---------------------------------------------------------------- events

    pub fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(4));
        let mut refresh = Vec::new();
        for t in &mut self.tabs {
            if let (TabKind::Activity(a), Some(conn)) = (&mut t.kind, t.conn) {
                let due = a.last_refresh.is_none_or(|l| l.elapsed() >= a.interval);
                if !a.paused && !a.in_flight && due {
                    a.in_flight = true;
                    refresh.push((t.id, conn));
                }
            }
        }
        for (id, conn) in refresh {
            if let Some(c) = self.conn(conn) {
                c.meta().send(Tag::Tab(id, 0), Request::Activity);
            }
        }
    }

    pub fn handle(&mut self, ev: Event) {
        match ev {
            Event::Term(CEvent::Key(k)) if k.kind != KeyEventKind::Release => self.on_key(k),
            Event::Term(CEvent::Mouse(m)) => self.on_mouse(m),
            Event::Term(CEvent::Paste(text)) => self.on_paste(&text),
            Event::Term(_) => {}
            Event::App(ev) => self.on_app(ev),
        }
    }

    fn on_paste(&mut self, text: &str) {
        match &mut self.overlay {
            Some(Overlay::Commands(p)) => {
                p.handle_paste(text);
            }
            Some(Overlay::Themes(p, _)) => {
                if let PaletteEvent::Preview(name) = p.handle_paste(text)
                    && let Ok(t) = crate::theme::load(&name, &self.config.themes_dir()) {
                        self.theme = t.adapted(self.depth);
                    }
            }
            Some(Overlay::Help(h)) => h.handle_paste(text),
            Some(Overlay::Prompt(p)) => {
                p.input.handle_paste(text);
            }
            Some(Overlay::Connect(form)) => form.handle_paste(text),
            Some(Overlay::Confirm(_) | Overlay::Text(_)) => {}
            None => match self.focus {
                Focus::Sidebar => self.sidebar.handle_paste(text),
                Focus::Main => match self.tabs.get_mut(self.active).map(|t| &mut t.kind) {
                    Some(TabKind::Query(q)) => {
                        q.pane = Pane::Editor;
                        q.editor.handle_paste(text);
                    }
                    Some(TabKind::History(h)) => {
                        h.filter.handle_paste(text);
                        h.refilter();
                    }
                    _ => {}
                },
            },
        }
    }

    fn on_app(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Connected { conn, name, spec, save_as, result } => self.on_connected(conn, name, *spec, save_as, result),
            AppEvent::MetaReady { conn, connection } => {
                let w = Worker::spawn(&self.rt, conn, *connection, self.tx.clone(), false);
                if let Some(c) = self.conn_mut(conn) {
                    c.meta_worker = Some(w);
                }
            }
            AppEvent::State { conn, main, info, in_transaction } => {
                if let Some(c) = self.conn_mut(conn)
                    && main {
                        c.in_tx = in_transaction;
                        c.info = info;
                    }
            }
            AppEvent::Db { conn, tag, reply } => self.on_reply(conn, tag, reply),
            AppEvent::Llm { tab, result } => self.on_llm(tab, result),
        }
    }

    fn on_reply(&mut self, conn: ConnId, tag: Tag, reply: Reply) {
        match tag {
            Tag::Catalog => {
                if let Reply::Catalog(r) = reply {
                    self.on_catalog(conn, r);
                }
            }
            Tag::Sidebar => match reply {
                Reply::Relations(schema, Ok(rels)) => {
                    self.sidebar.set_relations(conn, &schema, &rels);
                    self.merge_relations(conn, &schema, rels);
                }
                Reply::Relations(_, Err(e)) => self.toast(Level::Error, e.to_string()),
                Reply::Ddl(Ok(text)) => {
                    let title = "Definition".to_string();
                    self.open_text_tab(Some(conn), title, text, true);
                }
                Reply::Ddl(Err(e)) => self.toast(Level::Error, e.to_string()),
                Reply::Done(Ok(())) => {
                    self.toast(Level::Success, "Database switched");
                    self.load_catalog(conn);
                }
                Reply::Done(Err(e)) => self.toast(Level::Error, e.to_string()),
                _ => {}
            },
            Tag::Silent | Tag::Palette => {
                if let Reply::Done(Err(e)) | Reply::Committed(Err(e)) = reply {
                    self.toast(Level::Error, e.to_string());
                } else if let Reply::Rows(Err(e), _) = reply {
                    self.toast(Level::Error, e.to_string());
                }
            }
            Tag::Tab(id, seq) => self.on_tab_reply(conn, id, seq, reply),
        }
    }

    fn make_completer(&self, cat: Arc<Catalog>) -> Completer {
        let casing = match self.config.main.keyword_casing.to_ascii_lowercase().as_str() {
            "upper" => KeywordCasing::Upper,
            "lower" => KeywordCasing::Lower,
            _ => KeywordCasing::Auto,
        };
        let backend = cat.backend;
        let extras = Extras {
            specials: special::registry()
                .iter()
                .filter(|s| s.backends.is_empty() || s.backends.contains(&backend))
                .flat_map(|s| s.names.iter().map(move |n| (n.to_string(), s.description.to_string())))
                .collect(),
            favorites: self.favorites.queries.keys().cloned().collect(),
            ..Default::default()
        };
        let options = CompleteOptions {
            keyword_casing: casing,
            smart: self.config.main.smart_completion,
            join_suggestions: self.config.main.join_suggestions,
            ..Default::default()
        };
        Completer::new(backend, cat, options, extras)
    }

    /// Relations loaded lazily (a MySQL database other than the current one) join the completer's catalog.
    fn merge_relations(&mut self, conn: ConnId, schema: &str, rels: Vec<crate::db::Relation>) {
        let Some(old) = self.conn(conn).and_then(|c| c.catalog.clone()) else { return };
        let mut cat = (*old).clone();
        match cat.schemas.iter_mut().find(|s| s.name == schema) {
            Some(s) if s.relations.iter().any(|r| !r.columns.is_empty()) => return,
            Some(s) => s.relations = rels,
            None => cat.schemas.push(crate::db::SchemaInfo { name: schema.to_string(), relations: rels, functions: Vec::new(), types: Vec::new() }),
        }
        let cat = Arc::new(cat);
        let completer = Arc::new(self.make_completer(cat.clone()));
        if let Some(c) = self.conn_mut(conn) {
            c.completer = Some(completer);
            c.catalog = Some(cat);
            c.scoped_completers.clear();
        }
        if self.editor_focused() && self.tabs.get(self.active).and_then(|t| t.conn) == Some(conn) {
            self.update_completion(false);
        }
    }

    /// `db.` before the cursor names a database whose tables aren't loaded yet: fetch them.
    fn request_schema_for_completion(&mut self, conn: ConnId, before_cursor: &str) {
        let Some(qualifier) = before_cursor
            .trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '$')
            .strip_suffix('.')
            .map(|q| q.rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '`' || c == '"')).next().unwrap_or(""))
            .map(|q| q.trim_matches(|c| c == '`' || c == '"').to_string())
        else {
            return;
        };
        let Some(c) = self.conn_mut(conn) else { return };
        let Some(cat) = &c.catalog else { return };
        let unloaded = cat.schemas.iter().any(|s| s.name == qualifier && s.relations.is_empty());
        if unloaded && c.requested_schemas.insert(qualifier.clone()) {
            c.meta().send(Tag::Sidebar, Request::ListRelations(qualifier));
        }
    }

    fn on_catalog(&mut self, conn: ConnId, r: Result<Catalog, DbError>) {
        let show_system = false;
        match r {
            Ok(cat) => {
                let cat = Arc::new(cat);
                self.sidebar.set_catalog(conn, &cat, show_system);
                let completer = Arc::new(self.make_completer(cat.clone()));
                if let Some(c) = self.conn_mut(conn) {
                    c.completer = Some(completer);
                    c.catalog = Some(cat);
                    c.loading_catalog = false;
                    c.requested_schemas.clear();
                    c.scoped_completers.clear();
                }
            }
            Err(e) => {
                if let Some(c) = self.conn_mut(conn) {
                    c.loading_catalog = false;
                }
                self.toast(Level::Error, format!("Loading schema failed: {e}"));
            }
        }
    }

    fn on_tab_reply(&mut self, conn: ConnId, id: u64, seq: u64, reply: Reply) {
        let Some(idx) = self.tab_index(id) else { return };
        let null = self.config.main.null_string.clone();
        let mut toast: Option<(Level, String)> = None;
        let mut refresh_catalog = false;
        let mut history: Vec<String> = Vec::new();
        let tab = &mut self.tabs[idx];
        match (&mut tab.kind, reply) {
            (TabKind::Query(q), reply) => match reply {
                Reply::StatementStart { index, .. } => {
                    if let Some(r) = &mut q.running {
                        r.current = index;
                    }
                }
                Reply::Event { index, event } => match event {
                    ExecEvent::Columns(cols) => {
                        let first = q.results.is_empty();
                        q.results.push(ResultMeta {
                            index,
                            sql: q.exec_sqls.get(index).cloned().unwrap_or_default(),
                            columns: cols.clone(),
                            summary: None,
                            elapsed: None,
                            truncated: false,
                            stash: if first { None } else { Some(Vec::new()) },
                            row_count: 0,
                        });
                        if first {
                            q.shown = 0;
                            q.grid.null_text = null;
                            q.grid.set_data(cols, Vec::new());
                        }
                    }
                    ExecEvent::Rows(rows) => {
                        let shown = q.shown;
                        if let Some((ri, r)) = q.results.iter_mut().enumerate().rev().find(|(_, r)| r.index == index) {
                            r.row_count += rows.len();
                            if ri == shown && r.stash.is_none() {
                                q.grid.push_rows(rows);
                            } else if let Some(st) = &mut r.stash {
                                st.extend(rows);
                            }
                        }
                    }
                    ExecEvent::Done(summary) => {
                        let status = summary.status.clone();
                        let has_result = q.results.iter().any(|r| r.index == index && r.summary.is_none());
                        if has_result {
                            if let Some(r) = q.results.iter_mut().rev().find(|r| r.index == index) {
                                r.summary = Some(summary);
                            }
                        } else {
                            let affected = summary.rows_affected.map(|n| format!(" · {n} row(s) affected")).unwrap_or_default();
                            q.log(MessageKind::Ok, format!("{}{affected}", status.unwrap_or_else(|| "OK".into())));
                        }
                    }
                    ExecEvent::Notice(n) => q.log(MessageKind::Notice, format!("{}: {}", n.severity, n.message)),
                },
                Reply::StatementDone { index, result, elapsed, truncated } => {
                    let sql = q.exec_sqls.get(index).cloned().unwrap_or_default();
                    for r in q.results.iter_mut().filter(|r| r.index == index) {
                        r.elapsed = Some(elapsed);
                        r.truncated = truncated;
                    }
                    match result {
                        Ok(()) => {
                            if truncated {
                                q.log(MessageKind::Info, format!("Result truncated to {} rows", self.max_rows));
                            }
                            if classify::changes_schema(&sql, q.editor_backend()) {
                                refresh_catalog = true;
                            }
                            history.push(sql);
                        }
                        Err(e) => {
                            let mut msg = format!("{} {e}", crate::icons::get().error);
                            if let Some(d) = &e.detail {
                                msg.push_str(&format!("\n  detail: {d}"));
                            }
                            if let Some(h) = &e.hint {
                                msg.push_str(&format!("\n  hint: {h}"));
                            }
                            q.log(MessageKind::Error, msg);
                            if let (Some(pos), Some(base)) = (e.position, q.exec_offsets().get(index).copied()) {
                                let byte = sql.char_indices().nth(pos.saturating_sub(1)).map(|(b, _)| b).unwrap_or(0);
                                q.editor.set_error_marker(Some(base + byte));
                            }
                            let level = if e.kind == crate::db::ErrorKind::Cancelled { Level::Warning } else { Level::Error };
                            toast = Some((level, e.message.clone()));
                        }
                    }
                }
                Reply::ScriptDone { elapsed, ok } => {
                    q.running = None;
                    q.last_elapsed = Some(elapsed);
                    if q.results.is_empty() || !ok && q.grid.row_count() == 0 {
                        q.shown = q.results.len();
                        if q.results.is_empty() {
                            q.grid.clear();
                        }
                    }
                    if ok {
                        q.log(MessageKind::Info, format!("Finished in {}", crate::repl::prompt::human_duration(elapsed)));
                    }
                }
                Reply::Titled(r) => {
                    q.running = None;
                    match r {
                        Ok(Some(items)) => {
                            q.reset_results();
                            let mut first = true;
                            for (i, t) in items.into_iter().enumerate() {
                                if let Some(text) = t.text {
                                    q.log(MessageKind::Info, text);
                                    continue;
                                }
                                let rows = t.result.rows;
                                q.results.push(ResultMeta {
                                    index: i,
                                    sql: t.title.unwrap_or_default(),
                                    columns: t.result.columns.clone(),
                                    summary: Some(t.result.summary),
                                    elapsed: None,
                                    truncated: false,
                                    row_count: rows.len(),
                                    stash: if first { None } else { Some(rows.clone()) },
                                });
                                if first {
                                    q.grid.set_data(t.result.columns, rows);
                                    first = false;
                                }
                            }
                            q.shown = 0;
                        }
                        Ok(None) => toast = Some((Level::Info, "That command is only available in the CLI".into())),
                        Err(e) => {
                            q.log(MessageKind::Error, format!("{} {e}", crate::icons::get().error));
                            toast = Some((Level::Error, e.message));
                        }
                    }
                }
                _ => {}
            },
            (TabKind::Table(t), reply) => match reply {
                Reply::Details(Ok(d)) => {
                    if t.order.is_none() {
                        let pk: Vec<usize> = d.columns.iter().enumerate().filter(|(_, c)| c.primary_key).map(|(i, _)| i).collect();
                        if pk.len() == 1 {
                            t.order = Some((pk[0], true));
                        }
                    }
                    t.details = Some(d);
                    let _ = seq;
                    self.fetch_table_page(idx, true);
                    return;
                }
                Reply::Details(Err(e)) => {
                    t.error = Some(e.to_string());
                    self.fetch_table_page(idx, true);
                    return;
                }
                Reply::Rows(r, elapsed) if seq == t.generation * 2 + 1 => {
                    if let Ok(rs) = r {
                        t.total = rs.rows.first().and_then(|row| row.first()).and_then(|v| match v {
                            Value::Int(i) => Some(*i as u64),
                            Value::UInt(u) => Some(*u),
                            Value::Text(s) => s.parse().ok(),
                            _ => None,
                        });
                    }
                    let _ = elapsed;
                }
                Reply::Rows(r, elapsed) if seq == t.generation * 2 => {
                    t.loading = false;
                    t.last_elapsed = Some(elapsed);
                    match r {
                        Ok(rs) => {
                            t.error = None;
                            let n = rs.rows.len();
                            if t.loaded == 0 {
                                t.grid.null_text = null;
                                t.grid.set_data(rs.columns, rs.rows);
                                if let Some((c, asc)) = t.order {
                                    t.grid.set_sort(c, asc);
                                }
                            } else {
                                t.grid.push_rows(rs.rows);
                            }
                            t.loaded += n;
                            t.has_more = n >= t.page_size;
                            if !t.has_more && t.filter.is_empty() {
                                t.total = Some(t.loaded as u64);
                            }
                        }
                        Err(e) => {
                            t.error = Some(e.to_string());
                            toast = Some((Level::Error, e.message));
                        }
                    }
                }
                Reply::Committed(r) => match r {
                    Ok(n) => {
                        toast = Some((Level::Success, format!("Applied changes · {n} row(s) affected")));
                        t.edits.clear();
                        t.deleted.clear();
                        t.inserted.clear();
                        t.original.clear();
                        t.grid.clear_marks();
                        self.fetch_table_page(idx, true);
                        if let Some((l, m)) = toast {
                            self.toast(l, m);
                        }
                        return;
                    }
                    Err(e) => toast = Some((Level::Error, format!("Changes rolled back: {e}"))),
                },
                _ => {}
            },
            (TabKind::Structure(s), reply) => match reply {
                Reply::Details(Ok(d)) => {
                    s.details = Some(d);
                    s.load_section();
                }
                Reply::Details(Err(e)) => s.error = Some(e.to_string()),
                Reply::Ddl(Ok(ddl)) => s.ddl = Some(ddl),
                Reply::Ddl(Err(e)) => s.ddl = Some(format!("-- {e}")),
                _ => {}
            },
            (TabKind::Activity(a), reply) => match reply {
                Reply::Rows(Ok(rs), _) => {
                    a.in_flight = false;
                    a.last_refresh = Some(Instant::now());
                    a.error = None;
                    a.grid.set_empty_message("No other sessions");
                    a.grid.set_data(rs.columns, rs.rows);
                }
                Reply::Rows(Err(e), _) => {
                    a.in_flight = false;
                    a.last_refresh = Some(Instant::now());
                    a.error = Some(e.to_string());
                }
                Reply::Done(r) => {
                    toast = Some(match r {
                        Ok(()) => (Level::Success, "Session terminated".into()),
                        Err(e) => (Level::Error, e.to_string()),
                    });
                    a.last_refresh = None;
                }
                _ => {}
            },
            (TabKind::Explain(x), Reply::Plan(r, elapsed)) => {
                x.elapsed = Some(elapsed);
                match r {
                    Ok(plan) => x.set_plan(plan),
                    Err(e) => x.error = Some(e.to_string()),
                }
            }
            _ => {}
        }
        if let Some((l, m)) = toast {
            self.toast(l, m);
        }
        if refresh_catalog && self.config.main.auto_refresh_catalog {
            self.load_catalog(conn);
        }
        if !history.is_empty() {
            append_history(&self.config.history_path(), &history);
        }
    }

    // ---------------------------------------------------------------- execution

    fn run_query(&mut self, all: bool) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let tab_id = tab.id;
        let Some(conn_id) = tab.conn else {
            self.toast(Level::Warning, "Not connected — press Ctrl+O to connect");
            return;
        };
        let TabKind::Query(q) = &mut tab.kind else { return };
        if q.running.is_some() {
            self.toast(Level::Warning, "A query is already running (Esc to cancel)");
            return;
        }
        let backend = q.editor_backend();
        let text = q.editor.text();
        let (sql, base) = if all {
            (text.clone(), 0)
        } else if let Some(sel) = q.editor.selected_text().filter(|s| !s.trim().is_empty()) {
            let base = text.find(&sel).unwrap_or(0);
            (sel, base)
        } else {
            let r = q.editor.current_statement_range();
            (text[r.clone()].to_string(), r.start)
        };
        if sql.trim().is_empty() {
            self.toast(Level::Info, "Nothing to run");
            return;
        }
        if let Some(parsed) = special::parse(sql.trim(), backend) {
            match parsed {
                Ok(special::Special::Llm { question }) => self.start_llm(tab_id, question),
                Ok(cmd) => {
                    q.running = Some(Running { started: Instant::now(), total: 1, current: 0 });
                    let scope = q.scope.clone();
                    if let Some(setup) = self.session_setup(conn_id, scope)
                        && let Some(c) = self.conn(conn_id)
                    {
                        // a special command has no script to fail with, so a failed switch shows as a toast
                        let (req, tag) = match setup {
                            Setup::Database(db) => (Request::ChangeDatabase(db), Tag::Silent),
                            Setup::Sql(sql) => (Request::Query(sql), Tag::Silent),
                        };
                        c.main.send(tag, req);
                    }
                    if let Some(c) = self.conn(conn_id) {
                        c.main.send(Tag::Tab(tab_id, 0), Request::Special(cmd));
                    }
                }
                Err(e) => self.toast(Level::Error, e),
            }
            return;
        }
        let stmts = split::split(&sql, backend, ";");
        if stmts.is_empty() {
            self.toast(Level::Info, "Only comments — nothing to run");
            return;
        }
        q.exec_base = base;
        q.exec_starts = stmts.iter().map(|s| base + s.start).collect();
        let texts: Vec<String> = stmts.iter().map(|s| s.text.clone()).collect();
        let Some(conn) = self.conn(conn_id) else { return };
        if conn.readonly
            && let Some(bad) = texts.iter().find(|s| !classify::is_read_only(s, backend)) {
                let short: String = bad.chars().take(60).collect();
                self.toast(Level::Error, format!("Read-only connection: refused `{short}`"));
                return;
            }
        let rules = self.config.main.destructive_warning.clone();
        let flagged: Vec<(String, String)> = texts
            .iter()
            .filter_map(|s| classify::destructive(s, backend, &rules).map(|d| (d.reason, s.clone())))
            .collect();
        if !flagged.is_empty() {
            let mut lines = vec![Line::from(Span::styled(
                "These statements can destroy data:",
                Style::default().fg(self.theme.warning).add_modifier(Modifier::BOLD),
            ))];
            for (reason, sql) in flagged.iter().take(8) {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(format!("• {reason}"), Style::default().fg(self.theme.error))));
                for l in sql.lines().take(6) {
                    let mut spans = vec![Span::raw("  ")];
                    spans.extend(highlight_spans(l, backend, &self.theme));
                    lines.push(Line::from(spans));
                }
            }
            if conn.in_tx {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("A transaction is open: you can still ROLLBACK afterwards.", Style::default().fg(self.theme.muted))));
            }
            self.overlay = Some(Overlay::Confirm(Confirm {
                title: "Confirm destructive statement".into(),
                lines,
                yes: "Run anyway".into(),
                danger: true,
                on_yes: Command::RunConfirmed(tab_id, texts),
                scroll: 0,
            }));
            return;
        }
        self.dispatch_script(tab_id, texts);
    }

    fn dispatch_script(&mut self, tab_id: u64, statements: Vec<String>) {
        let max_rows = self.max_rows;
        let Some(idx) = self.tab_index(tab_id) else { return };
        let conn_id = self.tabs[idx].conn;
        let TabKind::Query(q) = &mut self.tabs[idx].kind else { return };
        q.reset_results();
        q.editor.set_error_marker(None);
        q.running = Some(Running { started: Instant::now(), total: statements.len(), current: 0 });
        q.exec_sqls = statements.clone();
        let preview: String = statements.first().map(|s| s.lines().next().unwrap_or("").chars().take(80).collect()).unwrap_or_default();
        let more = if statements.len() > 1 { format!(" (+{} more)", statements.len() - 1) } else { String::new() };
        q.log(MessageKind::Info, format!("{} {preview}{more}", crate::icons::get().run));
        self.completion = None;
        let scope = q.scope.clone();
        let setup = conn_id.and_then(|id| self.session_setup(id, scope));
        if let Some(c) = conn_id.and_then(|c| self.conn(c)) {
            c.main.send(Tag::Tab(tab_id, 0), Request::Script { statements, keep_going: false, max_rows, setup });
        }
    }

    fn start_llm(&mut self, tab_id: u64, question: String) {
        let Some(idx) = self.tab_index(tab_id) else { return };
        let conn = self.tabs[idx].conn.and_then(|c| self.conn(c));
        let catalog = conn.and_then(|c| c.catalog.clone());
        let backend = conn.map(|c| c.backend()).unwrap_or(Backend::Postgres);
        let version = conn.map(|c| c.info.version.clone()).unwrap_or_default();
        let llm = self.config.llm.clone();
        if let TabKind::Query(q) = &mut self.tabs[idx].kind {
            q.running = Some(Running { started: Instant::now(), total: 1, current: 0 });
            q.asking = Some((crate::llm::describe(&llm), question.clone()));
            q.log(MessageKind::Info, format!("{} Asking {}: {question}", crate::icons::get().ask, crate::llm::describe(&llm)));
        }
        let tx = self.tx.clone();
        self.rt.spawn_blocking(move || {
            let req = crate::llm::Request { question: &question, backend, server_version: &version, catalog: catalog.as_deref() };
            let result = crate::llm::ask(&llm, &req);
            let _ = tx.send(Event::App(AppEvent::Llm { tab: tab_id, result }));
        });
    }

    fn on_llm(&mut self, tab: u64, result: Result<crate::llm::Answer, String>) {
        let Some(idx) = self.tab_index(tab) else { return };
        let TabKind::Query(q) = &mut self.tabs[idx].kind else { return };
        q.running = None;
        q.asking = None;
        match result {
            Ok(a) => {
                if !a.explanation.is_empty() {
                    q.log(MessageKind::Info, a.explanation.clone());
                }
                if a.sql.is_empty() {
                    self.toast(Level::Info, "The model replied without SQL — see Messages");
                    return;
                }
                let r = q.editor.current_statement_range();
                let text = q.editor.text();
                if text[r.clone()].trim_start().starts_with('\\') {
                    q.editor.replace_range(r, &a.sql);
                } else {
                    let end = text.len();
                    let sep = if text.trim().is_empty() { "" } else { "\n\n" };
                    q.editor.replace_range(end..end, &format!("{sep}{}", a.sql));
                }
                q.pane = Pane::Editor;
                self.toast(Level::Success, "SQL ready — review it, then Ctrl+Enter to run");
            }
            Err(e) => {
                q.log(MessageKind::Error, format!("{} {e}", crate::icons::get().error));
                self.toast(Level::Error, e);
            }
        }
    }

    fn cancel_active(&mut self) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return false };
        if !tab.is_busy() {
            return false;
        }
        if let Some(c) = tab.conn.and_then(|c| self.conn(c)) {
            c.main.cancel();
            if let Some(m) = &c.meta_worker
                && m.is_busy() {
                    m.cancel();
                }
        }
        self.toast(Level::Warning, "Cancelling…");
        true
    }

    fn fetch_table_page(&mut self, idx: usize, reset: bool) {
        let tab_id = self.tabs[idx].id;
        let Some(conn_id) = self.tabs[idx].conn else { return };
        let Some(backend) = self.conn(conn_id).map(|c| c.backend()) else { return };
        let TabKind::Table(t) = &mut self.tabs[idx].kind else { return };
        if reset {
            t.generation += 1;
            t.loaded = 0;
            t.has_more = true;
            t.total = None;
        }
        if !t.has_more {
            return;
        }
        t.loading = true;
        let name = qualified(Some(&t.schema), &t.name, backend);
        let where_ = if t.filter.trim().is_empty() { String::new() } else { format!(" WHERE {}", t.filter.trim()) };
        let order = t
            .order
            .and_then(|(c, asc)| {
                let col = t.grid.columns().get(c).map(|c| c.name.clone())
                    .or_else(|| t.details.as_ref().and_then(|d| d.columns.get(c)).map(|c| c.name.clone()))?;
                Some(format!(" ORDER BY {} {}", quote_ident(&col, backend), if asc { "ASC" } else { "DESC" }))
            })
            .unwrap_or_default();
        let sql = format!("SELECT * FROM {name}{where_}{order} LIMIT {} OFFSET {}", t.page_size, t.loaded);
        let generation = t.generation;
        let count_sql = format!("SELECT COUNT(*) FROM {name}{where_}");
        let first = t.loaded == 0;
        if let Some(c) = self.conn(conn_id) {
            c.meta().send(Tag::Tab(tab_id, generation * 2), Request::Query(sql));
            if first {
                c.meta().send(Tag::Tab(tab_id, generation * 2 + 1), Request::Query(count_sql));
            }
        }
    }

    // ---------------------------------------------------------------- keys

    fn on_key(&mut self, key: KeyEvent) {
        if self.overlay.is_some() {
            self.on_overlay_key(key);
            return;
        }
        if self.completion.is_some() && self.on_completion_key(key) {
            return;
        }
        if self.on_global_key(key) {
            return;
        }
        match self.focus {
            Focus::Sidebar => {
                if let Some(action) = self.sidebar.handle_key(key) {
                    self.on_sidebar_action(action);
                }
            }
            Focus::Main => self.on_main_key(key),
        }
    }

    fn on_global_key(&mut self, key: KeyEvent) -> bool {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if let KeyCode::Char(c @ '1'..='9') = key.code
            && alt
        {
            let i = (c as usize) - ('1' as usize);
            if i < self.tabs.len() {
                self.active = i;
                self.focus = Focus::Main;
                self.completion = None;
            }
            return true;
        }
        if key.code == KeyCode::BackTab && self.focus == Focus::Sidebar {
            self.cycle_focus(false);
            return true;
        }
        let in_editor = self.editor_focused();
        let typing = in_editor || self.sidebar.is_filtering() || self.table_filter_active();
        let cmd = match self.keymap.action(&key, typing, in_editor) {
            Some(Action::Commands) => Command::Commands,
            Some(Action::Help) => Command::Help,
            Some(Action::Connections) => Command::Connections,
            Some(Action::NewQuery) => Command::NewQuery,
            Some(Action::CloseTab) => Command::CloseTab,
            Some(Action::NextTab) => Command::NextTab,
            Some(Action::PrevTab) => Command::PrevTab,
            Some(Action::FocusExplorer) => Command::FocusSidebar,
            Some(Action::ToggleExplorer) => Command::ToggleSidebar,
            Some(Action::GoToTable) => Command::GoToTable,
            Some(Action::Themes) => Command::Themes,
            Some(Action::History) => Command::History,
            Some(Action::Quit) => Command::Quit,
            Some(Action::NextPane) => {
                self.cycle_focus(true);
                return true;
            }
            Some(Action::PrevPane) => {
                self.cycle_focus(false);
                return true;
            }
            _ => return false,
        };
        self.run_command(cmd);
        true
    }

    fn table_filter_active(&self) -> bool {
        matches!(self.active_tab().map(|t| &t.kind), Some(TabKind::Table(t)) if t.filter_input.is_some())
            || matches!(self.active_tab().map(|t| &t.kind), Some(TabKind::History(_)))
    }

    fn editor_focused(&self) -> bool {
        self.focus == Focus::Main
            && matches!(self.active_tab().map(|t| &t.kind), Some(TabKind::Query(q)) if q.pane == Pane::Editor)
    }

    fn cycle_focus(&mut self, forward: bool) {
        self.completion = None;
        let query = matches!(self.active_tab().map(|t| &t.kind), Some(TabKind::Query(_)));
        let pane = match self.tabs.get(self.active).map(|t| &t.kind) {
            Some(TabKind::Query(q)) => Some(q.pane),
            _ => None,
        };
        let order: Vec<(Focus, Option<Pane>)> = if query {
            vec![(Focus::Sidebar, None), (Focus::Main, Some(Pane::Editor)), (Focus::Main, Some(Pane::Results))]
        } else {
            vec![(Focus::Sidebar, None), (Focus::Main, None)]
        };
        let cur = order
            .iter()
            .position(|(f, p)| *f == self.focus && (p.is_none() || *p == pane))
            .unwrap_or(0);
        let mut next = if forward { (cur + 1) % order.len() } else { (cur + order.len() - 1) % order.len() };
        if !self.sidebar_visible && order[next].0 == Focus::Sidebar {
            next = if forward { (next + 1) % order.len() } else { (next + order.len() - 1) % order.len() };
        }
        let (f, p) = order[next];
        self.focus = f;
        if let (Some(p), Some(Tab { kind: TabKind::Query(q), .. })) = (p, self.tabs.get_mut(self.active)) {
            q.pane = p;
        }
    }

    fn on_main_key(&mut self, key: KeyEvent) {
        let Some(tab) = self.tabs.get(self.active) else {
            if key.code == KeyCode::Enter {
                self.run_command(Command::Connections);
            }
            return;
        };
        match &tab.kind {
            TabKind::Query(_) => self.on_query_key(key),
            TabKind::Table(_) => self.on_table_key(key),
            TabKind::Structure(_) => self.on_structure_key(key),
            TabKind::Activity(_) => self.on_activity_key(key),
            TabKind::Text(_) => self.on_text_key(key),
            TabKind::Explain(_) => self.on_explain_key(key),
            TabKind::History(_) => self.on_history_key(key),
        }
    }

    fn on_query_key(&mut self, key: KeyEvent) {
        let running = self.tabs[self.active].is_busy();
        let in_editor = self.editor_focused();
        // Esc belongs to vim first: it leaves insert or visual mode even while a query runs
        let vim_busy = in_editor
            && matches!(&self.tabs[self.active].kind, TabKind::Query(q) if !matches!(q.editor.vim_mode(), None | Some(VimMode::Normal)));
        if running && !(vim_busy && key.code == KeyCode::Esc) && self.keymap.action(&key, false, false) == Some(Action::Cancel) {
            self.cancel_active();
            return;
        }
        match self.keymap.action(&key, in_editor, in_editor) {
            Some(Action::RunAll) => return self.run_query(true),
            Some(Action::RunStatement) => return self.run_query(false),
            Some(Action::Explain) => return self.open_explain(false),
            Some(Action::ExplainAnalyze) => return self.open_explain(true),
            Some(Action::FormatSql) => return self.run_command(Command::FormatSql),
            Some(Action::SaveFavorite) => return self.run_command(Command::SaveFavorite),
            Some(Action::Export) => return self.run_command(Command::Export),
            Some(Action::EditorSmaller) => {
                if let TabKind::Query(q) = &mut self.tabs[self.active].kind {
                    q.split = q.split.saturating_sub(5).max(15);
                }
                return;
            }
            Some(Action::EditorLarger) => {
                if let TabKind::Query(q) = &mut self.tabs[self.active].kind {
                    q.split = (q.split + 5).min(85);
                }
                return;
            }
            _ => {}
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let tab_id = self.tabs[self.active].id;
        let conn = self.tabs[self.active].conn;
        let TabKind::Query(q) = &mut self.tabs[self.active].kind else { return };
        match q.pane {
            Pane::Editor => {
                let ev = q.editor.handle_key(key);
                if key.code == KeyCode::Esc && ev == EditorEvent::Unhandled {
                    q.pane = Pane::Results;
                    return;
                }
                let inserting = matches!(q.editor.vim_mode(), None | Some(VimMode::Insert));
                match ev {
                    EditorEvent::Changed => {
                        let typed_word = inserting && matches!(key.code, KeyCode::Char(c) if c.is_alphanumeric() || c == '_' || c == '.');
                        if typed_word && self.config.main.complete_while_typing {
                            self.update_completion(false);
                        } else {
                            self.completion = None;
                        }
                    }
                    EditorEvent::RequestCompletion => self.update_completion(true),
                    EditorEvent::Moved => self.completion = None,
                    EditorEvent::Unhandled => {
                        if key.code == KeyCode::Char(' ') && ctrl {
                            self.update_completion(true);
                        }
                    }
                }
            }
            Pane::Results => {
                if q.showing_messages() {
                    match key.code {
                        KeyCode::Down | KeyCode::Char('j') => q.messages_scroll = q.messages_scroll.saturating_add(1),
                        KeyCode::Up | KeyCode::Char('k') => q.messages_scroll = q.messages_scroll.saturating_sub(1),
                        KeyCode::Char('[') => {
                            let n = q.results.len();
                            if n > 0 {
                                q.show(n - 1);
                            }
                        }
                        KeyCode::Char('m') | KeyCode::Char(']') => {}
                        KeyCode::Esc | KeyCode::Char('i') => q.pane = Pane::Editor,
                        _ => {}
                    }
                    return;
                }
                match key.code {
                    KeyCode::Char('[') => {
                        let s = q.shown;
                        q.show(s.saturating_sub(1));
                        return;
                    }
                    KeyCode::Char(']') => {
                        let s = q.shown;
                        q.show(s + 1);
                        return;
                    }
                    KeyCode::Char('m') => {
                        let n = q.results.len();
                        q.show(n);
                        return;
                    }
                    KeyCode::Char('i') => {
                        q.pane = Pane::Editor;
                        return;
                    }
                    _ => {}
                }
                let ev = q.grid.handle_key(key);
                if ev == GridEvent::Unhandled && key.code == KeyCode::Esc {
                    q.pane = Pane::Editor;
                    return;
                }
                let backend = conn.and_then(|c| self.conn(c)).map(|c| c.backend()).unwrap_or(Backend::Postgres);
                self.on_grid_event(tab_id, ev, backend);
            }
        }
    }

    fn on_grid_event(&mut self, tab_id: u64, ev: GridEvent, backend: Backend) {
        let Some(idx) = self.tab_index(tab_id) else { return };
        let (grid, table_name): (&mut GridState, String) = match &mut self.tabs[idx].kind {
            TabKind::Query(q) => (&mut q.grid, "table".into()),
            TabKind::Table(t) => {
                let n = qualified(Some(&t.schema), &t.name, backend);
                (&mut t.grid, n)
            }
            TabKind::Structure(s) => (&mut s.grid, "table".into()),
            TabKind::Activity(a) => (&mut a.grid, "sessions".into()),
            _ => return,
        };
        match ev {
            GridEvent::Copy(kind) => {
                let text = match kind {
                    CopyKind::Cells => grid.copy_cells_tsv(false),
                    CopyKind::Rows => grid.copy_rows_tsv(true),
                };
                let _ = table_name;
                self.copy_to_clipboard(text);
            }
            GridEvent::OpenCell(r, c) => {
                let title = grid.columns().get(c).map(|c| format!("{} · {}", c.name, c.type_name)).unwrap_or_default();
                let null = grid.null_text.clone();
                let value = grid.rows().get(r).and_then(|row| row.get(c)).cloned();
                let row_text = grid.rows().get(r).map(|row| {
                    let w = grid.columns().iter().map(|c| c.name.chars().count()).max().unwrap_or(4);
                    grid.columns()
                        .iter()
                        .zip(row)
                        .map(|(col, v)| format!("{:<w$}  {}", col.name, if v.is_null() { null.clone() } else { v.display().replace('\n', "↵") }))
                        .collect::<Vec<_>>()
                        .join("\n")
                });
                let text = match value {
                    Some(Value::Null) => null,
                    Some(v) => pretty_value(&v.display()),
                    None => return,
                };
                let body = format!("{text}\n\n── row {} ──\n{}", r + 1, row_text.unwrap_or_default());
                self.overlay = Some(Overlay::Text(TextView {
                    title,
                    text: body,
                    scroll: 0,
                    hscroll: 0,
                    footer: "y copy value · j/k scroll · esc close".into(),
                }));
            }
            GridEvent::StartSearch => {
                self.overlay = Some(Overlay::Prompt(Prompt {
                    title: "Search results".into(),
                    hint: String::new(),
                    input: Input::new("").with_placeholder("text to find (case-insensitive)"),
                    purpose: PromptPurpose::GridSearch { tab: tab_id },
                }));
            }
            GridEvent::NoMatch => self.toast(Level::Info, "No match"),
            GridEvent::SortRequested(c) => self.table_sort(idx, c),
            GridEvent::FilterRequested(_) => self.table_filter(idx),
            GridEvent::EditCell(r, c) => self.table_edit_cell(idx, r, c),
            GridEvent::InsertRow => self.table_insert_row(idx),
            GridEvent::DeleteRows(range) => self.table_delete_rows(idx, range),
            _ => {}
        }
    }

    fn copy_to_clipboard(&mut self, text: String) {
        let lines = text.lines().count();
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
            Ok(()) => self.toast(Level::Success, format!("Copied {lines} line(s)")),
            Err(e) => self.toast(Level::Error, format!("Clipboard unavailable: {e}")),
        }
    }

    fn table_sort(&mut self, idx: usize, col: usize) {
        let TabKind::Table(t) = &mut self.tabs[idx].kind else {
            if let TabKind::Query(_) = &self.tabs[idx].kind {
                self.toast(Level::Info, "Sorting re-runs queries only in table views; add ORDER BY here");
            }
            return;
        };
        t.order = match t.order {
            Some((c, true)) if c == col => Some((col, false)),
            Some((c, false)) if c == col => None,
            _ => Some((col, true)),
        };
        match t.order {
            Some((c, asc)) => t.grid.set_sort(c, asc),
            None => t.grid.clear_sort(),
        }
        self.fetch_table_page(idx, true);
    }

    fn table_filter(&mut self, idx: usize) {
        let id = self.tabs[idx].id;
        let TabKind::Table(t) = &self.tabs[idx].kind else { return };
        let mut input = Input::new(&t.filter);
        input.placeholder = "SQL condition, e.g. status = 'active' AND created_at > now() - interval '7 days'".into();
        self.overlay = Some(Overlay::Prompt(Prompt {
            title: format!("Filter {}", t.name),
            hint: "WHERE …   (empty clears the filter)".into(),
            input,
            purpose: PromptPurpose::Filter { tab: id },
        }));
    }

    fn table_pk(&self, idx: usize) -> Option<Vec<usize>> {
        let TabKind::Table(t) = &self.tabs[idx].kind else { return None };
        let d = t.details.as_ref()?;
        let names: Vec<&str> = d.columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
        if names.is_empty() {
            return None;
        }
        names.iter().map(|n| t.grid.columns().iter().position(|c| c.name == *n)).collect()
    }

    fn table_editable(&mut self, idx: usize) -> bool {
        if self.tabs[idx].conn.and_then(|c| self.conn(c)).is_some_and(|c| c.readonly) {
            self.toast(Level::Warning, "Read-only connection");
            return false;
        }
        let TabKind::Table(t) = &self.tabs[idx].kind else { return false };
        if t.details.as_ref().is_some_and(|d| d.kind.is_view()) {
            self.toast(Level::Warning, "Views are not editable here");
            return false;
        }
        if self.table_pk(idx).is_none() {
            self.toast(Level::Warning, "Editing needs a primary key on this table");
            return false;
        }
        true
    }

    fn table_edit_cell(&mut self, idx: usize, row: usize, col: usize) {
        if !self.table_editable(idx) {
            return;
        }
        let id = self.tabs[idx].id;
        let TabKind::Table(t) = &self.tabs[idx].kind else { return };
        if t.deleted.contains(&row) {
            self.toast(Level::Info, "Row is marked for deletion (D to unmark)");
            return;
        }
        let Some(colinfo) = t.grid.columns().get(col) else { return };
        let current = t.grid.rows().get(row).and_then(|r| r.get(col)).cloned().unwrap_or(Value::Null);
        let text = if current.is_null() { "\\N".to_string() } else { current.display().into_owned() };
        let title = format!("Edit {}.{} ({})", t.name, colinfo.name, colinfo.type_name);
        self.overlay = Some(Overlay::Prompt(Prompt {
            title,
            hint: "⏎ stage change · \\N for NULL · Esc cancel — nothing is written until Ctrl+S".into(),
            input: Input::new(&text),
            purpose: PromptPurpose::EditCell { tab: id, row, col },
        }));
    }

    fn table_insert_row(&mut self, idx: usize) {
        if !self.table_editable(idx) {
            return;
        }
        let TabKind::Table(t) = &mut self.tabs[idx].kind else { return };
        let n = t.grid.column_count();
        if n == 0 {
            return;
        }
        t.grid.push_rows(vec![vec![Value::Null; n]]);
        let row = t.grid.row_count() - 1;
        t.inserted.insert(row);
        t.grid.mark_new_row(row);
        t.grid.set_cursor(row, 0);
        self.toast(Level::Info, "New row added — edit cells with e, then Ctrl+S to review");
    }

    fn table_delete_rows(&mut self, idx: usize, range: std::ops::Range<usize>) {
        if !self.table_editable(idx) {
            return;
        }
        let TabKind::Table(t) = &mut self.tabs[idx].kind else { return };
        for r in range {
            if t.inserted.remove(&r) {
                continue;
            }
            if !t.deleted.insert(r) {
                t.deleted.remove(&r);
            }
        }
        t.grid.clear_marks();
        for k in t.edits.keys() {
            t.grid.mark_edited(k.row, k.col);
        }
        for &r in &t.deleted {
            t.grid.mark_deleted_row(r);
        }
        for &r in &t.inserted {
            t.grid.mark_new_row(r);
        }
        t.grid.clear_selection();
    }

    fn build_edit_sql(&self, idx: usize) -> Result<Vec<String>, String> {
        let pk = self.table_pk(idx).ok_or("table has no primary key")?;
        let conn = self.tabs[idx].conn.and_then(|c| self.conn(c)).ok_or("not connected")?;
        let backend = conn.backend();
        let TabKind::Table(t) = &self.tabs[idx].kind else { return Err("not a table".into()) };
        let name = qualified(Some(&t.schema), &t.name, backend);
        let cols = t.grid.columns();
        let rows = t.grid.rows();
        let where_for = |row: usize| -> String {
            pk.iter()
                .map(|&c| {
                    let v = &rows[row][c];
                    let col = quote_ident(&cols[c].name, backend);
                    if v.is_null() { format!("{col} IS NULL") } else { format!("{col} = {}", v.to_sql_literal(backend)) }
                })
                .collect::<Vec<_>>()
                .join(" AND ")
        };
        let mut out = Vec::new();
        for &r in &t.deleted {
            out.push(format!("DELETE FROM {name} WHERE {}", where_for(r)));
        }
        let mut by_row: std::collections::BTreeMap<usize, Vec<(usize, &Value)>> = Default::default();
        for (k, v) in &t.edits {
            if !t.deleted.contains(&k.row) {
                by_row.entry(k.row).or_default().push((k.col, v));
            }
        }
        for (row, changes) in &by_row {
            if t.inserted.contains(row) {
                continue;
            }
            let sets: Vec<String> = changes
                .iter()
                .map(|(c, v)| format!("{} = {}", quote_ident(&cols[*c].name, backend), v.to_sql_literal(backend)))
                .collect();
            out.push(format!("UPDATE {name} SET {} WHERE {}", sets.join(", "), original_where(t, *row, &pk, backend)));
        }
        for &r in &t.inserted {
            let changes = by_row.get(&r).cloned().unwrap_or_default();
            if changes.is_empty() {
                out.push(match backend {
                    Backend::MySql => format!("INSERT INTO {name} () VALUES ()"),
                    _ => format!("INSERT INTO {name} DEFAULT VALUES"),
                });
                continue;
            }
            let names: Vec<String> = changes.iter().map(|(c, _)| quote_ident(&cols[*c].name, backend)).collect();
            let vals: Vec<String> = changes.iter().map(|(_, v)| v.to_sql_literal(backend)).collect();
            out.push(format!("INSERT INTO {name} ({}) VALUES ({})", names.join(", "), vals.join(", ")));
        }
        let _ = where_for;
        Ok(out)
    }

    fn review_edits(&mut self, idx: usize) {
        let id = self.tabs[idx].id;
        let backend = self.tabs[idx].conn.and_then(|c| self.conn(c)).map(|c| c.backend()).unwrap_or(Backend::Postgres);
        match self.build_edit_sql(idx) {
            Ok(stmts) if stmts.is_empty() => self.toast(Level::Info, "No pending changes"),
            Ok(stmts) => {
                let mut lines = vec![Line::from(Span::styled(
                    format!("{} statement(s) will run in one transaction:", stmts.len()),
                    Style::default().fg(self.theme.muted),
                ))];
                lines.push(Line::from(""));
                for s in &stmts {
                    let mut spans = highlight_spans(s, backend, &self.theme);
                    spans.push(Span::raw(";"));
                    lines.push(Line::from(spans));
                }
                self.overlay = Some(Overlay::Confirm(Confirm {
                    title: "Apply changes".into(),
                    lines,
                    yes: "Apply".into(),
                    danger: stmts.iter().any(|s| s.starts_with("DELETE")),
                    on_yes: Command::ApplyEdits(id),
                    scroll: 0,
                }));
            }
            Err(e) => self.toast(Level::Error, e),
        }
    }

    fn on_table_key(&mut self, key: KeyEvent) {
        let idx = self.active;
        let tab_id = self.tabs[idx].id;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let backend = self.tabs[idx].conn.and_then(|c| self.conn(c)).map(|c| c.backend()).unwrap_or(Backend::Postgres);
        match key.code {
            KeyCode::Char('s') if ctrl => return self.review_edits(idx),
            KeyCode::Char('r') | KeyCode::F(5) if !ctrl => {
                let dirty = matches!(&self.tabs[idx].kind, TabKind::Table(t) if t.dirty());
                if dirty {
                    self.toast(Level::Warning, "Apply (Ctrl+S) or discard (u) pending changes first");
                } else {
                    self.fetch_table_page(idx, true);
                }
                return;
            }
            KeyCode::Char('u') => {
                if let TabKind::Table(t) = &mut self.tabs[idx].kind
                    && t.dirty() {
                        t.edits.clear();
                        t.deleted.clear();
                        t.inserted.clear();
                        t.original.clear();
                        t.grid.clear_marks();
                        self.toast(Level::Info, "Discarded pending changes");
                        self.fetch_table_page(idx, true);
                    }
                return;
            }
            KeyCode::Char('F') => {
                let TabKind::Table(t) = &mut self.tabs[idx].kind else { return };
                if let (Some((_, c)), Some(v)) = (t.grid.selected_cell(), t.grid.selected_value().cloned()) {
                    let col = quote_ident(&t.grid.columns()[c].name, backend);
                    t.filter = if v.is_null() { format!("{col} IS NULL") } else { format!("{col} = {}", v.to_sql_literal(backend)) };
                    self.fetch_table_page(idx, true);
                }
                return;
            }
            KeyCode::Char('x') if ctrl => return self.run_command(Command::Export),
            KeyCode::Esc => {
                let TabKind::Table(t) = &mut self.tabs[idx].kind else { return };
                if !t.filter.is_empty() && t.grid.selection_range().is_none_or(|(r, c)| r.len() <= 1 && c.len() <= 1) {
                    t.filter.clear();
                    self.fetch_table_page(idx, true);
                    return;
                }
            }
            _ => {}
        }
        let TabKind::Table(t) = &mut self.tabs[idx].kind else { return };
        let ev = t.grid.handle_key(key);
        let near_end = t.grid.selected_cell().is_some_and(|(r, _)| r + 50 >= t.loaded);
        let want_more = near_end && t.has_more && !t.loading && !t.dirty();
        self.on_grid_event(tab_id, ev, backend);
        if want_more {
            self.fetch_table_page(idx, false);
        }
    }

    fn on_structure_key(&mut self, key: KeyEvent) {
        let tab_id = self.tabs[self.active].id;
        let backend = self.tabs[self.active].conn.and_then(|c| self.conn(c)).map(|c| c.backend()).unwrap_or(Backend::Postgres);
        let TabKind::Structure(s) = &mut self.tabs[self.active].kind else { return };
        let pos = StructSection::ALL.iter().position(|x| *x == s.section).unwrap_or(0);
        let n = StructSection::ALL.len();
        let target = match key.code {
            KeyCode::Tab | KeyCode::Char(']') => Some((pos + 1) % n),
            KeyCode::BackTab | KeyCode::Char('[') => Some((pos + n - 1) % n),
            KeyCode::Char(c @ '1'..='7') => Some(c as usize - '1' as usize),
            _ => None,
        };
        if let Some(t) = target {
            s.section = StructSection::ALL[t];
            s.text_scroll = 0;
            s.load_section();
            return;
        }
        if s.section == StructSection::Ddl {
            let lines = s.ddl.as_deref().map(|d| d.lines().count()).unwrap_or(0);
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => s.text_scroll = (s.text_scroll + 1).min(lines.saturating_sub(1)),
                KeyCode::Up | KeyCode::Char('k') => s.text_scroll = s.text_scroll.saturating_sub(1),
                KeyCode::PageDown => s.text_scroll = (s.text_scroll + 20).min(lines.saturating_sub(1)),
                KeyCode::PageUp => s.text_scroll = s.text_scroll.saturating_sub(20),
                KeyCode::Char('y') => {
                    let text = s.ddl.clone().unwrap_or_default();
                    self.copy_to_clipboard(text);
                }
                KeyCode::Char('e') => {
                    let text = s.ddl.clone().unwrap_or_default();
                    let conn = self.tabs[self.active].conn;
                    self.new_query_tab(conn, Some(text));
                }
                _ => {}
            }
            return;
        }
        let ev = s.grid.handle_key(key);
        self.on_grid_event(tab_id, ev, backend);
    }

    fn on_activity_key(&mut self, key: KeyEvent) {
        let tab_id = self.tabs[self.active].id;
        let backend = self.tabs[self.active].conn.and_then(|c| self.conn(c)).map(|c| c.backend()).unwrap_or(Backend::Postgres);
        let TabKind::Activity(a) = &mut self.tabs[self.active].kind else { return };
        match key.code {
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                a.paused = !a.paused;
                return;
            }
            KeyCode::Char('r') => {
                a.last_refresh = None;
                return;
            }
            KeyCode::Char('K') => {
                if let Some(row) = a.grid.selected_row() {
                    let sid = row.first().map(|v| v.display().into_owned()).unwrap_or_default();
                    let summary: Vec<String> = a.grid.columns().iter().zip(row).take(6).map(|(c, v)| format!("{}: {}", c.name, v.display())).collect();
                    let mut lines = vec![Line::from(format!("Terminate session {sid}?")), Line::from("")];
                    lines.extend(summary.into_iter().map(Line::from));
                    self.overlay = Some(Overlay::Confirm(Confirm {
                        title: "Kill session".into(),
                        lines,
                        yes: "Terminate".into(),
                        danger: true,
                        on_yes: Command::Kill(tab_id, sid),
                        scroll: 0,
                    }));
                }
                return;
            }
            _ => {}
        }
        let ev = a.grid.handle_key(key);
        self.on_grid_event(tab_id, ev, backend);
    }

    fn on_text_key(&mut self, key: KeyEvent) {
        let conn = self.tabs[self.active].conn;
        let TabKind::Text(t) = &mut self.tabs[self.active].kind else { return };
        let lines = t.text.lines().count();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => t.scroll = (t.scroll + 1).min(lines.saturating_sub(1)),
            KeyCode::Up | KeyCode::Char('k') => t.scroll = t.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => t.scroll = (t.scroll + 20).min(lines.saturating_sub(1)),
            KeyCode::PageUp => t.scroll = t.scroll.saturating_sub(20),
            KeyCode::Char('g') | KeyCode::Home => t.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => t.scroll = lines.saturating_sub(1),
            KeyCode::Char('y') => {
                let text = t.text.clone();
                self.copy_to_clipboard(text);
            }
            KeyCode::Char('e') => {
                let text = t.text.clone();
                self.new_query_tab(conn, Some(text));
            }
            _ => {}
        }
    }

    fn on_explain_key(&mut self, key: KeyEvent) {
        let TabKind::Explain(x) = &mut self.tabs[self.active].kind else { return };
        let n = x.flat.len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => x.selected = (x.selected + 1).min(n.saturating_sub(1)),
            KeyCode::Up | KeyCode::Char('k') => x.selected = x.selected.saturating_sub(1),
            KeyCode::Home | KeyCode::Char('g') => x.selected = 0,
            KeyCode::End | KeyCode::Char('G') => x.selected = n.saturating_sub(1),
            _ => {}
        }
    }

    fn on_history_key(&mut self, key: KeyEvent) {
        let conn = self.tabs[self.active].conn;
        let TabKind::History(h) = &mut self.tabs[self.active].kind else { return };
        match key.code {
            KeyCode::Down => h.selected = (h.selected + 1).min(h.filtered.len().saturating_sub(1)),
            KeyCode::Up => h.selected = h.selected.saturating_sub(1),
            KeyCode::PageDown => h.selected = (h.selected + 20).min(h.filtered.len().saturating_sub(1)),
            KeyCode::PageUp => h.selected = h.selected.saturating_sub(20),
            KeyCode::Enter => {
                if let Some(&i) = h.filtered.get(h.selected) {
                    let sql = h.entries[i].sql.clone();
                    self.new_query_tab(conn, Some(sql));
                }
            }
            _ => {
                if let InputEvent::Changed = h.filter.handle_key(key) {
                    h.refilter();
                }
            }
        }
    }

    // ---------------------------------------------------------------- completion

    /// The completer for a tab: unqualified names resolve in the tab's database or schema first.
    fn completer_for(&mut self, conn: ConnId, scope: Option<&Scope>) -> Option<Arc<Completer>> {
        let Some(scope) = scope else { return self.conn(conn)?.completer.clone() };
        let c = self.conn(conn)?;
        if let Some(done) = c.scoped_completers.get(scope.name()) {
            return Some(done.clone());
        }
        let mut cat = (**c.catalog.as_ref()?).clone();
        cat.search_path.retain(|s| s != scope.name());
        cat.search_path.insert(0, scope.name().to_string());
        let completer = Arc::new(self.make_completer(Arc::new(cat)));
        self.conn_mut(conn)?.scoped_completers.insert(scope.name().to_string(), completer.clone());
        Some(completer)
    }

    fn update_completion(&mut self, explicit: bool) {
        let conn = self.tabs.get(self.active).and_then(|t| t.conn);
        let scope = match self.tabs.get(self.active).map(|t| &t.kind) {
            Some(TabKind::Query(q)) => q.scope.clone(),
            _ => None,
        };
        let completer = conn.and_then(|c| self.completer_for(c, scope.as_ref()));
        let Some(Tab { kind: TabKind::Query(q), .. }) = self.tabs.get_mut(self.active) else { return };
        let Some(completer) = completer else {
            self.completion = None;
            return;
        };
        let text = q.editor.text();
        let cursor = q.editor.cursor_byte();
        let word = q.editor.word_before_cursor().to_string();
        let after_dot = text[..cursor].ends_with('.');
        if let Some(conn) = conn {
            self.request_schema_for_completion(conn, &text[..cursor]);
        }
        if !explicit && word.is_empty() && !after_dot {
            self.completion = None;
            return;
        }
        let res = completer.complete(&text, cursor);
        let mut items = res.items;
        if !explicit && items.len() == 1 && items[0].text == word {
            items.clear();
        }
        if items.is_empty() {
            self.completion = None;
            return;
        }
        let keep = self.completion.as_ref().map(|c| c.selected).unwrap_or(0);
        self.completion = Some(CompletionPopup { selected: keep.min(items.len() - 1), offset: 0, replace_start: res.replace_start.min(cursor), items });
    }

    fn on_completion_key(&mut self, key: KeyEvent) -> bool {
        let Some(popup) = &mut self.completion else { return false };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Down => popup.selected = (popup.selected + 1) % popup.items.len(),
            KeyCode::Char('n') if ctrl => popup.selected = (popup.selected + 1) % popup.items.len(),
            KeyCode::Up => popup.selected = (popup.selected + popup.items.len() - 1) % popup.items.len(),
            KeyCode::Char('p') if ctrl => popup.selected = (popup.selected + popup.items.len() - 1) % popup.items.len(),
            KeyCode::PageDown => popup.selected = (popup.selected + 8).min(popup.items.len() - 1),
            KeyCode::PageUp => popup.selected = popup.selected.saturating_sub(8),
            KeyCode::Tab | KeyCode::Enter if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                let item = popup.items[popup.selected].clone();
                let start = popup.replace_start;
                self.completion = None;
                if let Some(Tab { kind: TabKind::Query(q), .. }) = self.tabs.get_mut(self.active) {
                    let cursor = q.editor.cursor_byte();
                    q.editor.replace_range(start..cursor, &item.text);
                }
            }
            KeyCode::Esc => {
                self.completion = None;
                // in vim, the same Esc also leaves insert mode
                return self.vim_mode().is_none();
            }
            _ => return false,
        }
        true
    }

    // ---------------------------------------------------------------- overlays

    fn on_overlay_key(&mut self, key: KeyEvent) {
        let Some(overlay) = self.overlay.as_mut() else { return };
        match overlay {
            Overlay::Commands(p) => match p.handle_key(key) {
                PaletteEvent::Execute(cmd) => {
                    self.overlay = None;
                    self.run_command(cmd);
                }
                PaletteEvent::Close => self.overlay = None,
                _ => {}
            },
            Overlay::Themes(p, original) => match p.handle_key(key) {
                PaletteEvent::Preview(name) => {
                    if let Ok(t) = crate::theme::load(&name, &self.config.themes_dir()) {
                        self.theme = t.adapted(self.depth);
                    }
                }
                PaletteEvent::Execute(name) => {
                    if let Ok(t) = crate::theme::load(&name, &self.config.themes_dir()) {
                        self.theme = t.adapted(self.depth);
                        let mut st = load_ui_state();
                        st.theme = Some(name.clone());
                        save_ui_state(&st);
                        self.toast(Level::Success, format!("Theme: {name}"));
                    }
                    self.overlay = None;
                }
                PaletteEvent::Close => {
                    self.theme = (**original).clone();
                    self.overlay = None;
                }
                PaletteEvent::None => {}
            },
            Overlay::Help(h) => {
                if let DialogResult::Close = h.handle_key(key) {
                    self.overlay = None;
                }
            }
            Overlay::Confirm(c) => match c.handle_key(key) {
                DialogResult::Emit(cmd) => {
                    self.overlay = None;
                    self.run_command(cmd);
                }
                DialogResult::Close => self.overlay = None,
                DialogResult::None => {}
            },
            Overlay::Prompt(p) => match p.handle_key(key) {
                DialogResult::Emit((purpose, value)) => {
                    self.overlay = None;
                    self.on_prompt(purpose, value);
                }
                DialogResult::Close => {
                    if let PromptPurpose::Password { conn } = p.purpose {
                        self.pending_connects.remove(&conn);
                    }
                    self.overlay = None;
                }
                DialogResult::None => {}
            },
            Overlay::Text(t) => match t.handle_key(key) {
                DialogResult::Close => self.overlay = None,
                DialogResult::Emit(()) => {
                    let first = t.text.split("\n\n── row").next().unwrap_or("").to_string();
                    self.overlay = None;
                    self.copy_to_clipboard(first);
                }
                DialogResult::None => {}
            },
            Overlay::Connect(form) => {
                let ev = form.handle_key(key);
                self.on_connect_event(ev);
            }
        }
    }

    fn on_connect_event(&mut self, ev: ConnectEvent) {
        match ev {
            ConnectEvent::Close => self.overlay = None,
            ConnectEvent::Connect { spec, save_as, name } => {
                if let Some(id) = open_conn_named(&self.conns, &name, spec.backend) {
                    self.overlay = None;
                    self.focus_connection(id);
                    self.toast(Level::Info, format!("Already connected to {name}"));
                    return;
                }
                if self.pending_connects.values().any(|(n, _, _)| *n == name) {
                    return;
                }
                if let Some(Overlay::Connect(form)) = &mut self.overlay {
                    form.busy = true;
                }
                self.start_connect(name, *spec, save_as);
            }
            ConnectEvent::Delete(name) => {
                self.config.connections.remove(&name);
                if let Some(Overlay::Connect(form)) = &mut self.overlay {
                    form.saved.retain(|(n, _)| *n != name);
                }
                match self.config.save() {
                    Ok(()) => self.toast(Level::Info, format!("Deleted connection '{name}'")),
                    Err(e) => self.toast(Level::Error, e.to_string()),
                }
            }
            ConnectEvent::None => {}
        }
    }

    fn on_prompt(&mut self, purpose: PromptPurpose, value: String) {
        match purpose {
            PromptPurpose::SaveFavorite => {
                let name = value.trim().to_string();
                if name.is_empty() {
                    return;
                }
                let Some(Tab { kind: TabKind::Query(q), .. }) = self.tabs.get(self.active) else { return };
                let sql = q.editor.selected_text().unwrap_or_else(|| q.editor.text());
                self.favorites.queries.insert(name.clone(), sql.trim().to_string());
                match self.favorites.save() {
                    Ok(()) => self.toast(Level::Success, format!("Saved favorite '{name}' (run with \\f {name})")),
                    Err(e) => self.toast(Level::Error, e.to_string()),
                }
            }
            PromptPurpose::EditCell { tab, row, col } => {
                let Some(idx) = self.tab_index(tab) else { return };
                let TabKind::Table(t) = &mut self.tabs[idx].kind else { return };
                let new = if value == "\\N" { Value::Null } else { Value::Text(value) };
                let old = t.grid.rows().get(row).and_then(|r| r.get(col)).cloned();
                if old.as_ref() == Some(&new) {
                    return;
                }
                if !t.original.contains_key(&row)
                    && let Some(r) = t.grid.rows().get(row) {
                        t.original.insert(row, r.clone());
                    }
                t.grid.set_cell(row, col, new.clone());
                t.grid.mark_edited(row, col);
                t.edits.insert(CellKey { row, col }, new);
            }
            PromptPurpose::Filter { tab } => {
                let Some(idx) = self.tab_index(tab) else { return };
                if let TabKind::Table(t) = &mut self.tabs[idx].kind {
                    t.filter = value.trim().to_string();
                }
                self.fetch_table_page(idx, true);
            }
            PromptPurpose::GridSearch { tab } => {
                let Some(idx) = self.tab_index(tab) else { return };
                let grid = match &mut self.tabs[idx].kind {
                    TabKind::Query(q) => &mut q.grid,
                    TabKind::Table(t) => &mut t.grid,
                    TabKind::Activity(a) => &mut a.grid,
                    TabKind::Structure(s) => &mut s.grid,
                    _ => return,
                };
                let q = value.trim().to_string();
                grid.set_search((!q.is_empty()).then_some(q));
                if !grid.search_next(true) {
                    self.toast(Level::Info, "No match");
                }
            }
            PromptPurpose::ExportPath => self.export_to(value.trim()),
            PromptPurpose::Llm { tab } => {
                if !value.trim().is_empty() {
                    self.start_llm(tab, value.trim().to_string());
                }
            }
            PromptPurpose::SaveFile => {
                let path = crate::conn::url::expand_tilde(value.trim());
                let Some(Tab { kind: TabKind::Query(q), title, .. }) = self.tabs.get_mut(self.active) else { return };
                match std::fs::write(&path, q.editor.text()) {
                    Ok(()) => {
                        *title = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                        q.file = Some(path.clone());
                        self.toast(Level::Success, format!("Saved {}", path.display()));
                    }
                    Err(e) => self.toast(Level::Error, format!("{}: {e}", path.display())),
                }
            }
            PromptPurpose::OpenFile => {
                let path = crate::conn::url::expand_tilde(value.trim());
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        let conn = self.active_conn_id();
                        let i = self.new_query_tab(conn, Some(text));
                        self.tabs[i].title = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                        if let TabKind::Query(q) = &mut self.tabs[i].kind {
                            q.file = Some(path);
                        }
                    }
                    Err(e) => self.toast(Level::Error, format!("{}: {e}", path.display())),
                }
            }
            PromptPurpose::Password { conn } => {
                if let Some((name, mut spec, save_as)) = self.pending_connects.remove(&conn) {
                    spec.password = Some(value);
                    self.start_connect(name, *spec, save_as);
                }
            }
        }
    }

    fn export_to(&mut self, path: &str) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let backend = tab.conn.and_then(|c| self.conn(c)).map(|c| c.backend()).unwrap_or(Backend::Postgres);
        let (grid, table) = match &tab.kind {
            TabKind::Query(q) => (&q.grid, "query_result".to_string()),
            TabKind::Table(t) => (&t.grid, qualified(Some(&t.schema), &t.name, backend)),
            TabKind::Activity(a) => (&a.grid, "sessions".into()),
            _ => {
                self.toast(Level::Info, "Nothing to export here");
                return;
            }
        };
        let dest = crate::conn::url::expand_tilde(path);
        let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or("csv").to_ascii_lowercase();
        let format = match ext.as_str() {
            "json" => TableFormat::Json,
            "jsonl" | "ndjson" => TableFormat::JsonLines,
            "tsv" | "tab" => TableFormat::Tsv,
            "md" | "markdown" => TableFormat::Markdown,
            "html" | "htm" => TableFormat::Html,
            "sql" => TableFormat::SqlInsert,
            "txt" => TableFormat::Ascii,
            _ => TableFormat::Csv,
        };
        let text = render_grid(grid, format, table, backend, &self.theme);
        let rows = grid.row_count();
        match crate::repl::session::write_private(&dest, &text) {
            Ok(()) => self.toast(Level::Success, format!("Exported {rows} rows → {}", dest.display())),
            Err(e) => self.toast(Level::Error, format!("{}: {e}", dest.display())),
        }
    }

    // ---------------------------------------------------------------- commands

    fn open_commands(&mut self) {
        let keymap = &self.keymap;
        let key = |a: Action| keymap.short(a);
        let mut items: Vec<Item<Command>> = Vec::new();
        let mut add = |label: &str, hint: &str, cmd: Command| {
            items.push(Item { label: label.into(), category: String::new(), hint: hint.into(), value: cmd });
        };
        add("Run statement", &key(Action::RunStatement), Command::RunStatement);
        add("Run all", &key(Action::RunAll), Command::RunAll);
        add("Cancel running query", &key(Action::Cancel), Command::Cancel);
        add("Explain", &key(Action::Explain), Command::Explain(false));
        add("Explain analyze", &key(Action::ExplainAnalyze), Command::Explain(true));
        add("Format SQL", &key(Action::FormatSql), Command::FormatSql);
        add("Ask the model to write SQL…", "\\llm", Command::AskLlm);
        add("New query tab", &key(Action::NewQuery), Command::NewQuery);
        add("Close tab", &key(Action::CloseTab), Command::CloseTab);
        add("Next tab", &key(Action::NextTab), Command::NextTab);
        add("Previous tab", &key(Action::PrevTab), Command::PrevTab);
        add("Go to table…", &key(Action::GoToTable), Command::GoToTable);
        add("Table structure", "s in explorer", Command::Structure);
        add("Toggle explorer", &key(Action::ToggleExplorer), Command::ToggleSidebar);
        add("Focus explorer", &key(Action::FocusExplorer), Command::FocusSidebar);
        add("Focus editor", "", Command::FocusEditor);
        add("Focus results", "", Command::FocusResults);
        add("Commit transaction", "", Command::Commit);
        add("Rollback transaction", "", Command::Rollback);
        add("Server activity / sessions", "", Command::Activity);
        add("Query history", &key(Action::History), Command::History);
        add("Switch theme…", &key(Action::Themes), Command::Themes);
        add("Toggle transparent background", "", Command::ToggleTransparent);
        add("Connections…", &key(Action::Connections), Command::Connections);
        add("Refresh schema", "r in explorer", Command::Refresh);
        add("Export results to file…", &key(Action::Export), Command::Export);
        add("Copy results as CSV", "", Command::Copy(TableFormat::Csv));
        add("Copy results as JSON", "", Command::Copy(TableFormat::Json));
        add("Copy results as Markdown", "", Command::Copy(TableFormat::Markdown));
        add("Copy results as SQL INSERT", "", Command::Copy(TableFormat::SqlInsert));
        add("Save query as favorite…", &key(Action::SaveFavorite), Command::SaveFavorite);
        add("Favorite queries…", "", Command::Favorites);
        add("Save editor to file…", "", Command::SaveFile);
        add("Open SQL file…", "", Command::OpenFile);
        add("Toggle read-only for this connection", "", Command::ToggleReadonly);
        add("Keyboard shortcuts", &key(Action::Help), Command::Help);
        add("Quit", &key(Action::Quit), Command::Quit);
        for name in self.favorites.queries.keys() {
            items.push(Item { label: name.clone(), category: "Favorite".into(), hint: String::new(), value: Command::OpenFavorite(name.clone()) });
        }
        self.overlay = Some(Overlay::Commands(Palette::new("Commands", "type a command…", items)));
    }

    pub fn run_command(&mut self, cmd: Command) {
        match cmd {
            Command::NewQuery => match self.sidebar.console_target() {
                Some(sidebar::Action::NewConsole { conn, scope, database }) if self.focus == Focus::Sidebar => {
                    self.open_console(conn, scope, database)
                }
                _ => {
                    self.new_query_tab(None, None);
                }
            },
            Command::CloseTab => self.close_tab(self.active),
            Command::CloseTabAt(i) => self.close_tab(i),
            Command::Commands => self.open_commands(),
            Command::ToggleTransparent => self.config.main.transparent = !self.config.main.transparent,
            Command::NextTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + 1) % self.tabs.len();
                    self.completion = None;
                }
            }
            Command::PrevTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
                    self.completion = None;
                }
            }
            Command::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                if !self.sidebar_visible && self.focus == Focus::Sidebar {
                    self.focus = Focus::Main;
                }
            }
            Command::FocusSidebar => {
                self.sidebar_visible = true;
                self.focus = Focus::Sidebar;
            }
            Command::FocusEditor | Command::FocusResults => {
                self.focus = Focus::Main;
                if let Some(Tab { kind: TabKind::Query(q), .. }) = self.tabs.get_mut(self.active) {
                    q.pane = if cmd == Command::FocusEditor { Pane::Editor } else { Pane::Results };
                }
            }
            Command::RunStatement => self.run_query(false),
            Command::RunAll => self.run_query(true),
            Command::Cancel => {
                if !self.cancel_active() {
                    self.toast(Level::Info, "Nothing is running");
                }
            }
            Command::Explain(a) => self.open_explain(a),
            Command::FormatSql => {
                if let Some(Tab { kind: TabKind::Query(q), .. }) = self.tabs.get_mut(self.active) {
                    let backend = q.editor_backend();
                    let formatted = crate::sql::format::format_sql(&q.editor.text(), backend, &Default::default());
                    let all = q.editor.text().len();
                    q.editor.replace_range(0..all, &formatted);
                }
            }
            Command::Commit | Command::Rollback => {
                let sql = if cmd == Command::Commit { "COMMIT" } else { "ROLLBACK" };
                if let Some(c) = self.active_conn_id().and_then(|c| self.conn(c)) {
                    c.main.send(Tag::Silent, Request::Query(sql.into()));
                    self.toast(Level::Info, sql);
                }
            }
            Command::Activity => self.open_activity(),
            Command::History => self.open_history(),
            Command::Themes => {
                let names = crate::theme::list_all(&self.config.themes_dir());
                let items = names
                    .into_iter()
                    .map(|n| Item { label: n.clone(), category: String::new(), hint: String::new(), value: n })
                    .collect();
                let mut p = Palette::new("Theme", "type to filter — ↑↓ preview", items);
                p.select_label(&self.theme.name);
                self.overlay = Some(Overlay::Themes(p, Box::new(self.theme.clone())));
            }
            Command::Connections => self.open_connection_manager(),
            Command::GoToTable => {
                let mut items = Vec::new();
                for c in self.conns.iter().flatten() {
                    if let Some(cat) = &c.catalog {
                        for r in cat.relations() {
                            if sidebar::is_system_schema(&r.schema, c.backend()) {
                                continue;
                            }
                            items.push(Item {
                                label: format!("{}.{}", r.schema, r.name),
                                category: if self.conns.iter().flatten().count() > 1 { c.name.clone() } else { String::new() },
                                hint: r.kind.label().into(),
                                value: Command::OpenTable(c.id, r.schema.clone(), r.name.clone()),
                            });
                        }
                    }
                }
                if items.is_empty() {
                    self.toast(Level::Info, "No tables loaded yet");
                } else {
                    self.overlay = Some(Overlay::Commands(Palette::new("Go to table", "table name…", items)));
                }
            }
            Command::OpenTable(c, s, n) => self.open_table(c, s, n),
            Command::Structure => {
                let target = match self.active_tab().map(|t| (&t.kind, t.conn)) {
                    Some((TabKind::Table(t), Some(c))) => Some((c, t.schema.clone(), t.name.clone())),
                    _ => match self.sidebar.selected_node().map(|n| (n.conn, n.kind.clone())) {
                        Some((c, sidebar::NodeKind::Relation { schema, name, .. })) => Some((c, schema, name)),
                        _ => None,
                    },
                };
                match target {
                    Some((c, s, n)) => self.open_structure(c, s, n),
                    None => self.toast(Level::Info, "Select a table first"),
                }
            }
            Command::Refresh => {
                if let Some(c) = self.active_conn_id() {
                    self.load_catalog(c);
                    self.toast(Level::Info, "Refreshing schema…");
                }
            }
            Command::Export => {
                self.overlay = Some(Overlay::Prompt(Prompt {
                    title: "Export results".into(),
                    hint: "Format follows the extension: .csv .tsv .json .jsonl .md .html .sql .txt".into(),
                    input: Input::new("~/export.csv"),
                    purpose: PromptPurpose::ExportPath,
                }));
            }
            Command::Copy(format) => {
                let Some(tab) = self.tabs.get(self.active) else { return };
                let backend = tab.conn.and_then(|c| self.conn(c)).map(|c| c.backend()).unwrap_or(Backend::Postgres);
                let (grid, table) = match &tab.kind {
                    TabKind::Query(q) => (&q.grid, "query_result".to_string()),
                    TabKind::Table(t) => (&t.grid, qualified(Some(&t.schema), &t.name, backend)),
                    _ => return,
                };
                if grid.column_count() > 0 {
                    let text = render_grid(grid, format, table, backend, &self.theme);
                    self.copy_to_clipboard(text);
                }
            }
            Command::SaveFavorite => {
                if matches!(self.active_tab().map(|t| &t.kind), Some(TabKind::Query(_))) {
                    self.overlay = Some(Overlay::Prompt(Prompt {
                        title: "Save favorite query".into(),
                        hint: "Name to recall it with \\f name (placeholders $1, $2 … are supported)".into(),
                        input: Input::new(""),
                        purpose: PromptPurpose::SaveFavorite,
                    }));
                }
            }
            Command::Favorites => {
                let items: Vec<Item<Command>> = self
                    .favorites
                    .queries
                    .iter()
                    .map(|(k, v)| Item { label: k.clone(), category: String::new(), hint: v.lines().next().unwrap_or("").chars().take(40).collect(), value: Command::OpenFavorite(k.clone()) })
                    .collect();
                if items.is_empty() {
                    self.toast(Level::Info, "No favorites yet — Ctrl+S in a query tab saves one");
                } else {
                    self.overlay = Some(Overlay::Commands(Palette::new("Favorites", "name…", items)));
                }
            }
            Command::OpenFavorite(name) => {
                if let Some(sql) = self.favorites.queries.get(&name).cloned() {
                    let conn = self.active_conn_id();
                    let i = self.new_query_tab(conn, Some(sql));
                    self.tabs[i].title = name;
                }
            }
            Command::DropFavorite(name) => {
                self.favorites.queries.remove(&name);
                let _ = self.favorites.save();
            }
            Command::ToggleReadonly => {
                if let Some(id) = self.active_conn_id() {
                    let backend = self.conn(id).map(|c| c.backend());
                    if let Some(c) = self.conn_mut(id) {
                        c.readonly = !c.readonly;
                        let on = c.readonly;
                        let sql = match (backend, on) {
                            (Some(Backend::Postgres), true) => "SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY",
                            (Some(Backend::Postgres), false) => "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE",
                            (Some(Backend::MySql), true) => "SET SESSION TRANSACTION READ ONLY",
                            (Some(Backend::MySql), false) => "SET SESSION TRANSACTION READ WRITE",
                            (_, true) => "PRAGMA query_only = ON",
                            (_, false) => "PRAGMA query_only = OFF",
                        };
                        c.main.send(Tag::Silent, Request::Query(sql.into()));
                        self.toast(Level::Info, if on { "Read-only on" } else { "Read-only off" });
                    }
                }
            }
            Command::SaveFile => {
                let current = match self.active_tab().map(|t| &t.kind) {
                    Some(TabKind::Query(q)) => q.file.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "~/query.sql".into()),
                    _ => return,
                };
                self.overlay = Some(Overlay::Prompt(Prompt { title: "Save SQL file".into(), hint: String::new(), input: Input::new(&current), purpose: PromptPurpose::SaveFile }));
            }
            Command::OpenFile => {
                self.overlay = Some(Overlay::Prompt(Prompt { title: "Open SQL file".into(), hint: String::new(), input: Input::new("~/"), purpose: PromptPurpose::OpenFile }));
            }
            Command::Help => {
                let rows = Action::ALL
                    .iter()
                    .map(|&a| HelpRow {
                        section: a.section().to_string(),
                        keys: self.keymap.label(a),
                        what: a.description().to_string(),
                        action: a.name().to_string(),
                    })
                    .collect();
                self.overlay = Some(Overlay::Help(HelpView::new(rows)));
            }
            Command::Quit => {
                let busy = self.tabs.iter().any(|t| t.is_busy());
                let tx = self.conns.iter().flatten().any(|c| c.in_tx);
                let dirty = self.tabs.iter().any(|t| matches!(&t.kind, TabKind::Table(tt) if tt.dirty()));
                if busy || tx || dirty {
                    let mut lines = Vec::new();
                    if busy {
                        lines.push(Line::from("• a query is still running"));
                    }
                    if tx {
                        lines.push(Line::from("• a transaction is open (it will be rolled back)"));
                    }
                    if dirty {
                        lines.push(Line::from("• a table view has unapplied changes"));
                    }
                    self.overlay = Some(Overlay::Confirm(Confirm {
                        title: "Quit quarry?".into(),
                        lines,
                        yes: "Quit".into(),
                        danger: true,
                        on_yes: Command::ForceQuit,
                        scroll: 0,
                    }));
                } else {
                    self.quit = true;
                }
            }
            Command::ForceQuit => self.quit = true,
            Command::RunConfirmed(tab, stmts) => self.dispatch_script(tab, stmts),
            Command::AskLlm => {
                let tab = match self.tabs.get(self.active) {
                    Some(t) if matches!(t.kind, TabKind::Query(_)) => t.id,
                    _ => {
                        let i = self.new_query_tab(None, None);
                        self.tabs[i].id
                    }
                };
                self.overlay = Some(Overlay::Prompt(Prompt {
                    title: "Ask the model".into(),
                    hint: "Describe the data you want; the SQL is written into the editor for you to review.".into(),
                    input: Input::new("").with_placeholder("e.g. top 10 customers by revenue this month"),
                    purpose: PromptPurpose::Llm { tab },
                }));
            }
            Command::ApplyEdits(id) if id > u64::MAX / 2 => {
                let real = u64::MAX - id;
                if let Some(i) = self.tab_index(real) {
                    if let TabKind::Table(t) = &mut self.tabs[i].kind {
                        t.edits.clear();
                        t.deleted.clear();
                        t.inserted.clear();
                    }
                    self.close_tab(i);
                }
            }
            Command::ApplyEdits(id) => {
                let Some(idx) = self.tab_index(id) else { return };
                match self.build_edit_sql(idx) {
                    Ok(stmts) => {
                        if let Some(c) = self.tabs[idx].conn.and_then(|c| self.conn(c)) {
                            c.meta().send(Tag::Tab(id, 0), Request::Transaction(stmts));
                        }
                    }
                    Err(e) => self.toast(Level::Error, e),
                }
            }
            Command::Kill(tab, sid) => {
                if let Some(idx) = self.tab_index(tab)
                    && let Some(c) = self.tabs[idx].conn.and_then(|c| self.conn(c)) {
                        c.meta().send(Tag::Tab(tab, 0), Request::Kill(sid));
                    }
            }
            Command::SwitchDatabase(conn, name) => {
                if let Some(c) = self.conn(conn) {
                    c.main.send(Tag::Silent, Request::ChangeDatabase(name.clone()));
                    c.meta().send(Tag::Sidebar, Request::ChangeDatabase(name.clone()));
                }
                if let Some(c) = self.conn_mut(conn) {
                    c.spec.database = Some(name);
                }
            }
        }
    }

    fn on_sidebar_action(&mut self, action: sidebar::Action) {
        use sidebar::Action;
        match action {
            Action::OpenTable { conn, schema, name } => self.open_table(conn, schema, name),
            Action::OpenStructure { conn, schema, name } => self.open_structure(conn, schema, name),
            Action::Generate { conn, schema, name, what } => {
                let sql = self.generate_script(conn, &schema, &name, what);
                if what == Script::Create {
                    if let Some(c) = self.conn(conn) {
                        c.meta().send(Tag::Sidebar, Request::ObjectDdl { schema: Some(schema), name, kind: "table".into() });
                    }
                    return;
                }
                if let Some(sql) = sql {
                    self.insert_into_editor(conn, &sql, true);
                }
            }
            Action::InsertText(text) => {
                let conn = self.sidebar.selected_conn();
                self.insert_into_editor(conn.unwrap_or(0), &text, false);
            }
            Action::SwitchDatabase { conn, name } => {
                let mut lines = vec![Line::from(format!("Switch this connection to database \"{name}\"?"))];
                if self.conn(conn).is_some_and(|c| c.in_tx) {
                    lines.push(Line::from("The open transaction will be lost."));
                }
                self.overlay = Some(Overlay::Confirm(Confirm {
                    title: "Switch database".into(),
                    lines,
                    yes: "Switch".into(),
                    danger: false,
                    on_yes: Command::SwitchDatabase(conn, name),
                    scroll: 0,
                }));
            }
            Action::LoadRelations { conn, schema } => {
                if let Some(c) = self.conn(conn) {
                    c.meta().send(Tag::Sidebar, Request::ListRelations(schema));
                }
            }
            Action::Refresh(conn) => {
                self.load_catalog(conn);
                self.toast(Level::Info, "Refreshing schema…");
            }
            Action::ShowFunction { conn, schema, name } => {
                if let Some(c) = self.conn(conn) {
                    c.meta().send(Tag::Sidebar, Request::ObjectDdl { schema: Some(schema), name, kind: "function".into() });
                }
            }
            Action::NewConnection => self.open_connection_manager(),
            Action::NewConsole { conn, scope, database } => self.open_console(conn, scope, database),
            Action::Disconnect(conn) => {
                self.sidebar.remove_connection(conn);
                self.tabs.retain(|t| t.conn != Some(conn));
                self.active = self.active.min(self.tabs.len().saturating_sub(1));
                if let Some(slot) = self.conns.get_mut(conn) {
                    *slot = None;
                }
                self.toast(Level::Info, "Disconnected");
            }
        }
    }

    fn generate_script(&self, conn: ConnId, schema: &str, name: &str, what: Script) -> Option<String> {
        let c = self.conn(conn)?;
        let backend = c.backend();
        let full = qualified(Some(schema), name, backend);
        let rel = c.catalog.as_ref().and_then(|cat| cat.find_relation(Some(schema), name));
        let cols: Vec<String> = rel.map(|r| r.columns.iter().map(|c| quote_ident(&c.name, backend)).collect()).unwrap_or_default();
        let pk: Vec<String> = rel
            .map(|r| r.columns.iter().filter(|c| c.primary_key).map(|c| quote_ident(&c.name, backend)).collect())
            .unwrap_or_default();
        let key_cond = if pk.is_empty() { "/* condition */".to_string() } else { pk.iter().map(|p| format!("{p} = ?")).collect::<Vec<_>>().join(" AND ") };
        Some(match what {
            Script::Select => {
                let list = if cols.is_empty() || cols.len() > 12 { "*".to_string() } else { cols.join(", ") };
                format!("SELECT {list}\nFROM {full}\nLIMIT 100;")
            }
            Script::Count => format!("SELECT COUNT(*) FROM {full};"),
            Script::Insert => {
                let insertable: Vec<String> = rel
                    .map(|r| r.columns.iter().filter(|c| !c.auto).map(|c| quote_ident(&c.name, backend)).collect())
                    .unwrap_or_default();
                let placeholders = vec!["?"; insertable.len().max(1)].join(", ");
                format!("INSERT INTO {full} ({})\nVALUES ({placeholders});", insertable.join(", "))
            }
            Script::Update => {
                let sets: Vec<String> = cols.iter().filter(|c| !pk.contains(c)).take(8).map(|c| format!("  {c} = ?")).collect();
                format!("UPDATE {full}\nSET\n{}\nWHERE {key_cond};", sets.join(",\n"))
            }
            Script::Delete => format!("DELETE FROM {full}\nWHERE {key_cond};"),
            Script::Drop => format!("DROP TABLE {full};"),
            Script::Create => String::new(),
        })
    }

    fn insert_into_editor(&mut self, conn: ConnId, text: &str, new_tab: bool) {
        let use_current = !new_tab
            && matches!(self.tabs.get(self.active), Some(Tab { kind: TabKind::Query(_), conn: c, .. }) if *c == Some(conn) || c.is_none());
        if use_current {
            if let Some(Tab { kind: TabKind::Query(q), .. }) = self.tabs.get_mut(self.active) {
                q.editor.insert_str(text);
                q.pane = Pane::Editor;
            }
        } else {
            let reuse = self.tabs.get(self.active).is_some_and(|t| matches!(&t.kind, TabKind::Query(q) if q.editor.is_empty()) && t.conn == Some(conn));
            if reuse {
                if let Some(Tab { kind: TabKind::Query(q), .. }) = self.tabs.get_mut(self.active) {
                    q.editor.set_text(text);
                }
            } else {
                self.new_query_tab(Some(conn), Some(text.to_string()));
            }
        }
        self.focus = Focus::Main;
    }

    // ---------------------------------------------------------------- mouse

    fn on_completion_mouse(&mut self, m: MouseEvent) -> bool {
        let r = self.areas.completion;
        if !(m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height) {
            if matches!(m.kind, MouseEventKind::Down(_)) {
                self.completion = None;
            }
            return false;
        }
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        match m.kind {
            MouseEventKind::ScrollDown => {
                self.on_completion_key(key(KeyCode::Down));
            }
            MouseEventKind::ScrollUp => {
                self.on_completion_key(key(KeyCode::Up));
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(p) = &mut self.completion {
                    let i = p.offset + (m.row - r.y) as usize;
                    if i < p.items.len() {
                        p.selected = i;
                        self.on_completion_key(key(KeyCode::Tab));
                    }
                }
            }
            _ => {}
        }
        true
    }

    /// History and Explain rows: the wheel moves the selection, a click selects, a click on the
    /// selected History entry opens it.
    fn on_list_mouse(&mut self, m: MouseEvent) -> bool {
        let r = self.areas.list;
        let inside = m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height;
        if !inside {
            return false;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        let history = matches!(tab.kind, TabKind::History(_));
        let (selected, offset, len) = match &mut tab.kind {
            TabKind::History(h) => (&mut h.selected, h.offset, h.filtered.len()),
            TabKind::Explain(x) => (&mut x.selected, x.offset, x.flat.len()),
            _ => return false,
        };
        match m.kind {
            MouseEventKind::ScrollDown => *selected = (*selected + 1).min(len.saturating_sub(1)),
            MouseEventKind::ScrollUp => *selected = selected.saturating_sub(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let i = offset + (m.row - r.y) as usize;
                let again = i == *selected;
                if i < len {
                    *selected = i;
                }
                self.focus = Focus::Main;
                if again && history {
                    self.on_history_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            _ => return false,
        }
        true
    }

    /// Dragging the borders between panes resizes them. Returns true when the event was a drag.
    fn on_drag(&mut self, m: MouseEvent) -> bool {
        let main = self.areas.main;
        let on_sidebar_edge = self.sidebar_visible && main.x > 0 && (m.column == main.x - 1 || m.column == main.x) && m.row >= main.y;
        let split = self.areas.split;
        let on_split = split.height > 0 && m.row >= split.y && m.row < split.y + split.height && m.column >= split.x && m.column < split.x + split.width;
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if on_sidebar_edge => self.dragging = Some(Drag::Sidebar),
            MouseEventKind::Down(MouseButton::Left) if on_split => self.dragging = Some(Drag::Split),
            MouseEventKind::Drag(MouseButton::Left) => match self.dragging {
                Some(Drag::Sidebar) => {
                    self.sidebar_width = m.column.saturating_sub(self.areas.sidebar.x).clamp(16, 80);
                }
                Some(Drag::Split) => {
                    let body_y = main.y;
                    let h = main.height.max(1) as u32;
                    let pct = ((m.row.saturating_sub(body_y) as u32 + 1) * 100 / h) as u16;
                    if let Some(Tab { kind: TabKind::Query(q), .. }) = self.tabs.get_mut(self.active) {
                        q.split = pct.clamp(15, 85);
                    }
                }
                None => return false,
            },
            MouseEventKind::Up(_) if self.dragging.is_some() => self.dragging = None,
            _ => return false,
        }
        true
    }

    /// Mouse input on a dialog becomes the keys it stands for, so each dialog keeps one code path:
    /// the wheel is Up/Down, a click outside is Esc, a button or a list row is Enter.
    fn on_overlay_mouse(&mut self, m: MouseEvent) {
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        let inside = |r: Rect| m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height;
        let layout = self.areas.modal;
        match m.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                if let Some(Overlay::Connect(form)) = &mut self.overlay {
                    let ev = form.handle_mouse(m);
                    return self.on_connect_event(ev);
                }
                let code = if m.kind == MouseEventKind::ScrollDown { KeyCode::Down } else { KeyCode::Up };
                self.on_overlay_key(key(code));
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if !inside(layout.area) {
                    return self.on_overlay_key(key(KeyCode::Esc));
                }
                if layout.yes.is_some_and(inside) {
                    return self.on_overlay_key(key(KeyCode::Enter));
                }
                if layout.cancel.is_some_and(inside) {
                    return self.on_overlay_key(key(KeyCode::Esc));
                }
                let picked = match &mut self.overlay {
                    Some(Overlay::Connect(form)) => {
                        let ev = form.handle_mouse(m);
                        return self.on_connect_event(ev);
                    }
                    Some(Overlay::Commands(p)) => p.select_at(m.column, m.row),
                    Some(Overlay::Themes(p, _)) => p.select_at(m.column, m.row),
                    _ => false,
                };
                if picked {
                    self.on_overlay_key(key(KeyCode::Enter));
                }
            }
            _ => {}
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if self.overlay.is_some() {
            self.on_overlay_mouse(m);
            return;
        }
        let inside = |r: Rect| m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height;
        if self.on_drag(m) {
            return;
        }
        if self.completion.is_some() && self.on_completion_mouse(m) {
            return;
        }
        if self.on_list_mouse(m) {
            return;
        }
        if let MouseEventKind::Down(MouseButton::Left) = m.kind
            && let Some((_, cmd)) = self.areas.buttons.iter().find(|(r, _)| inside(*r))
        {
            let cmd = cmd.clone();
            self.run_command(cmd);
            return;
        }
        if let MouseEventKind::Down(MouseButton::Middle) = m.kind
            && let Some((_, i)) = self.areas.header_tabs.iter().find(|(r, _)| inside(*r))
        {
            self.close_tab(*i);
            return;
        }
        if let MouseEventKind::Down(MouseButton::Left) = m.kind {
            if let Some((_, i)) = self.areas.header_tabs.iter().find(|(r, _)| inside(*r)) {
                self.active = *i;
                self.completion = None;
                return;
            }
            if let Some((_, i)) = self.areas.result_tabs.clone().iter().find(|(r, _)| inside(*r)) {
                if let Some(Tab { kind: TabKind::Query(q), .. }) = self.tabs.get_mut(self.active) {
                    q.show(*i);
                    q.pane = Pane::Results;
                    self.focus = Focus::Main;
                }
                return;
            }
            if let Some((_, i)) = self.areas.struct_tabs.clone().iter().find(|(r, _)| inside(*r)) {
                if let Some(Tab { kind: TabKind::Structure(s), .. }) = self.tabs.get_mut(self.active) {
                    s.section = StructSection::ALL[*i];
                    s.load_section();
                }
                return;
            }
        }
        if self.sidebar_visible && inside(self.areas.sidebar) {
            if matches!(m.kind, MouseEventKind::Down(_)) {
                self.focus = Focus::Sidebar;
                self.completion = None;
            }
            if let Some(a) = self.sidebar.handle_mouse(m) {
                self.on_sidebar_action(a);
            }
            return;
        }
        let editor_area = self.areas.editor;
        let grid_area = self.areas.grid;
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let tab_id = tab.id;
        let backend = Backend::Postgres;
        let ev = match &mut tab.kind {
            TabKind::Query(q) if q.showing_messages() && inside(grid_area) => {
                match m.kind {
                    MouseEventKind::ScrollDown => q.messages_scroll = q.messages_scroll.saturating_add(3),
                    MouseEventKind::ScrollUp => q.messages_scroll = q.messages_scroll.saturating_sub(3),
                    MouseEventKind::Down(_) => {
                        self.focus = Focus::Main;
                        q.pane = Pane::Results;
                    }
                    _ => {}
                }
                None
            }
            TabKind::Query(q) => {
                if inside(editor_area) {
                    if matches!(m.kind, MouseEventKind::Down(_)) {
                        self.focus = Focus::Main;
                        q.pane = Pane::Editor;
                        self.completion = None;
                    }
                    q.editor.handle_mouse(m, editor_area);
                    None
                } else if inside(grid_area) || matches!(m.kind, MouseEventKind::Drag(_)) && q.pane == Pane::Results {
                    if matches!(m.kind, MouseEventKind::Down(_)) {
                        self.focus = Focus::Main;
                        q.pane = Pane::Results;
                        self.completion = None;
                    }
                    Some(q.grid.handle_mouse(m, grid_area))
                } else {
                    None
                }
            }
            TabKind::Table(t) if inside(grid_area) || matches!(m.kind, MouseEventKind::Drag(_)) => {
                self.focus = Focus::Main;
                Some(t.grid.handle_mouse(m, grid_area))
            }
            TabKind::Structure(s) if inside(grid_area) => Some(s.grid.handle_mouse(m, grid_area)),
            TabKind::Activity(a) if inside(grid_area) => Some(a.grid.handle_mouse(m, grid_area)),
            TabKind::Text(t) => {
                match m.kind {
                    MouseEventKind::ScrollDown => t.scroll += 3,
                    MouseEventKind::ScrollUp => t.scroll = t.scroll.saturating_sub(3),
                    _ => {}
                }
                None
            }
            _ => None,
        };
        if let Some(ev) = ev {
            if inside(self.areas.main) && matches!(m.kind, MouseEventKind::Down(_)) {
                self.focus = Focus::Main;
            }
            let backend = self.tabs.get(self.active).and_then(|t| t.conn).and_then(|c| self.conn(c)).map(|c| c.backend()).unwrap_or(backend);
            self.on_grid_event(tab_id, ev, backend);
        }
    }
}

/// Saved connection names are unique, so a second connect to the same name reuses the open one.
fn open_conn_named(conns: &[Option<ConnEntry>], name: &str, backend: Backend) -> Option<ConnId> {
    conns.iter().flatten().find(|c| c.name == name && c.backend() == backend).map(|c| c.id)
}

fn original_where(t: &TableTab, row: usize, pk: &[usize], backend: Backend) -> String {
    let orig = t.original.get(&row).or_else(|| t.grid.rows().get(row));
    let cols = t.grid.columns();
    pk.iter()
        .map(|&c| {
            let col = quote_ident(&cols[c].name, backend);
            match orig.and_then(|r| r.get(c)) {
                Some(v) if !v.is_null() => format!("{col} = {}", v.to_sql_literal(backend)),
                _ => format!("{col} IS NULL"),
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// Every row in the grid in `format`, for export and "Copy results as …".
fn render_grid(grid: &GridState, format: TableFormat, table: String, backend: Backend, theme: &Theme) -> String {
    let opts = OutputOptions {
        format,
        expanded: crate::output::Expanded::Off,
        null_string: String::new(),
        max_field_width: None,
        terminal_width: usize::MAX / 4,
        color: ColorDepth::None,
        theme: Arc::new(theme.clone()),
        backend,
        table_name: Some(table),
        align_numbers: true,
        row_lines: false,
    };
    output::render(grid.columns(), grid.rows(), &opts)
}

fn pretty_value(s: &str) -> String {
    let t = s.trim_start();
    if (t.starts_with('{') || t.starts_with('['))
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(s)
            && let Ok(p) = serde_json::to_string_pretty(&v) {
                return p;
            }
    s.to_string()
}

pub fn selected_or_statement(q: &QueryTab) -> String {
    if let Some(sel) = q.editor.selected_text().filter(|s| !s.trim().is_empty()) {
        return sel;
    }
    let text = q.editor.text();
    let r = q.editor.current_statement_range();
    let stmt = text[r].to_string();
    split::split(&stmt, q.editor_backend(), ";").into_iter().next().map(|s| s.text).unwrap_or(stmt)
}

impl QueryTab {
    pub fn editor_backend(&self) -> Backend {
        self.editor.backend()
    }

    pub fn exec_offsets(&self) -> &[usize] {
        &self.exec_starts
    }
}

fn default_conn_name(spec: &ConnSpec) -> String {
    match spec.backend {
        Backend::Sqlite => spec.label(),
        _ => format!("{}@{}", spec.user_or_default(), spec.host.as_deref().unwrap_or("localhost")),
    }
}

fn read_history(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).map(|l| l.replace("<\\n>", "\n")).collect())
        .unwrap_or_default()
}

fn append_history(path: &std::path::Path, entries: &[String]) {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    if let Ok(mut f) = opts.open(path) {
        for e in entries {
            if crate::repl::editing::is_sensitive(e) {
                continue;
            }
            let _ = writeln!(f, "{}", e.replace('\n', "<\\n>"));
        }
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct UiState {
    theme: Option<String>,
}

fn ui_state_path() -> std::path::PathBuf {
    crate::config::data_dir().join("ui-state.toml")
}

fn load_ui_state() -> UiState {
    std::fs::read_to_string(ui_state_path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

fn save_ui_state(st: &UiState) {
    if let Ok(s) = toml::to_string(st) {
        let _ = crate::config::write_private(&ui_state_path(), &s);
    }
}
