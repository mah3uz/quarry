use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::conn::ConnSpec;
use crate::db::{
    Catalog, CancelHandle, Connection, DbError, ExecEvent, PlanNode, Relation, ResultSet, ServerInfo, TableDetails,
};

pub type ConnId = usize;

/// Where a reply should be routed in the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Tab(u64),
    Sidebar,
    Catalog,
    Palette,
    Silent,
}

pub enum Request {
    /// Runs statements in order; stops at the first error unless `keep_going`.
    Script { statements: Vec<String>, keep_going: bool, max_rows: usize },
    /// One statement, collected (table browser pages, counts, generated SQL).
    Query(String),
    /// Several statements inside one transaction (applying grid edits).
    Transaction(Vec<String>),
    LoadCatalog,
    ListDatabases,
    ListRelations(String),
    TableDetails { schema: Option<String>, table: String },
    ObjectDdl { schema: Option<String>, name: String, kind: String },
    Activity,
    Kill(String),
    Explain { sql: String, analyze: bool },
    ChangeDatabase(String),
}

pub enum Reply {
    StatementStart { index: usize, sql: String },
    Event { index: usize, event: ExecEvent },
    StatementDone { index: usize, result: Result<(), DbError>, elapsed: Duration, truncated: bool },
    ScriptDone { elapsed: Duration, ok: bool },
    Rows(Result<ResultSet, DbError>, Duration),
    Catalog(Result<Catalog, DbError>),
    Databases(Result<Vec<String>, DbError>),
    Relations(String, Result<Vec<Relation>, DbError>),
    Details(Result<TableDetails, DbError>),
    Ddl(Result<String, DbError>),
    Plan(Result<PlanNode, DbError>, Duration),
    Done(Result<(), DbError>),
    Committed(Result<usize, DbError>),
}

pub enum AppEvent {
    Db { conn: ConnId, tag: Tag, reply: Reply },
    State { conn: ConnId, info: ServerInfo, in_transaction: bool },
    Connected { conn: ConnId, result: Result<Box<(Connection, Option<Connection>, ConnSpec)>, String> },
}

pub type AppSender = std::sync::mpsc::Sender<crate::tui::Event>;

pub struct Worker {
    tx: mpsc::UnboundedSender<(Request, Tag)>,
    cancel: Arc<Mutex<CancelHandle>>,
    busy: Arc<AtomicBool>,
    rt: Handle,
}

impl Worker {
    pub fn spawn(rt: &Handle, id: ConnId, conn: Connection, app: AppSender) -> Worker {
        let (tx, rx) = mpsc::unbounded_channel();
        let cancel = Arc::new(Mutex::new(conn.cancel_handle()));
        let busy = Arc::new(AtomicBool::new(false));
        rt.spawn(run(id, conn, rx, app, cancel.clone(), busy.clone()));
        Worker { tx, cancel, busy, rt: rt.clone() }
    }

    pub fn send(&self, tag: Tag, req: Request) {
        let _ = self.tx.send((req, tag));
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        let handle = self.cancel.lock().unwrap().clone();
        self.rt.spawn(async move {
            let _ = handle.cancel().await;
        });
    }
}

fn post(app: &AppSender, id: ConnId, tag: Tag, reply: Reply) -> bool {
    app.send(crate::tui::Event::App(AppEvent::Db { conn: id, tag, reply })).is_ok()
}

async fn run(
    id: ConnId,
    mut conn: Connection,
    mut rx: mpsc::UnboundedReceiver<(Request, Tag)>,
    app: AppSender,
    cancel: Arc<Mutex<CancelHandle>>,
    busy: Arc<AtomicBool>,
) {
    while let Some((req, tag)) = rx.recv().await {
        busy.store(true, Ordering::Relaxed);
        let started = Instant::now();
        let alive = match req {
            Request::Script { statements, keep_going, max_rows } => {
                script(id, &mut conn, &app, tag, statements, keep_going, max_rows, &cancel).await
            }
            Request::Query(sql) => {
                let r = conn.query(&sql).await;
                post(&app, id, tag, Reply::Rows(r, started.elapsed()))
            }
            Request::Transaction(stmts) => {
                let r = transaction(&mut conn, &stmts).await;
                post(&app, id, tag, Reply::Committed(r))
            }
            Request::LoadCatalog => {
                let r = conn.load_catalog().await;
                post(&app, id, tag, Reply::Catalog(r))
            }
            Request::ListDatabases => {
                let r = conn.list_databases().await;
                post(&app, id, tag, Reply::Databases(r))
            }
            Request::ListRelations(schema) => {
                let r = conn.list_relations(&schema).await;
                post(&app, id, tag, Reply::Relations(schema, r))
            }
            Request::TableDetails { schema, table } => {
                let r = conn.table_details(schema.as_deref(), &table).await;
                post(&app, id, tag, Reply::Details(r))
            }
            Request::ObjectDdl { schema, name, kind } => {
                let r = conn.object_ddl(schema.as_deref(), &name, &kind).await;
                post(&app, id, tag, Reply::Ddl(r))
            }
            Request::Activity => {
                let r = conn.activity().await;
                post(&app, id, tag, Reply::Rows(r, started.elapsed()))
            }
            Request::Kill(sid) => {
                let r = conn.kill_session(&sid).await;
                post(&app, id, tag, Reply::Done(r))
            }
            Request::Explain { sql, analyze } => {
                let r = conn.explain(&sql, analyze).await;
                post(&app, id, tag, Reply::Plan(r, started.elapsed()))
            }
            Request::ChangeDatabase(db) => {
                let r = conn.change_database(&db).await;
                *cancel.lock().unwrap() = conn.cancel_handle();
                post(&app, id, tag, Reply::Done(r))
            }
        };
        busy.store(false, Ordering::Relaxed);
        let state = AppEvent::State { conn: id, info: conn.info().clone(), in_transaction: conn.in_transaction() };
        if !alive || app.send(crate::tui::Event::App(state)).is_err() {
            break;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn script(
    id: ConnId,
    conn: &mut Connection,
    app: &AppSender,
    tag: Tag,
    statements: Vec<String>,
    keep_going: bool,
    max_rows: usize,
    cancel: &Arc<Mutex<CancelHandle>>,
) -> bool {
    let script_start = Instant::now();
    let mut ok = true;
    for (index, sql) in statements.into_iter().enumerate() {
        if !post(app, id, tag, Reply::StatementStart { index, sql: sql.clone() }) {
            return false;
        }
        let started = Instant::now();
        let (etx, mut erx) = mpsc::channel::<ExecEvent>(64);
        let exec = async {
            let etx = etx;
            conn.execute(&sql, &etx).await
        };
        let forward = async {
            let mut rows = 0usize;
            let mut truncated = false;
            while let Some(ev) = erx.recv().await {
                if let ExecEvent::Columns(_) = ev {
                    rows = 0;
                }
                if let ExecEvent::Rows(r) = &ev {
                    if truncated {
                        continue;
                    }
                    rows += r.len();
                    if max_rows > 0 && rows > max_rows {
                        truncated = true;
                        let over = rows - max_rows;
                        let mut r = r.clone();
                        r.truncate(r.len().saturating_sub(over));
                        post(app, id, tag, Reply::Event { index, event: ExecEvent::Rows(r) });
                        let handle = cancel.lock().unwrap().clone();
                        let _ = handle.cancel().await;
                        erx.close();
                        continue;
                    }
                }
                if !post(app, id, tag, Reply::Event { index, event: ev }) {
                    break;
                }
            }
            truncated
        };
        let (mut result, truncated) = tokio::join!(exec, forward);
        if truncated && result.is_err() {
            result = Ok(());
        }
        let failed = result.is_err();
        ok &= !failed;
        post(app, id, tag, Reply::StatementDone { index, result, elapsed: started.elapsed(), truncated });
        if failed && !keep_going {
            break;
        }
    }
    post(app, id, tag, Reply::ScriptDone { elapsed: script_start.elapsed(), ok })
}

async fn transaction(conn: &mut Connection, stmts: &[String]) -> Result<usize, DbError> {
    let begin = match conn.backend() {
        crate::db::Backend::MySql => "START TRANSACTION",
        _ => "BEGIN",
    };
    let already = conn.in_transaction();
    if !already {
        conn.query(begin).await?;
    }
    let mut affected = 0usize;
    for s in stmts {
        match conn.query(s).await {
            Ok(rs) => affected += rs.summary.rows_affected.unwrap_or(0) as usize,
            Err(e) => {
                if !already {
                    let _ = conn.query("ROLLBACK").await;
                }
                return Err(e);
            }
        }
    }
    if !already {
        conn.query("COMMIT").await?;
    }
    Ok(affected)
}
