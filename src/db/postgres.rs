use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::{Pin, pin};

use futures_util::{FutureExt, StreamExt};
use serde_json::{Map, Value as Json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_postgres::error::ErrorPosition;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{AsyncMessage, CancelToken, Client, NoTls, SimpleQueryMessage, SimpleQueryStream};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::*;
use crate::conn::{ConnSpec, SslMode};
use crate::sql::classify::{self, TxEffect};
use crate::sql::lexer::tokenize;

const B: Backend = Backend::Postgres;

#[derive(Clone)]
enum PgTls {
    Plain,
    Rustls(MakeRustlsConnect),
}

pub struct PgConn {
    client: Client,
    spec: Box<ConnSpec>,
    info: ServerInfo,
    tls: PgTls,
    notices: mpsc::UnboundedReceiver<Notice>,
    in_tx: bool,
}

#[derive(Clone)]
pub struct PgCancel {
    token: CancelToken,
    tls: PgTls,
}

impl PgCancel {
    pub async fn cancel(&self) -> DbResult<()> {
        let res = match &self.tls {
            PgTls::Plain => self.token.cancel_query(NoTls).await,
            PgTls::Rustls(t) => self.token.cancel_query(t.clone()).await,
        };
        res.map_err(pg_err)
    }
}

fn pg_err(e: tokio_postgres::Error) -> DbError {
    if let Some(db) = e.as_db_error() {
        let code = db.code().code();
        let kind = match code {
            "28P01" | "28000" => ErrorKind::Auth,
            "57014" => ErrorKind::Cancelled,
            "57P01" | "57P02" | "57P03" => ErrorKind::Connection,
            c if c.starts_with("08") => ErrorKind::Connection,
            _ => ErrorKind::Query,
        };
        let mut message = db.message().to_string();
        if let Some(w) = db.where_() {
            message = format!("{message}\nCONTEXT: {w}");
        }
        return DbError {
            kind,
            message,
            code: Some(code.to_string()),
            detail: db.detail().map(str::to_string),
            hint: db.hint().map(str::to_string),
            position: match db.position() {
                Some(ErrorPosition::Original(p)) => Some(*p as usize),
                _ => None,
            },
        };
    }
    let mut message = e.to_string();
    let mut io = false;
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        io |= s.is::<std::io::Error>();
        message = format!("{message}: {s}");
        src = s.source();
    }
    let lower = message.to_ascii_lowercase();
    let kind = if lower.contains("password authentication failed")
        || lower.contains("no password supplied")
        || lower.contains("password missing")
    {
        ErrorKind::Auth
    } else if io || e.is_closed() {
        ErrorKind::Connection
    } else {
        ErrorKind::Other
    };
    DbError::new(kind, message)
}

fn connect_err(e: tokio_postgres::Error) -> DbError {
    let mut err = pg_err(e);
    if !matches!(err.kind, ErrorKind::Auth) {
        err.kind = ErrorKind::Connection;
    }
    err
}

fn notice(n: &tokio_postgres::error::DbError) -> Notice {
    let mut message = n.message().to_string();
    if let Some(d) = n.detail() {
        message.push_str("\nDETAIL: ");
        message.push_str(d);
    }
    if let Some(h) = n.hint() {
        message.push_str("\nHINT: ");
        message.push_str(h);
    }
    Notice { severity: n.severity().to_string(), message }
}

fn spawn_connection<S, T>(mut conn: tokio_postgres::Connection<S, T>, notices: mpsc::UnboundedSender<Notice>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut messages = futures_util::stream::poll_fn(move |cx| conn.poll_message(cx));
        while let Some(msg) = messages.next().await {
            match msg {
                Ok(AsyncMessage::Notice(n)) => {
                    let _ = notices.send(notice(&n));
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

/// libpq accepts either the socket directory or the full `.s.PGSQL.<port>` path.
fn socket_dir(path: &Path, port: u16) -> (PathBuf, u16) {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match name.strip_prefix(".s.PGSQL.").and_then(|p| p.parse().ok()) {
        Some(p) => (path.parent().map(Path::to_path_buf).unwrap_or_default(), p),
        None => (path.to_path_buf(), port),
    }
}

#[derive(Clone, Copy)]
enum Conv {
    Int,
    Float,
    Bool,
    Bytea,
    Text,
}

fn conv_for(ty: &Type) -> Conv {
    match ty.oid() {
        20 | 21 | 23 | 26 => Conv::Int,
        700 | 701 => Conv::Float,
        16 => Conv::Bool,
        17 => Conv::Bytea,
        _ => Conv::Text,
    }
}

fn pg_value(s: Option<&str>, conv: Conv) -> Value {
    let Some(s) = s else { return Value::Null };
    let parsed = match conv {
        Conv::Int => s.parse().ok().map(Value::Int),
        Conv::Float => s.parse().ok().map(Value::Float),
        Conv::Bool => match s {
            "t" => Some(Value::Bool(true)),
            "f" => Some(Value::Bool(false)),
            _ => None,
        },
        Conv::Bytea => s.strip_prefix("\\x").and_then(decode_hex).map(Value::Bytes),
        Conv::Text => None,
    };
    parsed.unwrap_or_else(|| Value::Text(s.to_string()))
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    let (pairs, rest) = s.as_bytes().as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    let digit = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    pairs.iter().map(|[hi, lo]| Some(digit(*hi)? << 4 | digit(*lo)?)).collect()
}

fn upper_words(sql: &str, max: usize) -> Vec<String> {
    tokenize(sql, B)
        .into_iter()
        .filter(|t| !t.is_trivia())
        .take_while(|t| t.is_word())
        .take(max)
        .map(|t| t.text(sql).to_ascii_uppercase())
        .collect()
}

/// tokio-postgres only exposes the row count of CommandComplete, so the tag is rebuilt from the statement.
fn command_tag(sql: &str, count: u64) -> (String, Option<u64>) {
    let words = upper_words(sql, 8);
    let w = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
    let mut verb = w(0).to_string();
    if verb == "WITH" {
        verb = classify::main_verb(sql, B).unwrap_or_else(|| "SELECT".into());
    }
    match verb.as_str() {
        "INSERT" => (format!("INSERT 0 {count}"), Some(count)),
        "SELECT" | "VALUES" | "TABLE" => (format!("SELECT {count}"), Some(count)),
        "UPDATE" | "DELETE" | "MERGE" | "FETCH" | "MOVE" | "COPY" => (format!("{verb} {count}"), Some(count)),
        "CREATE" | "DROP" | "ALTER" => {
            if verb == "CREATE" && count > 0 {
                return (format!("SELECT {count}"), Some(count));
            }
            let mut i = 1;
            while matches!(
                w(i),
                "OR" | "REPLACE" | "UNIQUE" | "TEMP" | "TEMPORARY" | "UNLOGGED" | "GLOBAL" | "LOCAL" | "RECURSIVE"
                    | "TRUSTED" | "PROCEDURAL" | "DEFAULT"
            ) {
                i += 1;
            }
            let object = match w(i) {
                o @ ("MATERIALIZED" | "FOREIGN" | "EVENT" | "TEXT" | "ACCESS" | "USER" | "OPERATOR") => {
                    format!("{o} {}", w(i + 1))
                }
                o => o.to_string(),
            };
            (format!("{verb} {object}").trim_end().to_string(), None)
        }
        "START" => ("START TRANSACTION".into(), None),
        "END" | "COMMIT" => ("COMMIT".into(), None),
        "ABORT" | "ROLLBACK" => ("ROLLBACK".into(), None),
        "TRUNCATE" => ("TRUNCATE TABLE".into(), None),
        "REFRESH" => ("REFRESH MATERIALIZED VIEW".into(), None),
        "" => ("OK".into(), None),
        _ => (verb, None),
    }
}

/// `COPY … FROM STDIN` would leave the server waiting for data the simple-query stream can't send.
fn is_copy_from_stdin(sql: &str) -> bool {
    let mut words = tokenize(sql, B).into_iter().filter(|t| t.is_word()).map(|t| t.text(sql));
    words.next().is_some_and(|w| w.eq_ignore_ascii_case("copy")) && words.any(|w| w.eq_ignore_ascii_case("stdin"))
}

fn is_single_statement(sql: &str) -> bool {
    crate::sql::split::split(sql, B, ";").len() == 1
}

async fn flush(batch: &mut Vec<Row>, tx: &mpsc::Sender<ExecEvent>) -> bool {
    if batch.is_empty() {
        return true;
    }
    let rows = std::mem::replace(batch, Vec::with_capacity(BATCH_ROWS));
    tx.send(ExecEvent::Rows(rows)).await.is_ok()
}

fn rel_kind(kind: &str, schema: &str) -> RelKind {
    match kind {
        "v" => RelKind::View,
        "m" => RelKind::MaterializedView,
        "f" => RelKind::ForeignTable,
        "p" => RelKind::PartitionedTable,
        _ if schema == "pg_catalog" || schema == "information_schema" => RelKind::SystemTable,
        _ => RelKind::Table,
    }
}

fn fk_action(code: &str) -> Option<String> {
    Some(
        match code {
            "r" => "RESTRICT",
            "c" => "CASCADE",
            "n" => "SET NULL",
            "d" => "SET DEFAULT",
            _ => return None,
        }
        .to_string(),
    )
}

// Relation-less pg_get_expr is much faster and only generated columns may reference other columns.
const RELATIONS_SQL: &str = "\
SELECT c.oid, n.nspname::text, c.relname::text, c.relkind::text, td.description, c.reltuples::int8,
       a.attnum::int4, a.attname::text, format_type(a.atttypid, a.atttypmod), a.attnotnull,
       pg_get_expr(d.adbin, CASE WHEN a.attgenerated = '' THEN 0::oid ELSE d.adrelid END),
       coalesce(a.attnum = ANY(i.indkey), false), a.attidentity::text, cd.description
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_index i ON i.indrelid = c.oid AND i.indisprimary
LEFT JOIN pg_description td ON td.objoid = c.oid AND td.classoid = 'pg_class'::regclass AND td.objsubid = 0
LEFT JOIN pg_description cd ON cd.objoid = c.oid AND cd.classoid = 'pg_class'::regclass AND cd.objsubid = a.attnum
WHERE c.relkind IN ('r', 'v', 'm', 'f', 'p') AND n.nspname !~ '^pg_(toast|temp_)'
ORDER BY c.oid, a.attnum";

/// FK column numbers only; names are resolved from the relations query (avoids per-FK subplans).
const FK_KEYS_SQL: &str = "\
SELECT con.conname::text, sn.nspname::text, sc.relname::text, con.conrelid, con.conkey,
       tn.nspname::text, tc.relname::text, con.confrelid, con.confkey, con.confupdtype::text, con.confdeltype::text
FROM pg_constraint con
JOIN pg_class sc ON sc.oid = con.conrelid JOIN pg_namespace sn ON sn.oid = sc.relnamespace
JOIN pg_class tc ON tc.oid = con.confrelid JOIN pg_namespace tn ON tn.oid = tc.relnamespace
WHERE con.contype = 'f'
ORDER BY 2, 3, 1";

const FK_SQL: &str = "\
SELECT con.conname::text, sn.nspname::text, sc.relname::text,
       ARRAY(SELECT a.attname::text FROM unnest(con.conkey) WITH ORDINALITY k(num, ord)
             JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.num ORDER BY k.ord),
       tn.nspname::text, tc.relname::text,
       ARRAY(SELECT a.attname::text FROM unnest(con.confkey) WITH ORDINALITY k(num, ord)
             JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.num ORDER BY k.ord),
       con.confupdtype::text, con.confdeltype::text
FROM pg_constraint con
JOIN pg_class sc ON sc.oid = con.conrelid JOIN pg_namespace sn ON sn.oid = sc.relnamespace
JOIN pg_class tc ON tc.oid = con.confrelid JOIN pg_namespace tn ON tn.oid = tc.relnamespace
WHERE con.contype = 'f'";

fn fk_from_row(r: &tokio_postgres::Row) -> ForeignKey {
    ForeignKey {
        name: r.get(0),
        schema: r.get(1),
        table: r.get(2),
        columns: r.get(3),
        ref_schema: r.get(4),
        ref_table: r.get(5),
        ref_columns: r.get(6),
        on_update: fk_action(r.get(7)),
        on_delete: fk_action(r.get(8)),
    }
}

struct RelInfo {
    oid: u32,
    schema: String,
    name: String,
    relkind: String,
    reltuples: i64,
    size: Option<i64>,
    comment: Option<String>,
    view_def: Option<String>,
}

impl PgConn {
    pub async fn connect(spec: &ConnSpec) -> DbResult<Self> {
        let mut cfg = tokio_postgres::Config::new();
        let user = spec.user_or_default();
        cfg.user(&user);
        cfg.dbname(spec.database.as_deref().unwrap_or(&user));
        if let Some(p) = spec.password.clone().or_else(|| std::env::var("PGPASSWORD").ok()) {
            cfg.password(p);
        }
        cfg.application_name(spec.param("application_name").unwrap_or("quarry"));
        if let Some(o) = spec.param("options") {
            cfg.options(o);
        }
        if let Some(t) = spec.connect_timeout {
            cfg.connect_timeout(t);
        }
        let port = spec.port_or_default();
        // libpq ignores sslmode for unix sockets; the server refuses TLS there.
        let ssl = if spec.socket.is_some() { SslMode::Disable } else { spec.ssl_mode };
        match &spec.socket {
            Some(sock) => {
                let (dir, port) = socket_dir(sock, port);
                cfg.host_path(dir).port(port);
            }
            None => {
                cfg.host(spec.host_or_default()).port(port);
            }
        }
        cfg.ssl_mode(match ssl {
            SslMode::Disable => tokio_postgres::config::SslMode::Disable,
            SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
            _ => tokio_postgres::config::SslMode::Require,
        });
        let tls = match ssl {
            SslMode::Disable => PgTls::Plain,
            mode => {
                let spec = ConnSpec { ssl_mode: mode, ..spec.clone() };
                match tls::client_config(&spec)? {
                    Some(c) => PgTls::Rustls(MakeRustlsConnect::new(c)),
                    None => PgTls::Plain,
                }
            }
        };
        let (notice_tx, notices) = mpsc::unbounded_channel();
        let client = match &tls {
            PgTls::Plain => {
                let (client, conn) = cfg.connect(NoTls).await.map_err(connect_err)?;
                spawn_connection(conn, notice_tx);
                client
            }
            PgTls::Rustls(t) => {
                let (client, conn) = cfg.connect(t.clone()).await.map_err(connect_err)?;
                spawn_connection(conn, notice_tx);
                client
            }
        };

        let row = client
            .query_one(
                "SELECT current_setting('server_version'), current_user::text, current_database()::text,
                        pg_backend_pid(), coalesce((SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()), false)",
                &[],
            )
            .await
            .map_err(pg_err)?;
        let version: String = row.get(0);
        let info = ServerInfo {
            version: format!("PostgreSQL {}", version.split_whitespace().next().unwrap_or(&version)),
            user: Some(row.get(1)),
            host: Some(match &spec.socket {
                Some(s) => s.display().to_string(),
                None => spec.host_or_default().to_string(),
            }),
            port: Some(port),
            database: Some(row.get(2)),
            session_id: Some(row.get::<_, i32>(3).to_string()),
            tls: row.get(4),
            is_mariadb: false,
        };
        if spec.readonly {
            client.batch_execute("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY").await.map_err(pg_err)?;
        }
        for cmd in &spec.init_commands {
            client.batch_execute(cmd).await.map_err(pg_err)?;
        }
        Ok(PgConn { client, spec: Box::new(spec.clone()), info, tls, notices, in_tx: false })
    }

    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    fn canceller(&self) -> PgCancel {
        PgCancel { token: self.client.cancel_token(), tls: self.tls.clone() }
    }

    pub async fn execute(&mut self, sql: &str, tx: &mpsc::Sender<ExecEvent>) -> DbResult<()> {
        while self.notices.try_recv().is_ok() {}
        let res = self.run(sql, tx).await;
        match classify::transaction_effect(sql, B) {
            TxEffect::Begin if res.is_ok() => self.in_tx = true,
            TxEffect::End => self.in_tx = false,
            _ => {}
        }
        if res.as_ref().is_err_and(|e| e.kind == ErrorKind::Connection) || self.client.is_closed() {
            self.in_tx = false;
        }
        res
    }

    async fn forward_notices(&mut self, tx: &mpsc::Sender<ExecEvent>) {
        while let Ok(n) = self.notices.try_recv() {
            let _ = tx.send(ExecEvent::Notice(n)).await;
        }
    }

    /// Result column types via Parse/Describe; `None` when describing isn't possible.
    async fn describe(&self, sql: &str) -> DbResult<Option<Vec<Type>>> {
        match self.client.prepare(sql).await {
            Ok(st) => Ok(Some(st.columns().iter().map(|c| c.type_().clone()).collect())),
            // A failed Parse aborts an open transaction, so the real statement would only report 25P02.
            Err(e) if self.in_tx && e.as_db_error().is_some() => Err(pg_err(e)),
            Err(_) => Ok(None),
        }
    }

    async fn abandon(&self, mut stream: Pin<&mut SimpleQueryStream>) -> DbError {
        let _ = self.canceller().cancel().await;
        // Drain so the cancel request can't land on the next statement.
        while let Some(Ok(_)) = stream.next().await {}
        DbError::new(ErrorKind::Cancelled, "canceled: result consumer went away")
    }

    async fn run(&mut self, sql: &str, tx: &mpsc::Sender<ExecEvent>) -> DbResult<()> {
        if is_copy_from_stdin(sql) {
            return Err(DbError::query("COPY ... FROM STDIN is not supported here; use \\copy instead"));
        }
        let types = if classify::returns_rows(sql, B) && is_single_statement(sql) {
            self.describe(sql).await?
        } else {
            None
        };
        let stream = self.client.simple_query_raw(sql).await.map_err(pg_err)?;
        let mut stream = pin!(stream);
        let mut convs: Vec<Conv> = Vec::new();
        let mut batch: Vec<Row> = Vec::new();
        let mut rows_seen = 0u64;
        let mut had_rows = false;
        let mut count = 0u64;
        loop {
            let item = match stream.as_mut().next().now_or_never() {
                Some(item) => item,
                None => {
                    if !flush(&mut batch, tx).await {
                        return Err(self.abandon(stream.as_mut()).await);
                    }
                    stream.as_mut().next().await
                }
            };
            if !self.notices.is_empty() {
                self.forward_notices(tx).await;
            }
            match item {
                None => break,
                Some(Err(e)) => return Err(pg_err(e)),
                Some(Ok(SimpleQueryMessage::RowDescription(cols))) => {
                    if !flush(&mut batch, tx).await {
                        return Err(self.abandon(stream.as_mut()).await);
                    }
                    let typed = types.as_ref().filter(|t| t.len() == cols.len());
                    convs = match typed {
                        Some(t) => t.iter().map(conv_for).collect(),
                        None => vec![Conv::Text; cols.len()],
                    };
                    let columns = cols
                        .iter()
                        .enumerate()
                        .map(|(i, c)| Column::new(c.name(), typed.map(|t| t[i].name()).unwrap_or("")))
                        .collect();
                    had_rows = true;
                    rows_seen = 0;
                    batch = Vec::with_capacity(BATCH_ROWS);
                    if tx.send(ExecEvent::Columns(columns)).await.is_err() {
                        return Err(self.abandon(stream.as_mut()).await);
                    }
                }
                Some(Ok(SimpleQueryMessage::Row(row))) => {
                    let values = (0..row.len()).map(|i| pg_value(row.get(i), convs.get(i).copied().unwrap_or(Conv::Text)));
                    batch.push(values.collect());
                    rows_seen += 1;
                    if batch.len() >= BATCH_ROWS && !flush(&mut batch, tx).await {
                        return Err(self.abandon(stream.as_mut()).await);
                    }
                }
                Some(Ok(SimpleQueryMessage::CommandComplete(n))) => count = n,
                Some(Ok(_)) => {}
            }
        }
        if !flush(&mut batch, tx).await {
            return Err(DbError::new(ErrorKind::Cancelled, "canceled: result consumer went away"));
        }
        self.forward_notices(tx).await;
        let (status, rows_affected) = if had_rows {
            let (tag, _) = command_tag(sql, rows_seen);
            let tag = if tag.ends_with(char::is_numeric) { tag } else { format!("{tag} {rows_seen}") };
            (tag, Some(rows_seen))
        } else {
            command_tag(sql, count)
        };
        let summary = Summary { status: Some(status), rows_affected, ..Default::default() };
        let _ = tx.send(ExecEvent::Done(summary)).await;
        Ok(())
    }

    async fn collect(&mut self, sql: &str) -> DbResult<ResultSet> {
        let (tx, rx) = mpsc::channel(64);
        let run = async move {
            let tx = tx;
            self.execute(sql, &tx).await
        };
        let (res, rs) = tokio::join!(run, collect_events(rx));
        res.map(|_| rs)
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle::Postgres(self.canceller())
    }

    pub fn in_transaction(&self) -> bool {
        self.in_tx
    }

    pub async fn change_database(&mut self, database: &str) -> DbResult<()> {
        let spec = ConnSpec { database: Some(database.to_string()), ..(*self.spec).clone() };
        *self = PgConn::connect(&spec).await?;
        Ok(())
    }

    async fn rows(&self, sql: &str, params: &[(&(dyn ToSql + Sync), Type)]) -> DbResult<Vec<tokio_postgres::Row>> {
        self.client.query_typed(sql, params).await.map_err(pg_err)
    }

    pub async fn load_catalog(&mut self) -> DbResult<Catalog> {
        let c = &self.client;
        let (meta, schemas, rels, funcs, types, fks, users) = tokio::try_join!(
            c.query_typed(
                "SELECT current_database()::text, current_schemas(false)::text[],
                        ARRAY(SELECT datname::text FROM pg_database WHERE datallowconn ORDER BY 1)",
                &[],
            ),
            c.query_typed("SELECT nspname::text FROM pg_namespace WHERE nspname !~ '^pg_(toast|temp_)' ORDER BY 1", &[]),
            c.query_typed(RELATIONS_SQL, &[]),
            c.query_typed(
                "SELECT n.nspname::text, p.proname::text, pg_get_function_arguments(p.oid),
                        pg_get_function_result(p.oid), p.prokind::text, p.prorettype = 'trigger'::regtype
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
                 WHERE n.nspname !~ '^pg_(toast|temp_)' AND n.nspname <> 'information_schema'
                 ORDER BY 1, 2",
                &[],
            ),
            c.query_typed(
                "SELECT n.nspname::text, t.typname::text
                 FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
                 WHERE (t.typrelid = 0 OR (SELECT c.relkind = 'c' FROM pg_class c WHERE c.oid = t.typrelid))
                   AND NOT EXISTS (SELECT 1 FROM pg_type el WHERE el.oid = t.typelem AND el.typarray = t.oid)
                   AND n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname !~ '^pg_(toast|temp_)'
                 ORDER BY 1, 2",
                &[],
            ),
            c.query_typed(FK_KEYS_SQL, &[]),
            c.query_typed("SELECT rolname::text FROM pg_roles ORDER BY 1", &[]),
        )
        .map_err(pg_err)?;

        let mut cat = Catalog::empty(B);
        let meta = meta.first().ok_or_else(|| DbError::other("empty catalog metadata"))?;
        cat.current_database = Some(meta.get(0));
        cat.search_path = meta.get(1);
        if !cat.search_path.iter().any(|s| s == "pg_catalog") {
            cat.search_path.push("pg_catalog".into());
        }
        cat.databases = meta.get(2);

        let mut index: HashMap<String, usize> = HashMap::with_capacity(schemas.len());
        for r in &schemas {
            let name: String = r.get(0);
            index.insert(name.clone(), cat.schemas.len());
            cat.schemas.push(SchemaInfo { name, relations: Vec::new(), functions: Vec::new(), types: Vec::new() });
        }
        let fk_rels: HashSet<u32> = fks.iter().flat_map(|r| [r.get::<_, u32>(3), r.get::<_, u32>(7)]).collect();
        let mut attnames: HashMap<(u32, i16), String> = HashMap::new();
        let mut current: Option<(u32, usize, Relation)> = None;
        for r in &rels {
            let oid: u32 = r.get(0);
            if current.as_ref().is_none_or(|(o, _, _)| *o != oid) {
                if let Some((_, si, rel)) = current.take() {
                    cat.schemas[si].relations.push(rel);
                }
                let schema: &str = r.get(1);
                let Some(&si) = index.get(schema) else { continue };
                let kind = rel_kind(r.get(3), schema);
                let tuples: i64 = r.get(5);
                current = Some((
                    oid,
                    si,
                    Relation {
                        schema: schema.to_string(),
                        name: r.get(2),
                        kind,
                        columns: Vec::new(),
                        comment: r.get(4),
                        row_estimate: (tuples >= 0 && !kind.is_view()).then_some(tuples),
                    },
                ));
            }
            let Some((_, _, rel)) = current.as_mut() else { continue };
            let Some(col) = r.get::<_, Option<String>>(7) else { continue };
            if fk_rels.contains(&oid) {
                attnames.insert((oid, r.get::<_, i32>(6) as i16), col.clone());
            }
            let default: Option<String> = r.get(10);
            let identity: Option<String> = r.get(12);
            rel.columns.push(ColumnInfo {
                name: col,
                data_type: r.get(8),
                nullable: !r.get::<_, bool>(9),
                auto: identity.is_some_and(|i| !i.is_empty())
                    || default.as_deref().is_some_and(|d| d.starts_with("nextval(")),
                default,
                primary_key: r.get(11),
                comment: r.get(13),
            });
        }
        if let Some((_, si, rel)) = current.take() {
            cat.schemas[si].relations.push(rel);
        }
        for s in &mut cat.schemas {
            s.relations.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        }

        for r in &funcs {
            let Some(&si) = index.get(r.get::<_, &str>(0)) else { continue };
            let kind = match r.get::<_, &str>(4) {
                "a" => FunctionKind::Aggregate,
                "w" => FunctionKind::Window,
                "p" => FunctionKind::Procedure,
                _ if r.get::<_, bool>(5) => FunctionKind::Trigger,
                _ => FunctionKind::Function,
            };
            cat.schemas[si].functions.push(FunctionInfo {
                schema: r.get(0),
                name: r.get(1),
                args: r.get::<_, Option<String>>(2).unwrap_or_default(),
                return_type: r.get::<_, Option<String>>(3).unwrap_or_default(),
                kind,
            });
        }
        for r in &types {
            if let Some(&si) = index.get(r.get::<_, &str>(0)) {
                cat.schemas[si].types.push(r.get(1));
            }
        }
        let names = |rel: u32, keys: Vec<i16>| -> Vec<String> {
            keys.into_iter().map(|k| attnames.get(&(rel, k)).cloned().unwrap_or_default()).collect()
        };
        cat.foreign_keys = fks
            .iter()
            .map(|r| ForeignKey {
                name: r.get(0),
                schema: r.get(1),
                table: r.get(2),
                columns: names(r.get(3), r.get(4)),
                ref_schema: r.get(5),
                ref_table: r.get(6),
                ref_columns: names(r.get(7), r.get(8)),
                on_update: fk_action(r.get(9)),
                on_delete: fk_action(r.get(10)),
            })
            .collect();
        cat.users = users.iter().map(|r| r.get(0)).collect();
        Ok(cat)
    }

    pub async fn list_databases(&mut self) -> DbResult<Vec<String>> {
        let rows = self.rows("SELECT datname::text FROM pg_database WHERE datallowconn ORDER BY 1", &[]).await?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn list_relations(&mut self, schema: &str) -> DbResult<Vec<Relation>> {
        let rows = self
            .rows(
                "SELECT c.relname::text, c.relkind::text, obj_description(c.oid, 'pg_class'), c.reltuples::int8
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = $1 AND c.relkind IN ('r', 'v', 'm', 'f', 'p')
                 ORDER BY 1",
                &[(&schema, Type::TEXT)],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let kind = rel_kind(r.get(1), schema);
                let tuples: i64 = r.get(3);
                Relation {
                    schema: schema.to_string(),
                    name: r.get(0),
                    kind,
                    columns: Vec::new(),
                    comment: r.get(2),
                    row_estimate: (tuples >= 0 && !kind.is_view()).then_some(tuples),
                }
            })
            .collect())
    }

    async fn resolve(&self, schema: Option<&str>, name: &str) -> DbResult<RelInfo> {
        let target = qualified(schema, name, B);
        let rows = self
            .rows(
                "SELECT c.oid, n.nspname::text, c.relname::text, c.relkind::text, c.reltuples::int8,
                        CASE WHEN c.relkind IN ('r', 'm', 'p', 't') THEN pg_total_relation_size(c.oid) END,
                        obj_description(c.oid, 'pg_class'),
                        CASE WHEN c.relkind IN ('v', 'm') THEN pg_get_viewdef(c.oid, true) END
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE c.oid = to_regclass($1)",
                &[(&target, Type::TEXT)],
            )
            .await?;
        let r = rows.first().ok_or_else(|| DbError::query(format!("relation \"{target}\" does not exist")))?;
        Ok(RelInfo {
            oid: r.get(0),
            schema: r.get(1),
            name: r.get(2),
            relkind: r.get(3),
            reltuples: r.get(4),
            size: r.get(5),
            comment: r.get(6),
            view_def: r.get(7),
        })
    }

    pub async fn table_details(&mut self, schema: Option<&str>, table: &str) -> DbResult<TableDetails> {
        let rel = self.resolve(schema, table).await?;
        self.details_of(&rel).await
    }

    async fn details_of(&self, rel: &RelInfo) -> DbResult<TableDetails> {
        let p: [(&(dyn ToSql + Sync), Type); 1] = [(&rel.oid, Type::OID)];
        let fk_sql = format!("{FK_SQL} AND (con.conrelid = $1 OR con.confrelid = $1) ORDER BY 1");
        let c = &self.client;
        let (cols, idx, fks, cons, trigs) = tokio::try_join!(
            c.query_typed(
                "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), a.attnotnull, pg_get_expr(d.adbin, d.adrelid),
                        coalesce(a.attnum = ANY(i.indkey), false), a.attidentity::text, col_description(a.attrelid, a.attnum)
                 FROM pg_attribute a
                 LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
                 LEFT JOIN pg_index i ON i.indrelid = a.attrelid AND i.indisprimary
                 WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
                 ORDER BY a.attnum",
                &p,
            ),
            c.query_typed(
                "SELECT ic.relname::text,
                        ARRAY(SELECT pg_get_indexdef(i.indexrelid, k, true) FROM generate_series(1, i.indnkeyatts::int) k),
                        i.indisunique, i.indisprimary, am.amname::text, pg_get_indexdef(i.indexrelid)
                 FROM pg_index i JOIN pg_class ic ON ic.oid = i.indexrelid JOIN pg_am am ON am.oid = ic.relam
                 WHERE i.indrelid = $1
                 ORDER BY i.indisprimary DESC, ic.relname",
                &p,
            ),
            c.query_typed(&fk_sql, &p),
            c.query_typed(
                "SELECT conname::text, contype::text, pg_get_constraintdef(oid, true)
                 FROM pg_constraint WHERE conrelid = $1 AND contype IN ('p', 'u', 'c', 'f', 'x')
                 ORDER BY CASE contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'c' THEN 2 WHEN 'f' THEN 3 ELSE 4 END, conname",
                &p,
            ),
            c.query_typed(
                "SELECT tgname::text, tgtype::int4, pg_get_triggerdef(oid, true)
                 FROM pg_trigger WHERE tgrelid = $1 AND NOT tgisinternal ORDER BY 1",
                &p,
            ),
        )
        .map_err(pg_err)?;

        let columns = cols
            .iter()
            .map(|r| {
                let default: Option<String> = r.get(3);
                let identity: Option<String> = r.get(5);
                ColumnInfo {
                    name: r.get(0),
                    data_type: r.get(1),
                    nullable: !r.get::<_, bool>(2),
                    auto: identity.is_some_and(|i| !i.is_empty())
                        || default.as_deref().is_some_and(|d| d.starts_with("nextval(")),
                    default,
                    primary_key: r.get(4),
                    comment: r.get(6),
                }
            })
            .collect();
        let indexes = idx
            .iter()
            .map(|r| IndexInfo {
                name: r.get(0),
                columns: r.get(1),
                unique: r.get(2),
                primary: r.get(3),
                method: Some(r.get(4)),
                definition: Some(r.get(5)),
            })
            .collect();
        let (mut foreign_keys, mut referenced_by) = (Vec::new(), Vec::new());
        for fk in fks.iter().map(fk_from_row) {
            let outgoing = fk.schema == rel.schema && fk.table == rel.name;
            let incoming = fk.ref_schema == rel.schema && fk.ref_table == rel.name;
            if outgoing && incoming {
                referenced_by.push(fk.clone());
            }
            if outgoing { foreign_keys.push(fk) } else if incoming { referenced_by.push(fk) }
        }
        let constraints = cons
            .iter()
            .map(|r| ConstraintInfo {
                name: r.get(0),
                kind: match r.get::<_, &str>(1) {
                    "p" => "PRIMARY KEY",
                    "u" => "UNIQUE",
                    "c" => "CHECK",
                    "f" => "FOREIGN KEY",
                    _ => "EXCLUDE",
                }
                .to_string(),
                definition: r.get(2),
            })
            .collect();
        let triggers = trigs
            .iter()
            .map(|r| {
                let t: i32 = r.get(1);
                let timing = if t & 64 != 0 {
                    "INSTEAD OF"
                } else if t & 2 != 0 {
                    "BEFORE"
                } else {
                    "AFTER"
                };
                let events: Vec<&str> = [(4, "INSERT"), (8, "DELETE"), (16, "UPDATE"), (32, "TRUNCATE")]
                    .iter()
                    .filter(|(bit, _)| t & bit != 0)
                    .map(|(_, e)| *e)
                    .collect();
                TriggerInfo { name: r.get(0), event: format!("{timing} {}", events.join(" OR ")), definition: r.get(2) }
            })
            .collect();
        let kind = rel_kind(&rel.relkind, &rel.schema);
        Ok(TableDetails {
            schema: rel.schema.clone(),
            name: rel.name.clone(),
            kind,
            columns,
            indexes,
            foreign_keys,
            referenced_by,
            constraints,
            triggers,
            row_estimate: (rel.reltuples >= 0 && !kind.is_view()).then_some(rel.reltuples),
            size_bytes: rel.size,
            comment: rel.comment.clone(),
            view_definition: rel.view_def.clone(),
        })
    }

    pub async fn object_ddl(&mut self, schema: Option<&str>, name: &str, kind: &str) -> DbResult<String> {
        let kind = kind.trim().to_ascii_lowercase();
        match kind.as_str() {
            "function" | "procedure" | "routine" => self.function_ddl(schema, name).await,
            "type" | "enum" | "domain" | "composite" => self.type_ddl(schema, name).await,
            "trigger" => self.trigger_ddl(schema, name).await,
            _ => {
                let rel = self.resolve(schema, name).await?;
                match rel.relkind.as_str() {
                    "v" | "m" => {
                        let mat = if rel.relkind == "m" { "MATERIALIZED " } else { "" };
                        let def = rel.view_def.clone().unwrap_or_default();
                        let def = def.trim_end().trim_end_matches(';');
                        let mut out = format!("CREATE {mat}VIEW {} AS\n{def};\n", qualified(Some(&rel.schema), &rel.name, B));
                        if let Some(c) = &rel.comment {
                            out.push_str(&format!(
                                "\nCOMMENT ON {mat}VIEW {} IS {};\n",
                                qualified(Some(&rel.schema), &rel.name, B),
                                quote_literal(c, B)
                            ));
                        }
                        Ok(out)
                    }
                    "S" => self.sequence_ddl(&rel).await,
                    "i" | "I" => {
                        let r = self
                            .rows("SELECT pg_get_indexdef($1)", &[(&rel.oid, Type::OID)])
                            .await?;
                        Ok(format!("{};\n", r[0].get::<_, String>(0)))
                    }
                    "r" | "p" | "f" => self.table_ddl(&rel).await,
                    other => Err(DbError::other(format!("cannot generate DDL for relation kind '{other}'"))),
                }
            }
        }
    }

    async fn table_ddl(&self, rel: &RelInfo) -> DbResult<String> {
        let details = self.details_of(rel).await?;
        let p: [(&(dyn ToSql + Sync), Type); 1] = [(&rel.oid, Type::OID)];
        let c = &self.client;
        let (extra, cols) = tokio::try_join!(
            c.query_typed(
                "SELECT c.relpersistence::text,
                        CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END,
                        CASE WHEN c.relispartition THEN
                            (SELECT i.inhparent::regclass::text FROM pg_inherits i WHERE i.inhrelid = c.oid) END,
                        CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid) END,
                        (SELECT s.srvname::text FROM pg_foreign_table ft JOIN pg_foreign_server s ON s.oid = ft.ftserver
                         WHERE ft.ftrelid = c.oid),
                        ARRAY(SELECT ic.relname::text FROM pg_constraint con JOIN pg_class ic ON ic.oid = con.conindid
                              WHERE con.conrelid = c.oid AND con.contype IN ('p', 'u', 'x'))
                 FROM pg_class c WHERE c.oid = $1",
                &p,
            ),
            c.query_typed(
                "SELECT a.attidentity::text, a.attgenerated::text,
                        (SELECT quote_ident(co.collname) FROM pg_collation co JOIN pg_type t ON t.oid = a.atttypid
                         WHERE co.oid = a.attcollation AND a.attcollation <> t.typcollation)
                 FROM pg_attribute a WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
                 ORDER BY a.attnum",
                &p,
            ),
        )
        .map_err(pg_err)?;
        let x = extra.first().ok_or_else(|| DbError::other("relation vanished"))?;
        let persistence: String = x.get(0);
        let partkey: Option<String> = x.get(1);
        let parent: Option<String> = x.get(2);
        let bound: Option<String> = x.get(3);
        let server: Option<String> = x.get(4);
        let constraint_indexes: Vec<String> = x.get(5);

        let qname = qualified(Some(&rel.schema), &rel.name, B);
        let mut out = String::new();
        let prefix = match (persistence.as_str(), server.is_some()) {
            (_, true) => "CREATE FOREIGN TABLE",
            ("u", _) => "CREATE UNLOGGED TABLE",
            _ => "CREATE TABLE",
        };
        if let (Some(parent), Some(bound)) = (&parent, &bound) {
            out.push_str(&format!("{prefix} {qname} PARTITION OF {parent}\n    {bound}"));
        } else {
            let mut lines: Vec<String> = Vec::new();
            for (col, x) in details.columns.iter().zip(cols.iter()) {
                let identity: String = x.get(0);
                let generated: String = x.get(1);
                let collation: Option<String> = x.get(2);
                let mut line = format!("    {} {}", quote_ident(&col.name, B), col.data_type);
                if let Some(co) = collation {
                    line.push_str(&format!(" COLLATE {co}"));
                }
                match (identity.as_str(), generated.as_str(), &col.default) {
                    ("a", _, _) => line.push_str(" GENERATED ALWAYS AS IDENTITY"),
                    ("d", _, _) => line.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
                    (_, "s", Some(e)) => line.push_str(&format!(" GENERATED ALWAYS AS ({e}) STORED")),
                    (_, "v", Some(e)) => line.push_str(&format!(" GENERATED ALWAYS AS ({e}) VIRTUAL")),
                    (_, _, Some(d)) => line.push_str(&format!(" DEFAULT {d}")),
                    _ => {}
                }
                if !col.nullable {
                    line.push_str(" NOT NULL");
                }
                lines.push(line);
            }
            for con in &details.constraints {
                lines.push(format!("    CONSTRAINT {} {}", quote_ident(&con.name, B), con.definition));
            }
            out.push_str(&format!("{prefix} {qname} (\n{}\n)", lines.join(",\n")));
            if let Some(pk) = partkey {
                out.push_str(&format!(" PARTITION BY {pk}"));
            }
            if let Some(s) = server {
                out.push_str(&format!(" SERVER {}", quote_ident(&s, B)));
            }
        }
        out.push_str(";\n");
        let indexes: Vec<&IndexInfo> =
            details.indexes.iter().filter(|i| !constraint_indexes.contains(&i.name)).collect();
        if !indexes.is_empty() {
            out.push('\n');
            for i in indexes {
                out.push_str(&format!("{};\n", i.definition.as_deref().unwrap_or_default()));
            }
        }
        if !details.triggers.is_empty() {
            out.push('\n');
            for t in &details.triggers {
                out.push_str(&format!("{};\n", t.definition));
            }
        }
        let mut comments = String::new();
        if let Some(c) = &details.comment {
            comments.push_str(&format!("COMMENT ON TABLE {qname} IS {};\n", quote_literal(c, B)));
        }
        for col in &details.columns {
            if let Some(c) = &col.comment {
                comments.push_str(&format!(
                    "COMMENT ON COLUMN {qname}.{} IS {};\n",
                    quote_ident(&col.name, B),
                    quote_literal(c, B)
                ));
            }
        }
        if !comments.is_empty() {
            out.push('\n');
            out.push_str(&comments);
        }
        Ok(out)
    }

    async fn sequence_ddl(&self, rel: &RelInfo) -> DbResult<String> {
        let rows = self
            .rows(
                "SELECT format_type(seqtypid, NULL), seqstart, seqincrement, seqmin, seqmax, seqcache, seqcycle
                 FROM pg_sequence WHERE seqrelid = $1",
                &[(&rel.oid, Type::OID)],
            )
            .await?;
        let r = rows.first().ok_or_else(|| DbError::other("sequence not found"))?;
        Ok(format!(
            "CREATE SEQUENCE {} AS {} START WITH {} INCREMENT BY {} MINVALUE {} MAXVALUE {} CACHE {}{};\n",
            qualified(Some(&rel.schema), &rel.name, B),
            r.get::<_, String>(0),
            r.get::<_, i64>(1),
            r.get::<_, i64>(2),
            r.get::<_, i64>(3),
            r.get::<_, i64>(4),
            r.get::<_, i64>(5),
            if r.get::<_, bool>(6) { " CYCLE" } else { "" }
        ))
    }

    async fn function_ddl(&self, schema: Option<&str>, name: &str) -> DbResult<String> {
        let rows = self
            .rows(
                "SELECT pg_get_functiondef(p.oid)
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
                 WHERE p.proname = $2 AND p.prokind IN ('f', 'p', 'w')
                   AND (($1::text IS NULL AND pg_function_is_visible(p.oid)) OR n.nspname = $1)
                 ORDER BY p.oid",
                &[(&schema, Type::TEXT), (&name, Type::TEXT)],
            )
            .await?;
        if rows.is_empty() {
            return Err(DbError::query(format!("function \"{name}\" does not exist")));
        }
        let defs: Vec<String> =
            rows.iter().map(|r| format!("{};\n", r.get::<_, String>(0).trim_end())).collect();
        Ok(defs.join("\n"))
    }

    async fn trigger_ddl(&self, schema: Option<&str>, name: &str) -> DbResult<String> {
        let rows = self
            .rows(
                "SELECT pg_get_triggerdef(t.oid, true)
                 FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE t.tgname = $2 AND NOT t.tgisinternal
                   AND (($1::text IS NULL AND pg_table_is_visible(c.oid)) OR n.nspname = $1)",
                &[(&schema, Type::TEXT), (&name, Type::TEXT)],
            )
            .await?;
        let r = rows.first().ok_or_else(|| DbError::query(format!("trigger \"{name}\" does not exist")))?;
        Ok(format!("{};\n", r.get::<_, String>(0)))
    }

    async fn type_ddl(&self, schema: Option<&str>, name: &str) -> DbResult<String> {
        let target = qualified(schema, name, B);
        let rows = self
            .rows(
                "SELECT t.typtype::text, format_type(t.oid, NULL), format_type(t.typbasetype, t.typtypmod),
                        t.typnotnull, t.typdefault,
                        ARRAY(SELECT quote_literal(e.enumlabel) FROM pg_enum e WHERE e.enumtypid = t.oid ORDER BY e.enumsortorder),
                        ARRAY(SELECT 'CONSTRAINT ' || quote_ident(con.conname) || ' ' || pg_get_constraintdef(con.oid, true)
                              FROM pg_constraint con WHERE con.contypid = t.oid AND con.contype = 'c' ORDER BY con.conname),
                        ARRAY(SELECT quote_ident(a.attname) || ' ' || format_type(a.atttypid, a.atttypmod)
                              FROM pg_attribute a WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped
                              ORDER BY a.attnum),
                        (SELECT format_type(r.rngsubtype, NULL) FROM pg_range r WHERE r.rngtypid = t.oid)
                 FROM pg_type t WHERE t.oid = to_regtype($1)",
                &[(&target, Type::TEXT)],
            )
            .await?;
        let r = rows.first().ok_or_else(|| DbError::query(format!("type \"{target}\" does not exist")))?;
        let qname: String = r.get(1);
        let ddl = match r.get::<_, &str>(0) {
            "e" => format!("CREATE TYPE {qname} AS ENUM ({});\n", r.get::<_, Vec<String>>(5).join(", ")),
            "d" => {
                let mut s = format!("CREATE DOMAIN {qname} AS {}", r.get::<_, String>(2));
                if let Some(d) = r.get::<_, Option<String>>(4) {
                    s.push_str(&format!(" DEFAULT {d}"));
                }
                if r.get::<_, bool>(3) {
                    s.push_str(" NOT NULL");
                }
                for c in r.get::<_, Vec<String>>(6) {
                    s.push_str(&format!("\n    {c}"));
                }
                s + ";\n"
            }
            "c" => format!("CREATE TYPE {qname} AS (\n    {}\n);\n", r.get::<_, Vec<String>>(7).join(",\n    ")),
            "r" => format!(
                "CREATE TYPE {qname} AS RANGE (SUBTYPE = {});\n",
                r.get::<_, Option<String>>(8).unwrap_or_default()
            ),
            other => return Err(DbError::other(format!("cannot generate DDL for type kind '{other}'"))),
        };
        Ok(ddl)
    }

    pub async fn activity(&mut self) -> DbResult<ResultSet> {
        self.collect(
            "SELECT pid, usename AS \"user\", datname AS database, application_name AS application,
                    CASE WHEN client_port = -1 THEN 'local' ELSE host(client_addr) || ':' || client_port END AS client,
                    state,
                    date_trunc('milliseconds', now() - CASE WHEN state = 'active' THEN query_start ELSE state_change END)
                        AS duration,
                    wait_event_type || ': ' || wait_event AS wait_event,
                    query
             FROM pg_stat_activity
             WHERE pid <> pg_backend_pid() AND (backend_type = 'client backend' OR datname IS NOT NULL)
             ORDER BY pid",
        )
        .await
    }

    pub async fn kill_session(&mut self, id: &str) -> DbResult<()> {
        let pid: i32 = id.trim().parse().map_err(|_| DbError::other(format!("invalid backend pid '{id}'")))?;
        let rows = self.rows("SELECT pg_terminate_backend($1)", &[(&pid, Type::INT4)]).await?;
        if rows.first().is_some_and(|r| r.get::<_, bool>(0)) {
            Ok(())
        } else {
            Err(DbError::query(format!("no server process with pid {pid}")))
        }
    }

    pub async fn explain(&mut self, sql: &str, analyze: bool) -> DbResult<PlanNode> {
        let body = sql.trim().trim_end_matches(';').trim_end();
        let opts = if analyze { "FORMAT JSON, ANALYZE, BUFFERS" } else { "FORMAT JSON" };
        let stmt = format!("EXPLAIN ({opts}) {body}");
        let wrap = analyze && !classify::is_read_only(body, B);
        if wrap {
            let begin = if self.in_tx { "SAVEPOINT quarry_explain" } else { "BEGIN" };
            self.client.batch_execute(begin).await.map_err(pg_err)?;
        }
        let res = self.client.simple_query(&stmt).await;
        if wrap {
            let end = if self.in_tx {
                "ROLLBACK TO SAVEPOINT quarry_explain; RELEASE SAVEPOINT quarry_explain"
            } else {
                "ROLLBACK"
            };
            self.client.batch_execute(end).await.map_err(pg_err)?;
        }
        let text: String = res
            .map_err(pg_err)?
            .iter()
            .filter_map(|m| match m {
                SimpleQueryMessage::Row(r) => r.get(0).map(str::to_string),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let json: Json = serde_json::from_str(&text).map_err(|e| DbError::other(format!("unexpected EXPLAIN output: {e}")))?;
        let top = json.get(0).and_then(Json::as_object).ok_or_else(|| DbError::other("unexpected EXPLAIN output"))?;
        let plan = top.get("Plan").and_then(Json::as_object).ok_or_else(|| DbError::other("EXPLAIN output has no plan"))?;
        let mut root = plan_node(plan);
        for (k, v) in top {
            if k != "Plan" {
                root.details.push((k.clone(), json_text(v)));
            }
        }
        Ok(root)
    }
}

const LABEL_KEYS: &[&str] = &[
    "Node Type",
    "Plans",
    "Startup Cost",
    "Total Cost",
    "Plan Rows",
    "Actual Rows",
    "Actual Total Time",
    "Actual Loops",
    "Parallel Aware",
    "Async Capable",
    "Relation Name",
    "Alias",
    "Index Name",
    "Join Type",
    "Strategy",
    "Scan Direction",
    "CTE Name",
    "Function Name",
];

fn plan_label(p: &Map<String, Json>) -> String {
    let s = |k: &str| p.get(k).and_then(Json::as_str);
    let node = s("Node Type").unwrap_or("?");
    let mut label = match (node, s("Strategy"), s("Join Type")) {
        ("ModifyTable", _, _) => s("Operation").unwrap_or(node).to_string(),
        ("Aggregate", Some("Hashed"), _) => "HashAggregate".to_string(),
        ("Aggregate", Some("Sorted"), _) => "GroupAggregate".to_string(),
        ("Aggregate", Some("Mixed"), _) => "MixedAggregate".to_string(),
        ("SetOp", Some("Hashed"), _) => "HashSetOp".to_string(),
        ("Nested Loop", _, Some(j)) if j != "Inner" => format!("Nested Loop {j} Join"),
        (n, _, Some(j)) if j != "Inner" && n.ends_with(" Join") => format!("{} {j} Join", n.trim_end_matches(" Join")),
        (n, _, _) => n.to_string(),
    };
    if p.get("Parallel Aware").and_then(Json::as_bool) == Some(true) {
        label.insert_str(0, "Parallel ");
    }
    if s("Scan Direction") == Some("Backward") {
        label.push_str(" Backward");
    }
    if let Some(i) = s("Index Name") {
        label.push_str(&format!(" using {i}"));
    }
    let alias = s("Alias");
    let target = s("Relation Name").or(s("CTE Name")).or(s("Function Name"));
    match (target, alias) {
        (Some(t), Some(a)) if a != t => label.push_str(&format!(" on {t} {a}")),
        (Some(t), _) => label.push_str(&format!(" on {t}")),
        (None, Some(a)) => label.push_str(&format!(" on {a}")),
        _ => {}
    }
    label
}

fn plan_node(p: &Map<String, Json>) -> PlanNode {
    let f = |k: &str| p.get(k).and_then(Json::as_f64);
    PlanNode {
        label: plan_label(p),
        details: p
            .iter()
            .filter(|(k, _)| !LABEL_KEYS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), json_text(v)))
            .collect(),
        startup_cost: f("Startup Cost"),
        total_cost: f("Total Cost"),
        plan_rows: f("Plan Rows"),
        actual_rows: f("Actual Rows"),
        actual_time_ms: f("Actual Total Time"),
        loops: f("Actual Loops"),
        children: p
            .get("Plans")
            .and_then(Json::as_array)
            .map(|a| a.iter().filter_map(Json::as_object).map(plan_node).collect())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_typed_by_column_type() {
        assert_eq!(pg_value(Some("42"), Conv::Int), Value::Int(42));
        assert_eq!(pg_value(Some("-Infinity"), Conv::Float), Value::Float(f64::NEG_INFINITY));
        assert_eq!(pg_value(Some("t"), Conv::Bool), Value::Bool(true));
        assert_eq!(pg_value(Some("\\x00ff"), Conv::Bytea), Value::Bytes(vec![0, 255]));
        assert_eq!(pg_value(Some("007"), Conv::Text), Value::Text("007".into()));
        assert_eq!(pg_value(None, Conv::Int), Value::Null);
    }

    #[test]
    fn command_tags_match_psql() {
        assert_eq!(command_tag("insert into t values (1),(2)", 2).0, "INSERT 0 2");
        assert_eq!(command_tag("create unique index i on t (a)", 0).0, "CREATE INDEX");
        assert_eq!(command_tag("create or replace function f() ...", 0).0, "CREATE FUNCTION");
        assert_eq!(command_tag("drop materialized view v", 0).0, "DROP MATERIALIZED VIEW");
        assert_eq!(command_tag("with x as (select 1) delete from t", 4), ("DELETE 4".into(), Some(4)));
        assert_eq!(command_tag("create table t as select 1", 1).0, "SELECT 1");
    }

    #[test]
    fn copy_from_stdin_is_detected() {
        assert!(is_copy_from_stdin("copy t (a, b) from stdin with (format csv)"));
        assert!(!is_copy_from_stdin("copy t to stdout"));
        assert!(!is_copy_from_stdin("select 'stdin'"));
    }

    #[test]
    fn socket_path_forms() {
        assert_eq!(socket_dir(Path::new("/run/postgresql"), 5432), (PathBuf::from("/run/postgresql"), 5432));
        assert_eq!(socket_dir(Path::new("/tmp/.s.PGSQL.6000"), 5432), (PathBuf::from("/tmp"), 6000));
    }

    #[test]
    fn plan_labels_follow_text_format() {
        let j: Json = serde_json::json!({"Node Type": "Hash Join", "Join Type": "Left", "Plans": [
            {"Node Type": "Index Scan", "Index Name": "users_pkey", "Relation Name": "users", "Alias": "u",
             "Scan Direction": "Backward", "Total Cost": 8.5}
        ]});
        let n = plan_node(j.as_object().unwrap());
        assert_eq!(n.label, "Hash Left Join");
        assert_eq!(n.children[0].label, "Index Scan Backward using users_pkey on users u");
        assert_eq!(n.children[0].total_cost, Some(8.5));
    }
}
