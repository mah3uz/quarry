use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use super::editing::SharedEdit;
use super::highlight::highlight;
use super::prompt::{PromptInfo, human_duration};
use super::style::Palette;
use crate::complete::{CompleteOptions, Completer, Extras, KeywordCasing};
use crate::config::Config;
use crate::conn::ConnSpec;
use crate::conn::ssh::Tunnel;
use crate::db::{Backend, Catalog, Column, Connection, DbError, ErrorKind, ExecEvent, Notice, PlanNode, Row, Summary};
use crate::output::sink::Sinks;
use crate::output::{self, Expanded, OutputOptions, TableFormat, pager};
use crate::special::favorites::Favorites;
use crate::special::{self, Special, Titled, introspect};
use crate::sql::classify;
use crate::sql::split::{self, Terminator};
use crate::theme::ColorDepth;

pub enum Flow {
    Continue,
    Quit,
    /// Switch to the full-screen TUI with the current connection.
    Tui,
}

struct Block {
    columns: Vec<Column>,
    rows: Vec<Row>,
    summary: Summary,
}

struct Outcome {
    blocks: Vec<Block>,
    notices: Vec<Notice>,
    result: Result<(), DbError>,
    truncated: Option<usize>,
    elapsed: Duration,
}

pub struct Session {
    pub rt: Handle,
    pub spec: ConnSpec,
    pub conn: Connection,
    pub tunnel: Option<Tunnel>,
    pub config: Config,
    pub palette: Palette,
    pub opts: OutputOptions,
    pub timing: bool,
    pub pager_enabled: bool,
    pub pager_cmd: Option<String>,
    pub sinks: Sinks,
    pub favorites: Favorites,
    pub readonly: bool,
    pub edit: SharedEdit,
    pub interactive: bool,
    pub prompt_format: String,
    pub last: Option<(Duration, bool)>,
    pub last_query: Option<String>,
    /// Text to pre-fill the next prompt with (after `\e`, `\format`).
    pub pending_buffer: Option<String>,
    pub continue_on_error: bool,
    /// History entries (most recent last) for `\history`; filled by the REPL loop.
    pub history_snapshot: Vec<String>,
}

impl Session {
    pub fn out(&self, text: &str) {
        if self.interactive && std::io::stdout().is_terminal() {
            pager::page_or_print(text, self.pager_cmd.as_deref(), self.pager_enabled);
        } else {
            let mut so = std::io::stdout().lock();
            let _ = so.write_all(text.as_bytes());
            let _ = so.flush();
        }
    }

    fn msg(&self, text: &str) {
        println!("{text}");
    }

    fn err(&self, text: &str) {
        eprintln!("{}", self.palette.error(text));
    }

    pub fn prompt_info(&self) -> PromptInfo {
        let info = self.conn.info();
        let backend = match self.conn.backend() {
            Backend::MySql if info.is_mariadb => "MariaDB".to_string(),
            b => b.name().to_string(),
        };
        let (user, host, port) = match self.spec.backend {
            Backend::Sqlite => (String::new(), String::new(), None),
            _ => (
                info.user
                    .as_deref()
                    .map(|u| u.split('@').next().unwrap_or(u).to_string())
                    .unwrap_or_else(|| self.spec.user_or_default()),
                info.host.clone().unwrap_or_else(|| self.spec.host_or_default().to_string()),
                info.port.or(self.spec.port),
            ),
        };
        let database = match self.spec.backend {
            Backend::Sqlite => self.spec.label(),
            _ => info.database.clone().or_else(|| self.spec.database.clone()).unwrap_or_default(),
        };
        PromptInfo {
            backend,
            user,
            host,
            port,
            database,
            in_transaction: self.conn.in_transaction(),
            readonly: self.readonly,
            last_elapsed: self.last.map(|l| l.0),
            last_ok: self.last.map(|l| l.1),
        }
    }

    fn color_opts(&self) -> OutputOptions {
        let mut o = self.opts.clone();
        o.terminal_width = terminal_width();
        o
    }

    fn plain_opts(&self) -> OutputOptions {
        let mut o = self.color_opts();
        o.color = ColorDepth::None;
        o
    }

    /// Kicks off a catalog load. SQLite loads inline (a second connection to `:memory:` would see
    /// a different database); servers load on a separate connection so the prompt stays responsive.
    pub fn refresh_catalog(&mut self) {
        let backend = self.conn.backend();
        if backend == Backend::Sqlite {
            match self.rt.block_on(self.conn.load_catalog()) {
                Ok(cat) => install_catalog(&self.edit, &self.config, &self.favorites, cat),
                Err(e) => self.err(&format!("could not load schema for completion: {e}")),
            }
            return;
        }
        let spec = self.spec.clone();
        let edit = self.edit.clone();
        let config = self.config.clone();
        let favorites = self.favorites.clone();
        self.rt.spawn(async move {
            if let Ok(mut c) = Connection::connect(&spec).await
                && let Ok(cat) = c.load_catalog().await {
                    install_catalog(&edit, &config, &favorites, cat);
                }
        });
    }

    pub fn current_catalog(&self) -> Option<Arc<Catalog>> {
        self.edit.read().unwrap().completer.as_ref().map(|c| c.catalog.clone())
    }

    /// Entry point for one submitted buffer.
    pub fn handle_input(&mut self, input: &str) -> Flow {
        let text = input.trim();
        if text.is_empty() {
            return Flow::Continue;
        }
        let backend = self.conn.backend();
        match special::parse(text, backend) {
            Some(Ok(cmd)) => return self.run_special(cmd),
            Some(Err(msg)) => {
                self.err(&msg);
                return Flow::Continue;
            }
            None => {}
        }
        if let Some(delim) = parse_delimiter_cmd(text) {
            self.set_delimiter(delim);
            return Flow::Continue;
        }
        self.run_sql(text, None);
        Flow::Continue
    }

    fn set_delimiter(&mut self, delim: String) {
        self.msg(&self.palette.muted(&format!("Changed delimiter to {delim}")));
        self.edit.write().unwrap().delimiter = delim;
    }

    fn delimiter(&self) -> String {
        self.edit.read().unwrap().delimiter.clone()
    }

    /// Splits and runs a SQL script. Returns false if a statement failed (and stopped the script).
    pub fn run_sql(&mut self, text: &str, force_vertical: Option<bool>) -> bool {
        let backend = self.conn.backend();
        let stmts = split::split(text, backend, &self.delimiter());
        self.last_query = Some(text.to_string());
        let mut ok = true;
        for st in stmts {
            let vertical = force_vertical.unwrap_or(st.terminator == Terminator::Vertical);
            if !self.run_statement(&st.text, vertical) {
                ok = false;
                if !self.continue_on_error {
                    break;
                }
            }
        }
        ok
    }

    fn confirm(&self, question: &str) -> bool {
        if !self.interactive || !std::io::stdin().is_terminal() {
            return true;
        }
        print!("{} {} ", question, self.palette.muted("[y/N]"));
        let _ = std::io::stdout().flush();
        let mut ans = String::new();
        if std::io::stdin().read_line(&mut ans).is_err() {
            return false;
        }
        matches!(ans.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }

    pub fn run_statement(&mut self, sql: &str, vertical: bool) -> bool {
        let backend = self.conn.backend();
        if self.readonly && !classify::is_read_only(sql, backend) {
            self.err("✗ Read-only mode: statement refused (use \\readonly off to allow writes)");
            self.last = Some((Duration::ZERO, false));
            return false;
        }
        if let Some(d) = classify::destructive(sql, backend, &self.config.main.destructive_warning)
            && self.interactive && std::io::stdin().is_terminal() {
                eprintln!("{} {}", self.palette.warning("⚠ Destructive statement:"), self.palette.warning(&d.reason));
                eprintln!("  {}", render_sql(&truncate_sql(sql, 400), backend, &self.palette));
                if !self.confirm(&self.palette.warning("Do you want to proceed?")) {
                    self.msg(&self.palette.muted("Aborted."));
                    return true;
                }
            }

        let outcome = self.execute_streaming(sql);
        self.after_execute(sql, &outcome);
        self.present(sql, &outcome, vertical);
        outcome.result.is_ok()
    }

    fn after_execute(&mut self, sql: &str, outcome: &Outcome) {
        let backend = self.conn.backend();
        self.last = Some((outcome.elapsed, outcome.result.is_ok()));
        if self.config.main.log_queries {
            log_query(&self.config.log_path(), sql, outcome);
        }
        if outcome.result.is_err() {
            return;
        }
        if classify::use_database(sql, backend).is_some()
            && let Some(db) = self.conn.info().database.clone() {
                self.spec.database = Some(db);
            }
        if self.config.main.auto_refresh_catalog && classify::changes_schema(sql, backend) {
            self.refresh_catalog();
        }
    }

    fn execute_streaming(&mut self, sql: &str) -> Outcome {
        let row_limit = if self.interactive && std::io::stdin().is_terminal() { self.config.main.row_limit } else { 0 };
        let cancel = self.conn.cancel_handle();
        let palette = self.palette.clone();
        let conn = &mut self.conn;
        let started = Instant::now();
        self.rt.block_on(async move {
            let (tx, mut rx) = mpsc::channel::<ExecEvent>(32);
            let exec = async move {
                let tx = tx;
                conn.execute(sql, &tx).await
            };
            tokio::pin!(exec);
            let ctrl_c = tokio::signal::ctrl_c();
            tokio::pin!(ctrl_c);
            let mut blocks: Vec<Block> = Vec::new();
            let mut notices = Vec::new();
            let mut result = None;
            let mut rx_open = true;
            let mut cancelled = false;
            let mut discard = false;
            let mut truncated = None;
            let mut asked = false;
            let mut waited = Duration::ZERO;
            loop {
                tokio::select! {
                    r = &mut exec, if result.is_none() => result = Some(r),
                    ev = rx.recv(), if rx_open => match ev {
                        None => rx_open = false,
                        Some(ExecEvent::Columns(columns)) => blocks.push(Block { columns, rows: Vec::new(), summary: Summary::default() }),
                        Some(ExecEvent::Rows(mut rows)) => {
                            if discard {
                                continue;
                            }
                            if blocks.is_empty() {
                                blocks.push(Block { columns: Vec::new(), rows: Vec::new(), summary: Summary::default() });
                            }
                            let b = blocks.last_mut().unwrap();
                            b.rows.append(&mut rows);
                            if row_limit > 0 && !asked && b.rows.len() > row_limit {
                                asked = true;
                                let q = format!(
                                    "{} {}",
                                    palette.warning(&format!("The result has more than {row_limit} rows.")),
                                    "Fetch and show all of them?"
                                );
                                let asked_at = Instant::now();
                                let fetch_all = ask_blocking(&q, &palette);
                                waited += asked_at.elapsed();
                                if !fetch_all {
                                    b.rows.truncate(row_limit);
                                    truncated = Some(row_limit);
                                    discard = true;
                                    let _ = cancel.cancel().await;
                                    rx.close();
                                }
                            }
                        }
                        Some(ExecEvent::Done(s)) => {
                            match blocks.last_mut() {
                                Some(b) if b.summary == Summary::default() => b.summary = s,
                                _ => blocks.push(Block { columns: Vec::new(), rows: Vec::new(), summary: s }),
                            }
                        }
                        Some(ExecEvent::Notice(n)) => notices.push(n),
                    },
                    _ = &mut ctrl_c, if !cancelled => {
                        cancelled = true;
                        let _ = cancel.cancel().await;
                    }
                }
                if result.is_some() && !rx_open {
                    break;
                }
            }
            let mut result = result.unwrap_or(Ok(()));
            if truncated.is_some()
                && let Err(e) = &result
                    && (e.kind == ErrorKind::Cancelled || e.message.to_ascii_lowercase().contains("cancel") || e.message.contains("interrupt")) {
                        result = Ok(());
                    }
            Outcome { blocks, notices, result, truncated, elapsed: started.elapsed().saturating_sub(waited) }
        })
    }

    fn present(&mut self, sql: &str, o: &Outcome, vertical: bool) {
        let backend = self.conn.backend();
        let mut color = self.color_opts();
        let mut plain = self.plain_opts();
        if vertical {
            color.expanded = Expanded::On;
            plain.expanded = Expanded::On;
        }
        let mut screen = String::new();
        let mut file = String::new();
        for n in &o.notices {
            let line = format!("{}: {}\n", n.severity, n.message);
            screen.push_str(&self.palette.warning(&line));
            file.push_str(&line);
        }
        let n_blocks = o.blocks.len();
        let failed = o.result.is_err();
        for (i, b) in o.blocks.iter().enumerate() {
            if failed && b.rows.is_empty() {
                continue;
            }
            let elapsed = (self.timing && i + 1 == n_blocks).then_some(o.elapsed);
            if !b.columns.is_empty() {
                screen.push_str(&output::render(&b.columns, &b.rows, &color));
                if self.sinks.is_active() {
                    file.push_str(&output::render(&b.columns, &b.rows, &plain));
                }
            }
            if failed || (self.opts.format.is_machine() && !self.interactive) {
                continue;
            }
            let status = output::render_status(&b.summary, b.rows.len(), elapsed, &color);
            if !status.is_empty() {
                screen.push_str(&status);
                if !status.ends_with('\n') {
                    screen.push('\n');
                }
            }
        }
        if let Some(n) = o.truncated {
            screen.push_str(&self.palette.muted(&format!("(showing the first {n} rows)\n")));
        }
        if let Err(e) = &o.result {
            screen.push_str(&format_error(e, sql, backend, &self.palette));
        } else if o.blocks.is_empty() && self.interactive && !self.opts.format.is_machine() {
            let t = if self.timing { format!(" · {}", human_duration(o.elapsed)) } else { String::new() };
            screen.push_str(&self.palette.muted(&format!("OK{t}\n")));
        }

        let redirected = self.sinks.is_redirected_once();
        if let Err(e) = self.sinks.write_result(&file) {
            self.err(&format!("could not write output: {e}"));
        }
        if redirected {
            if let Err(e) = &o.result {
                eprint!("{}", format_error(e, sql, backend, &self.palette));
            }
            return;
        }
        if o.result.is_err() && !self.interactive {
            eprint!("{screen}");
        } else {
            self.out(&screen);
        }
    }

    fn print_titled(&mut self, items: Vec<Titled>) {
        let color = self.color_opts();
        let plain = self.plain_opts();
        let backend = self.conn.backend();
        let mut screen = String::new();
        let mut file = String::new();
        for t in items {
            if let Some(title) = &t.title {
                screen.push_str(&self.palette.accent(title));
                screen.push('\n');
                file.push_str(title);
                file.push('\n');
            }
            if let Some(text) = &t.text {
                screen.push_str(&render_sql(text, backend, &self.palette));
                screen.push('\n');
                file.push_str(text);
                file.push('\n');
            } else if !t.result.columns.is_empty() {
                screen.push_str(&output::render(&t.result.columns, &t.result.rows, &color));
                file.push_str(&output::render(&t.result.columns, &t.result.rows, &plain));
            }
            if let Some(f) = &t.footer {
                screen.push_str(&self.palette.muted(f));
                screen.push('\n');
                file.push_str(f);
                file.push('\n');
            }
        }
        let redirected = self.sinks.is_redirected_once();
        let _ = self.sinks.write_result(&file);
        if !redirected {
            self.out(&screen);
        }
    }

    pub fn run_special(&mut self, cmd: Special) -> Flow {
        let started = Instant::now();
        match self.rt.block_on(introspect::run(&mut self.conn, &cmd)) {
            Ok(Some(items)) => {
                self.last = Some((started.elapsed(), true));
                self.print_titled(items);
                return Flow::Continue;
            }
            Ok(None) => {}
            Err(e) => {
                self.err(&format!("✗ {e}"));
                self.last = Some((started.elapsed(), false));
                return Flow::Continue;
            }
        }
        let p = self.palette.clone();
        match cmd {
            Special::Quit => return Flow::Quit,
            Special::Tui => return Flow::Tui,
            Special::Help(topic) => self.print_help(topic.as_deref()),
            Special::Connect { target } => self.connect_to(target),
            Special::Expanded(mode) => {
                let next = mode.unwrap_or(match self.opts.expanded {
                    Expanded::Off => Expanded::On,
                    Expanded::On => Expanded::Auto,
                    Expanded::Auto => Expanded::Off,
                });
                self.opts.expanded = next;
                self.msg(&p.muted(&format!("Expanded display is {}.", match next {
                    Expanded::On => "on",
                    Expanded::Off => "off",
                    Expanded::Auto => "used automatically",
                })));
            }
            Special::Timing(v) => {
                self.timing = v.unwrap_or(!self.timing);
                self.msg(&p.muted(&format!("Timing is {}.", if self.timing { "on" } else { "off" })));
            }
            Special::TableFormat(None) => {
                let names: Vec<String> = TableFormat::ALL
                    .iter()
                    .map(|(n, f)| if *f == self.opts.format { p.accent(n) } else { n.to_string() })
                    .collect();
                self.msg(&format!("Formats: {}", names.join(", ")));
            }
            Special::TableFormat(Some(f)) => {
                self.opts.format = f;
                self.msg(&p.muted(&format!("Output format set to {}.", f.name())));
            }
            Special::Pager(cmd) => {
                self.pager_enabled = true;
                if let Some(c) = cmd {
                    self.msg(&p.muted(&format!("PAGER set to {c}.")));
                    self.pager_cmd = Some(c);
                } else {
                    self.msg(&p.muted("Pager enabled."));
                }
            }
            Special::NoPager => {
                self.pager_enabled = false;
                self.msg(&p.muted("Pager disabled."));
            }
            Special::Tee { path, overwrite } => match self.sinks.tee(&path, overwrite) {
                Ok(()) => self.msg(&p.muted(&format!("Logging results to {path}."))),
                Err(e) => self.err(&format!("✗ {path}: {e}")),
            },
            Special::NoTee => {
                self.sinks.notee();
                self.msg(&p.muted("Stopped logging results."));
            }
            Special::Once { path, overwrite } => {
                if let Err(e) = self.sinks.once(&path, overwrite) {
                    self.err(&format!("✗ {path}: {e}"));
                }
            }
            Special::PipeOnce { command } => self.sinks.pipe_once(&command),
            Special::Edit { file, query } => self.edit_external(file, query),
            Special::Source { path } => self.source_file(&path),
            Special::Clip { query } => {
                let text = query.or_else(|| self.last_query.clone()).unwrap_or_default();
                match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
                    Ok(()) => self.msg(&p.muted("Copied to clipboard.")),
                    Err(e) => self.err(&format!("✗ clipboard unavailable: {e}")),
                }
            }
            Special::Watch { seconds, clear, query } => {
                let q = query.or_else(|| self.last_query.clone());
                match q {
                    Some(q) => self.watch(&q, seconds, clear),
                    None => self.err("✗ nothing to watch: give a query or run one first"),
                }
            }
            Special::Favorite { name: None, .. } => self.list_favorites(),
            Special::Favorite { name: Some(name), args } => match self.favorites.expand(&name, &args) {
                Ok(sql) => {
                    if !self.config.main.less_chatty {
                        self.msg(&format!("{} {}", p.muted(">"), render_sql(&sql, self.conn.backend(), &p)));
                    }
                    self.run_sql(&sql, None);
                }
                Err(e) => self.err(&format!("✗ {e}")),
            },
            Special::FavoriteSave { name, query } => {
                self.favorites.queries.insert(name.clone(), query);
                match self.favorites.save() {
                    Ok(()) => {
                        self.msg(&p.success(&format!("Saved favorite '{name}'.")));
                        self.sync_extras();
                    }
                    Err(e) => self.err(&format!("✗ {e}")),
                }
            }
            Special::FavoriteDelete { name } => {
                if self.favorites.queries.remove(&name).is_none() {
                    self.err(&format!("✗ no favorite named '{name}'"));
                } else if let Err(e) = self.favorites.save() {
                    self.err(&format!("✗ {e}"));
                } else {
                    self.msg(&p.muted(&format!("Deleted favorite '{name}'.")));
                    self.sync_extras();
                }
            }
            Special::Refresh => {
                self.refresh_catalog();
                self.msg(&p.muted("Refreshing completions in the background…"));
            }
            Special::System { command } => {
                let status = std::process::Command::new("sh").arg("-c").arg(&command).status();
                if let Err(e) = status {
                    self.err(&format!("✗ {e}"));
                }
            }
            Special::Echo(text) => self.msg(&text),
            Special::Prompt(fmt) => {
                self.prompt_format = fmt.unwrap_or_else(|| "auto".into());
                self.msg(&p.muted(&format!("Prompt set to {:?}.", self.prompt_format)));
            }
            Special::Delimiter(d) => self.set_delimiter(d),
            Special::Theme(None) => {
                let names = crate::theme::list_all(&self.config.themes_dir());
                let names: Vec<String> =
                    names.iter().map(|n| if *n == self.palette.theme.name { p.accent(n) } else { n.clone() }).collect();
                self.msg(&format!("Themes: {}", names.join(", ")));
            }
            Special::Theme(Some(name)) => match crate::theme::load(&name, &self.config.themes_dir()) {
                Ok(theme) => {
                    self.set_theme(theme);
                    self.msg(&self.palette.success(&format!("Theme set to {name}.")));
                }
                Err(e) => self.err(&format!("✗ {e}")),
            },
            Special::Format { query } => {
                let q = query.or_else(|| self.last_query.clone()).unwrap_or_default();
                let formatted = crate::sql::format::format_sql(&q, self.conn.backend(), &Default::default());
                self.msg(&render_sql(&formatted, self.conn.backend(), &p));
                self.pending_buffer = Some(formatted);
            }
            Special::Explain { analyze, query } => {
                let started = Instant::now();
                match self.rt.block_on(self.conn.explain(&query, analyze)) {
                    Ok(plan) => {
                        let mut s = String::new();
                        render_plan(&plan, "", true, true, &p, &mut s);
                        if self.timing {
                            s.push_str(&p.muted(&format!("{}\n", human_duration(started.elapsed()))));
                        }
                        self.out(&s);
                    }
                    Err(e) => eprint!("{}", format_error(&e, &query, self.conn.backend(), &p)),
                }
            }
            Special::ReadOnly(v) => self.set_readonly(v.unwrap_or(!self.readonly)),
            Special::Export { format, path, query } => self.export(format, &path, &query),
            Special::LoadExtension { path } => match self.rt.block_on(self.conn.load_extension(&path)) {
                Ok(()) => self.msg(&p.muted(&format!("Loaded {path}."))),
                Err(e) => self.err(&format!("✗ {e}")),
            },
            Special::History(n) => {
                let n = n.unwrap_or(20);
                let start = self.history_snapshot.len().saturating_sub(n);
                let mut s = String::new();
                for (i, h) in self.history_snapshot[start..].iter().enumerate() {
                    s.push_str(&p.muted(&format!("{:>5}  ", start + i + 1)));
                    s.push_str(&render_sql(h, self.conn.backend(), &p));
                    s.push('\n');
                }
                self.out(&s);
            }
            Special::Llm { question } => self.ask_llm(&question),
            other => self.err(&format!("✗ {other:?} is not available here")),
        }
        Flow::Continue
    }

    fn ask_llm(&mut self, question: &str) {
        let p = self.palette.clone();
        let catalog = self.current_catalog();
        let version = self.conn.info().version.clone();
        let llm = self.config.llm.clone();
        if let Err(e) = crate::llm::check(&llm) {
            self.err(&format!("✗ {e}"));
            return;
        }
        eprintln!("{}", p.muted(&format!("Asking {}…", crate::llm::describe(&llm))));
        let req = crate::llm::Request { question, backend: self.conn.backend(), server_version: &version, catalog: catalog.as_deref() };
        match crate::llm::ask(&llm, &req) {
            Ok(a) if a.sql.is_empty() => self.msg(&a.explanation),
            Ok(a) => {
                if !a.explanation.is_empty() {
                    self.msg(&p.muted(&a.explanation));
                }
                self.msg(&p.muted("Review the query below and press Enter to run it."));
                self.pending_buffer = Some(a.sql);
            }
            Err(e) => self.err(&format!("✗ {e}")),
        }
    }

    pub fn set_theme(&mut self, theme: crate::theme::Theme) {
        self.palette = Palette::new(theme.clone(), self.palette.depth);
        self.opts.theme = Arc::new(theme);
        self.edit.write().unwrap().palette = self.palette.clone();
    }

    fn set_readonly(&mut self, on: bool) {
        let sql = match (self.conn.backend(), on) {
            (Backend::Postgres, true) => "SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY",
            (Backend::Postgres, false) => "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE",
            (Backend::MySql, true) => "SET SESSION TRANSACTION READ ONLY",
            (Backend::MySql, false) => "SET SESSION TRANSACTION READ WRITE",
            (Backend::Sqlite, true) => "PRAGMA query_only = ON",
            (Backend::Sqlite, false) => "PRAGMA query_only = OFF",
        };
        match self.rt.block_on(self.conn.query(sql)) {
            Ok(_) => {
                self.readonly = on;
                self.msg(&self.palette.muted(&format!("Read-only mode {}.", if on { "on" } else { "off" })));
            }
            Err(e) => self.err(&format!("✗ {e}")),
        }
    }

    fn sync_extras(&self) {
        let mut st = self.edit.write().unwrap();
        if let Some(c) = &st.completer {
            let mut extras = c.extras.clone();
            extras.favorites = self.favorites.queries.keys().cloned().collect();
            let nc = Completer::new(c.backend, c.catalog.clone(), c.options.clone(), extras);
            st.completer = Some(Arc::new(nc));
        }
    }

    fn list_favorites(&self) {
        let p = &self.palette;
        if self.favorites.queries.is_empty() {
            self.msg(&p.muted("No favorite queries. Save one with \\fs name query"));
            return;
        }
        let mut s = String::new();
        for (name, q) in &self.favorites.queries {
            s.push_str(&p.accent(name));
            s.push('\n');
            s.push_str("  ");
            s.push_str(&render_sql(q, self.conn.backend(), p));
            s.push('\n');
        }
        self.out(&s);
    }

    fn print_help(&self, topic: Option<&str>) {
        let p = &self.palette;
        let backend = self.conn.backend();
        let mut s = String::new();
        let topic = topic.map(|t| t.to_ascii_lowercase());
        let mut last_cat = None;
        for spec in special::registry() {
            if !spec.backends.is_empty() && !spec.backends.contains(&backend) {
                continue;
            }
            if let Some(t) = &topic {
                let hit = spec.names.iter().any(|n| n.to_ascii_lowercase().contains(t.as_str()))
                    || spec.description.to_ascii_lowercase().contains(t.as_str());
                if !hit {
                    continue;
                }
            }
            if last_cat != Some(spec.category) {
                s.push_str(&p.accent(&format!("\n{:?}\n", spec.category)));
                last_cat = Some(spec.category);
            }
            let names = spec.names.join(", ");
            s.push_str(&format!(
                "  {:<28} {}\n",
                p.paint(p.fg(p.theme.accent2), &pad(spec.syntax, 28)),
                spec.description
            ));
            if spec.names.len() > 1 {
                s.push_str(&p.muted(&format!("  {:<28} aliases: {names}\n", "")));
            }
        }
        if topic.is_none() {
            s.push_str(&p.muted(
                "\nKeys: Tab complete · Ctrl-R history search · Alt-Enter run now · F2 smart completion · F3 multi-line · F4 vi/emacs · Ctrl-D quit\n",
            ));
        }
        self.out(&s);
    }

    fn connect_to(&mut self, target: Option<String>) {
        let p = self.palette.clone();
        let Some(target) = target else {
            let db = self.conn.info().database.clone().unwrap_or_default();
            self.msg(&format!("You are connected to {} {}", p.accent(&self.spec.display_url()), p.muted(&format!("(database {db})"))));
            return;
        };
        let new_spec = if target.contains("://") || crate::conn::url::looks_like_sqlite_path(&target)
            || (self.conn.backend() == Backend::Sqlite)
        {
            match ConnSpec::parse(&target).or_else(|_| Ok::<_, String>(ConnSpec::sqlite(crate::conn::url::expand_tilde(&target)))) {
                Ok(mut s) => {
                    if s.backend == self.spec.backend && s.password.is_none() && s.user == self.spec.user {
                        s.password = self.spec.password.clone();
                    }
                    Some(s)
                }
                Err(e) => {
                    self.err(&format!("✗ {e}"));
                    return;
                }
            }
        } else {
            None
        };
        match new_spec {
            Some(spec) => match self.rt.block_on(crate::cli::open(spec, None, false, true)) {
                Ok(opened) => {
                    self.conn = opened.conn;
                    self.spec = opened.spec;
                    self.tunnel = opened.tunnel;
                    self.edit.write().unwrap().backend = self.conn.backend();
                    self.opts.backend = self.conn.backend();
                    self.msg(&p.success(&format!("Connected to {}", self.spec.display_url())));
                    self.refresh_catalog();
                }
                Err(e) => self.err(&format!("✗ {e:#}")),
            },
            None => match self.rt.block_on(self.conn.change_database(&target)) {
                Ok(()) => {
                    self.spec.database = Some(target.clone());
                    self.msg(&p.success(&format!("You are now connected to database \"{target}\"")));
                    self.refresh_catalog();
                }
                Err(e) => self.err(&format!("✗ {e}")),
            },
        }
    }

    fn edit_external(&mut self, file: Option<String>, query: Option<String>) {
        let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
        let (path, temp) = match &file {
            Some(f) => (crate::conn::url::expand_tilde(f), false),
            None => {
                let path = std::env::temp_dir().join(format!("quarry-{}.sql", std::process::id()));
                let initial = query.or_else(|| self.last_query.clone()).unwrap_or_default();
                if let Err(e) = write_private(&path, &initial) {
                    self.err(&format!("✗ {e}"));
                    return;
                }
                (path, true)
            }
        };
        let mut parts = shell_words::split(&editor).unwrap_or_else(|_| vec![editor.clone()]);
        let prog = parts.remove(0);
        let status = std::process::Command::new(prog).args(parts).arg(&path).status();
        match status {
            Ok(s) if s.success() => match std::fs::read_to_string(&path) {
                Ok(text) => self.pending_buffer = Some(text.trim_end().to_string()),
                Err(e) => self.err(&format!("✗ {e}")),
            },
            Ok(s) => self.err(&format!("✗ editor exited with {s}")),
            Err(e) => self.err(&format!("✗ could not start editor '{editor}': {e}")),
        }
        if temp {
            let _ = std::fs::remove_file(&path);
        }
    }

    pub fn source_file(&mut self, path: &str) {
        let p = crate::conn::url::expand_tilde(path);
        match std::fs::read_to_string(&p) {
            Ok(text) => {
                let saved = self.last_query.clone();
                self.run_sql(&text, None);
                self.last_query = saved;
            }
            Err(e) => self.err(&format!("✗ {}: {e}", p.display())),
        }
    }

    fn watch(&mut self, query: &str, seconds: f64, clear: bool) {
        let interval = Duration::from_secs_f64(seconds.max(0.1));
        let p = self.palette.clone();
        loop {
            if clear {
                print!("\x1b[2J\x1b[H");
            }
            println!(
                "{}",
                p.muted(&format!("Every {seconds}s · {} · Ctrl-C to stop", chrono::Local::now().format("%H:%M:%S")))
            );
            let saved = std::mem::replace(&mut self.pager_enabled, false);
            let ok = self.run_sql(query, None);
            self.pager_enabled = saved;
            if !ok {
                break;
            }
            let stop = self.rt.block_on(async {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => false,
                    _ = tokio::signal::ctrl_c() => true,
                }
            });
            if stop {
                break;
            }
        }
    }

    fn export(&mut self, format: TableFormat, path: &str, query: &str) {
        let started = Instant::now();
        let outcome = self.execute_streaming(query);
        if let Err(e) = &outcome.result {
            eprint!("{}", format_error(e, query, self.conn.backend(), &self.palette));
            return;
        }
        let mut opts = self.plain_opts();
        opts.format = format;
        opts.expanded = Expanded::Off;
        opts.max_field_width = None;
        let mut text = String::new();
        let mut rows = 0;
        for b in outcome.blocks.iter().filter(|b| !b.columns.is_empty()) {
            text.push_str(&output::render(&b.columns, &b.rows, &opts));
            rows += b.rows.len();
        }
        let dest = crate::conn::url::expand_tilde(path);
        match write_private(&dest, &text) {
            Ok(()) => self.msg(&self.palette.success(&format!(
                "Exported {rows} rows to {} ({}) in {}",
                dest.display(),
                format.name(),
                human_duration(started.elapsed())
            ))),
            Err(e) => self.err(&format!("✗ {}: {e}", dest.display())),
        }
    }
}

fn log_query(path: &std::path::Path, sql: &str, o: &Outcome) {
    if crate::repl::editing::is_sensitive(sql) {
        return;
    }
    let _ = ensure_dir(path);
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    if let Ok(mut f) = opts.open(path) {
        let status = match &o.result {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("error: {}", e.message.replace('\n', " ")),
        };
        let _ = writeln!(
            f,
            "{}\t{}\t{}\t{}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            human_duration(o.elapsed),
            status,
            sql.replace('\n', " ")
        );
    }
}

pub fn install_catalog(edit: &SharedEdit, config: &Config, favorites: &Favorites, catalog: Catalog) {
    let backend = catalog.backend;
    let casing = match config.main.keyword_casing.to_ascii_lowercase().as_str() {
        "upper" => KeywordCasing::Upper,
        "lower" => KeywordCasing::Lower,
        _ => KeywordCasing::Auto,
    };
    let mut st = edit.write().unwrap();
    let options = CompleteOptions {
        keyword_casing: casing,
        smart: st.smart_completion,
        join_suggestions: config.main.join_suggestions,
        ..Default::default()
    };
    let extras = Extras {
        specials: special::registry()
            .iter()
            .filter(|s| s.backends.is_empty() || s.backends.contains(&backend))
            .flat_map(|s| s.names.iter().map(move |n| (n.to_string(), s.description.to_string())))
            .collect(),
        favorites: favorites.queries.keys().cloned().collect(),
        ..Default::default()
    };
    st.completer = Some(Arc::new(Completer::new(backend, Arc::new(catalog), options, extras)));
}

fn ask_blocking(question: &str, p: &Palette) -> bool {
    eprint!("{question} {} ", p.muted("[y/N]"));
    let _ = std::io::stderr().flush();
    let mut ans = String::new();
    std::io::stdin().read_line(&mut ans).is_ok() && matches!(ans.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn parse_delimiter_cmd(text: &str) -> Option<String> {
    let rest = text.strip_prefix("delimiter ").or_else(|| text.strip_prefix("DELIMITER "))?;
    let d = rest.trim();
    (!d.is_empty() && !d.contains(char::is_whitespace)).then(|| d.to_string())
}

fn pad(s: &str, w: usize) -> String {
    let width = unicode_width::UnicodeWidthStr::width(s);
    if width >= w { s.to_string() } else { format!("{s}{}", " ".repeat(w - width)) }
}

pub fn terminal_width() -> usize {
    crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(120).max(20)
}

fn truncate_sql(sql: &str, max: usize) -> String {
    if sql.len() <= max {
        return sql.to_string();
    }
    let mut end = max;
    while !sql.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &sql[..end])
}

pub fn render_sql(sql: &str, backend: Backend, p: &Palette) -> String {
    if p.depth == ColorDepth::None {
        return sql.to_string();
    }
    let styled = highlight(sql, usize::MAX, backend, p);
    styled.buffer.iter().map(|(style, text)| style.paint(text).to_string()).collect()
}

pub fn format_error(e: &DbError, sql: &str, backend: Backend, p: &Palette) -> String {
    let mut s = String::new();
    let head = match (&e.code, e.kind) {
        (_, ErrorKind::Cancelled) => "✗ Cancelled".to_string(),
        (Some(code), _) => format!("✗ ERROR {code}"),
        (None, _) => "✗ ERROR".to_string(),
    };
    s.push_str(&p.error(&head));
    if e.kind != ErrorKind::Cancelled || !e.message.is_empty() {
        s.push_str("  ");
        s.push_str(&e.message);
    }
    s.push('\n');
    if let Some(pos) = e.position.filter(|p| *p > 0) {
        let char_idx = pos - 1;
        let byte_idx = sql.char_indices().nth(char_idx).map(|(i, _)| i).unwrap_or(sql.len());
        let line_start = sql[..byte_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_end = sql[byte_idx..].find('\n').map(|i| byte_idx + i).unwrap_or(sql.len());
        let line_no = sql[..line_start].matches('\n').count() + 1;
        let gutter = format!("{line_no:>4} │ ");
        s.push_str(&p.muted(&gutter));
        s.push_str(&render_sql(&sql[line_start..line_end], backend, p));
        s.push('\n');
        let col = unicode_width::UnicodeWidthStr::width(&sql[line_start..byte_idx]);
        s.push_str(&" ".repeat(unicode_width::UnicodeWidthStr::width(gutter.as_str()) + col));
        s.push_str(&p.error("^"));
        s.push('\n');
    }
    if let Some(d) = &e.detail {
        s.push_str(&p.muted("  detail: "));
        s.push_str(d);
        s.push('\n');
    }
    if let Some(h) = &e.hint {
        s.push_str(&p.info("  hint: "));
        s.push_str(h);
        s.push('\n');
    }
    s
}

pub fn render_plan(node: &PlanNode, prefix: &str, last: bool, root: bool, p: &Palette, out: &mut String) {
    let branch = if root { "" } else if last { "└─ " } else { "├─ " };
    out.push_str(&p.muted(prefix));
    out.push_str(&p.muted(branch));
    out.push_str(&p.paint(p.fg(p.theme.accent).bold(), &node.label));
    let mut facts = Vec::new();
    if let Some(c) = node.total_cost {
        facts.push(format!("cost {c:.2}"));
    }
    if let Some(r) = node.plan_rows {
        facts.push(format!("rows {r:.0}"));
    }
    if let Some(t) = node.actual_time_ms {
        facts.push(format!("actual {t:.3} ms"));
    }
    if let Some(r) = node.actual_rows {
        facts.push(format!("actual rows {r:.0}"));
    }
    if let Some(l) = node.loops.filter(|l| *l > 1.0) {
        facts.push(format!("loops {l:.0}"));
    }
    if !facts.is_empty() {
        out.push_str(&p.muted(&format!("  ({})", facts.join(" · "))));
    }
    out.push('\n');
    let child_prefix = if root { prefix.to_string() } else { format!("{prefix}{}", if last { "   " } else { "│  " }) };
    for (k, v) in &node.details {
        out.push_str(&p.muted(&child_prefix));
        out.push_str(&p.muted(if node.children.is_empty() { "   " } else { "│  " }));
        out.push_str(&p.paint(p.fg(p.theme.muted), &format!("{k}: ")));
        out.push_str(v);
        out.push('\n');
    }
    let n = node.children.len();
    for (i, c) in node.children.iter().enumerate() {
        render_plan(c, &child_prefix, i + 1 == n, false, p, out);
    }
}

pub fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(text.as_bytes())
}

pub fn history_path(config: &Config) -> PathBuf {
    config.history_path()
}

pub fn ensure_dir(p: &std::path::Path) -> Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn plain() -> Palette {
        Palette::new(Theme::default(), ColorDepth::None)
    }

    #[test]
    fn error_caret_points_at_reported_position() {
        let mut e = DbError::query("syntax error at or near \"FORM\"");
        e.code = Some("42601".into());
        e.position = Some(10);
        let out = format_error(&e, "select 1\nFORM t", Backend::Postgres, &plain());
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].contains("42601"));
        assert_eq!(lines[1], "   2 │ FORM t");
        assert_eq!(lines[2].find('^'), Some(7), "{out}");
    }

    #[test]
    fn delimiter_command() {
        assert_eq!(parse_delimiter_cmd("delimiter //").as_deref(), Some("//"));
        assert_eq!(parse_delimiter_cmd("delimiter"), None);
    }

    #[test]
    fn plan_tree_renders_hierarchy() {
        let plan = PlanNode {
            label: "Hash Join".into(),
            children: vec![
                PlanNode { label: "Seq Scan on a".into(), ..Default::default() },
                PlanNode { label: "Hash".into(), children: vec![PlanNode { label: "Seq Scan on b".into(), ..Default::default() }], ..Default::default() },
            ],
            ..Default::default()
        };
        let mut s = String::new();
        render_plan(&plan, "", true, true, &plain(), &mut s);
        assert_eq!(s, "Hash Join\n├─ Seq Scan on a\n└─ Hash\n   └─ Seq Scan on b\n");
    }
}
