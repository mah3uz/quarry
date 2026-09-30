use std::time::{Duration, Instant};

use mysql_async::consts::{ColumnFlags, ColumnType, StatusFlags};
use mysql_async::prelude::*;
use mysql_async::{ClientIdentity, Conn, Opts, OptsBuilder, QueryResult, SslOpts, TextProtocol};
use serde_json::{Map, Value as Json};
use tokio::sync::mpsc;

use super::*;
use crate::conn::{ConnSpec, SslMode};
use crate::sql::classify::{self, TxEffect};

const B: Backend = Backend::MySql;
/// Rows trickling in slower than this are flushed early so the consumer sees progress.
const FLUSH_AFTER: Duration = Duration::from_millis(50);

type MyRow = mysql_async::Row;

pub struct MyConn {
    conn: Conn,
    opts: Opts,
    info: ServerInfo,
    version: (u16, u16, u16),
    in_tx: bool,
}

#[derive(Clone)]
pub struct MyCancel {
    opts: Opts,
    id: u32,
    timeout: Option<Duration>,
}

impl MyCancel {
    pub async fn cancel(&self) -> DbResult<()> {
        let mut conn = open(self.opts.clone(), self.timeout).await?;
        let res = conn.query_drop(format!("KILL QUERY {}", self.id)).await.map_err(my_err);
        let _ = conn.disconnect().await;
        res
    }
}

fn my_err(e: mysql_async::Error) -> DbError {
    use mysql_async::{DriverError, Error};
    match e {
        Error::Server(s) => {
            let kind = match s.code {
                1045 | 1698 => ErrorKind::Auth,
                1317 => ErrorKind::Cancelled,
                1053 | 1077 | 1152 | 1153 | 1927 => ErrorKind::Connection,
                _ => ErrorKind::Query,
            };
            DbError { code: Some(s.code.to_string()), ..DbError::new(kind, s.message) }
        }
        Error::Io(io) => DbError::new(ErrorKind::Connection, io.to_string()),
        Error::Driver(d @ DriverError::ConnectionClosed) => DbError::new(ErrorKind::Connection, d.to_string()),
        other => DbError::other(other.to_string()),
    }
}

fn connect_err(e: mysql_async::Error) -> DbError {
    let mut err = my_err(e);
    if err.kind != ErrorKind::Auth {
        err.kind = ErrorKind::Connection;
    }
    err
}

async fn open_raw(opts: Opts, timeout: Option<Duration>) -> DbResult<Result<Conn, mysql_async::Error>> {
    let fut = Conn::new(opts);
    match timeout {
        Some(t) => tokio::time::timeout(t, fut)
            .await
            .map_err(|_| DbError::new(ErrorKind::Connection, format!("connection timed out after {}s", t.as_secs()))),
        None => Ok(fut.await),
    }
}

async fn open(opts: Opts, timeout: Option<Duration>) -> DbResult<Conn> {
    open_raw(opts, timeout).await?.map_err(connect_err)
}

/// Names the server certificate is valid for, when TLS failed only on the host name check.
fn cert_names_on_mismatch(e: &mysql_async::Error) -> Option<Vec<String>> {
    let mysql_async::Error::Io(mysql_async::IoError::Io(io)) = e else { return None };
    match io.get_ref()?.downcast_ref::<rustls::Error>()? {
        rustls::Error::InvalidCertificate(rustls::CertificateError::NotValidForNameContext { presented, .. }) => {
            Some(presented.iter().map(|n| n.trim_matches('"').to_string()).collect())
        }
        rustls::Error::InvalidCertificate(rustls::CertificateError::NotValidForName) => Some(Vec::new()),
        _ => None,
    }
}

/// mysql_async's skip-domain-validation misses rustls' newer name-mismatch error; retry pinned to a cert name.
async fn open_verify_ca(base: OptsBuilder, ssl: SslOpts, timeout: Option<Duration>) -> DbResult<(Conn, Opts)> {
    let opts = Opts::from(base.clone().ssl_opts(ssl.clone()));
    let err = match open_raw(opts.clone(), timeout).await? {
        Ok(c) => return Ok((c, opts)),
        Err(e) => e,
    };
    match cert_names_on_mismatch(&err) {
        Some(names) if !names.is_empty() => {
            let opts = Opts::from(base.ssl_opts(ssl.with_danger_tls_hostname_override(Some(names[0].clone()))));
            Ok((open(opts.clone(), timeout).await?, opts))
        }
        Some(_) => Err(DbError::new(
            ErrorKind::Connection,
            "ssl-mode verify-ca: the server certificate carries no subjectAltName, which the MySQL TLS backend \
             needs to accept it; use ssl-mode require, or give the server a certificate with a subjectAltName",
        )),
        None => Err(connect_err(err)),
    }
}

fn base_opts(spec: &ConnSpec) -> OptsBuilder {
    let pass = spec.password.clone().or_else(|| std::env::var("MYSQL_PWD").ok());
    let b = OptsBuilder::default()
        .user(Some(spec.user_or_default()))
        .pass(pass)
        .db_name(spec.database.clone())
        // Otherwise mysql_async silently hops from TCP to the server's unix socket on localhost.
        .prefer_socket(false)
        .tcp_port(spec.port_or_default());
    match &spec.socket {
        Some(s) => b.socket(Some(s.display().to_string())),
        None => b.ip_or_hostname(spec.host_or_default()),
    }
}

fn ssl_opts(spec: &ConnSpec) -> Option<SslOpts> {
    if spec.ssl_mode == SslMode::Disable || (spec.socket.is_some() && spec.ssl_mode == SslMode::Prefer) {
        return None;
    }
    let mut o = SslOpts::default();
    if let Some(ca) = &spec.ssl_ca {
        o = o.with_root_certs(vec![ca.clone().into()]);
    }
    if let (Some(cert), Some(key)) = (&spec.ssl_cert, &spec.ssl_key) {
        o = o.with_client_identity(Some(ClientIdentity::new(cert.clone().into(), key.clone().into())));
    }
    Some(match spec.ssl_mode {
        SslMode::VerifyFull => o,
        SslMode::VerifyCa => o.with_danger_skip_domain_validation(true),
        _ => o.with_danger_accept_invalid_certs(true).with_danger_skip_domain_validation(true),
    })
}

#[derive(Clone, Copy)]
enum Conv {
    Int,
    UInt,
    Float,
    Bit,
    Bytes,
    Text,
    Null,
}

fn conv_for(c: &mysql_async::Column) -> Conv {
    use ColumnType::*;
    let binary = c.character_set() == 63;
    match c.column_type() {
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_LONG | MYSQL_TYPE_INT24 | MYSQL_TYPE_LONGLONG => {
            if c.flags().contains(ColumnFlags::UNSIGNED_FLAG) { Conv::UInt } else { Conv::Int }
        }
        MYSQL_TYPE_YEAR => Conv::Int,
        MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE => Conv::Float,
        MYSQL_TYPE_BIT => Conv::Bit,
        MYSQL_TYPE_NULL => Conv::Null,
        MYSQL_TYPE_GEOMETRY | MYSQL_TYPE_VECTOR => Conv::Bytes,
        MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB | MYSQL_TYPE_VAR_STRING
        | MYSQL_TYPE_STRING | MYSQL_TYPE_VARCHAR
            if binary =>
        {
            Conv::Bytes
        }
        _ => Conv::Text,
    }
}

fn type_name(c: &mysql_async::Column) -> String {
    use ColumnType::*;
    let flags = c.flags();
    let binary = c.character_set() == 63;
    let unsigned = if flags.contains(ColumnFlags::UNSIGNED_FLAG) { " UNSIGNED" } else { "" };
    let name = match c.column_type() {
        MYSQL_TYPE_TINY => return format!("TINYINT{unsigned}"),
        MYSQL_TYPE_SHORT => return format!("SMALLINT{unsigned}"),
        MYSQL_TYPE_INT24 => return format!("MEDIUMINT{unsigned}"),
        MYSQL_TYPE_LONG => return format!("INT{unsigned}"),
        MYSQL_TYPE_LONGLONG => return format!("BIGINT{unsigned}"),
        MYSQL_TYPE_FLOAT => "FLOAT",
        MYSQL_TYPE_DOUBLE => "DOUBLE",
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => "DECIMAL",
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => "DATE",
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => "TIME",
        MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => "DATETIME",
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => "TIMESTAMP",
        MYSQL_TYPE_YEAR => "YEAR",
        MYSQL_TYPE_BIT => "BIT",
        MYSQL_TYPE_JSON => "JSON",
        MYSQL_TYPE_NULL => "NULL",
        MYSQL_TYPE_GEOMETRY => "GEOMETRY",
        MYSQL_TYPE_VECTOR => "VECTOR",
        _ if flags.contains(ColumnFlags::ENUM_FLAG) => "ENUM",
        _ if flags.contains(ColumnFlags::SET_FLAG) => "SET",
        MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB => {
            if binary { "BLOB" } else { "TEXT" }
        }
        MYSQL_TYPE_VAR_STRING | MYSQL_TYPE_VARCHAR => {
            if binary { "VARBINARY" } else { "VARCHAR" }
        }
        MYSQL_TYPE_STRING => {
            if binary { "BINARY" } else { "CHAR" }
        }
        _ => "",
    };
    name.to_string()
}

fn my_value(v: Option<mysql_async::Value>, conv: Conv) -> Value {
    use mysql_async::Value as V;
    let bytes = match v {
        None | Some(V::NULL) => return Value::Null,
        Some(V::Bytes(b)) => b,
        Some(V::Int(i)) => return Value::Int(i),
        Some(V::UInt(u)) => return Value::UInt(u),
        Some(V::Float(f)) => return Value::Float(f as f64),
        Some(V::Double(f)) => return Value::Float(f),
        Some(other) => return Value::Text(other.as_sql(true).trim_matches('\'').to_string()),
    };
    let text = || std::str::from_utf8(&bytes).ok();
    let parsed = match conv {
        Conv::Null => return Value::Null,
        Conv::Bytes => return Value::Bytes(bytes),
        Conv::Int => text().and_then(|s| s.parse().ok()).map(Value::Int),
        Conv::UInt => text().and_then(|s| s.parse().ok()).map(Value::UInt),
        Conv::Float => text().and_then(|s| s.parse().ok()).map(Value::Float),
        Conv::Bit => {
            let n = bytes.iter().fold(0u64, |acc, b| acc << 8 | *b as u64);
            Some(i64::try_from(n).map(Value::Int).unwrap_or(Value::UInt(n)))
        }
        Conv::Text => None,
    };
    parsed.unwrap_or_else(|| match String::from_utf8(bytes) {
        Ok(s) => Value::Text(s),
        Err(e) => Value::Bytes(e.into_bytes()),
    })
}

fn gs(r: &MyRow, i: usize) -> String {
    go(r, i).unwrap_or_default()
}

fn go(r: &MyRow, i: usize) -> Option<String> {
    r.get_opt::<Option<String>, _>(i).and_then(Result::ok).flatten()
}

fn gi(r: &MyRow, i: usize) -> Option<i64> {
    r.get_opt::<Option<i64>, _>(i).and_then(Result::ok).flatten()
}

fn lit(s: &str) -> String {
    quote_literal(s, B)
}

fn plural(n: u64, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

fn rel_kind(table_type: &str) -> RelKind {
    match table_type {
        "VIEW" => RelKind::View,
        "SYSTEM VIEW" => RelKind::SystemTable,
        _ => RelKind::Table,
    }
}

async fn flush(batch: &mut Vec<Row>, tx: &mpsc::Sender<ExecEvent>) -> bool {
    if batch.is_empty() {
        return true;
    }
    let rows = std::mem::replace(batch, Vec::with_capacity(BATCH_ROWS));
    tx.send(ExecEvent::Rows(rows)).await.is_ok()
}

async fn collect_sets(conn: &mut Conn, sql: &str, sets: usize) -> DbResult<Vec<Vec<MyRow>>> {
    let mut qr = conn.query_iter(sql).await.map_err(my_err)?;
    let mut out = Vec::with_capacity(sets);
    for _ in 0..sets {
        out.push(qr.collect::<MyRow>().await.map_err(my_err)?);
    }
    qr.drop_result().await.map_err(my_err)?;
    Ok(out)
}

fn fk_rows(rows: &[MyRow]) -> Vec<ForeignKey> {
    let mut out: Vec<ForeignKey> = Vec::new();
    for r in rows {
        let (name, schema, table) = (gs(r, 0), gs(r, 1), gs(r, 2));
        match out.last_mut() {
            Some(fk) if fk.name == name && fk.schema == schema && fk.table == table => {
                fk.columns.push(gs(r, 3));
                fk.ref_columns.push(gs(r, 6));
            }
            _ => out.push(ForeignKey {
                name,
                schema,
                table,
                columns: vec![gs(r, 3)],
                ref_schema: gs(r, 4),
                ref_table: gs(r, 5),
                ref_columns: vec![gs(r, 6)],
                on_update: go(r, 7).filter(|a| a != "NO ACTION" && a != "RESTRICT"),
                on_delete: go(r, 8).filter(|a| a != "NO ACTION" && a != "RESTRICT"),
            }),
        }
    }
    out
}

const FK_SELECT: &str = "\
SELECT k.CONSTRAINT_NAME, k.TABLE_SCHEMA, k.TABLE_NAME, k.COLUMN_NAME,
       k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, r.UPDATE_RULE, r.DELETE_RULE
FROM information_schema.KEY_COLUMN_USAGE k
JOIN information_schema.REFERENTIAL_CONSTRAINTS r
  ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME
WHERE k.REFERENCED_TABLE_NAME IS NOT NULL";

fn column_info(r: &MyRow, at: usize) -> ColumnInfo {
    ColumnInfo {
        name: gs(r, at),
        data_type: gs(r, at + 1),
        nullable: gs(r, at + 2) == "YES",
        default: go(r, at + 3),
        primary_key: gs(r, at + 4) == "PRI",
        auto: gs(r, at + 5).to_ascii_lowercase().contains("auto_increment"),
        comment: go(r, at + 6).filter(|c| !c.is_empty()),
    }
}

const COLUMN_FIELDS: &str =
    "c.COLUMN_NAME, c.COLUMN_TYPE, c.IS_NULLABLE, c.COLUMN_DEFAULT, c.COLUMN_KEY, c.EXTRA, c.COLUMN_COMMENT";

impl MyConn {
    pub async fn connect(spec: &ConnSpec) -> DbResult<Self> {
        tls::install_crypto_provider();
        let base = base_opts(spec);
        let timeout = spec.connect_timeout;
        let (conn, opts, tls) = match ssl_opts(spec) {
            None => {
                let opts = Opts::from(base);
                (open(opts.clone(), timeout).await?, opts, false)
            }
            Some(ssl) if spec.ssl_mode == SslMode::VerifyCa => {
                let (conn, opts) = open_verify_ca(base, ssl, timeout).await?;
                (conn, opts, true)
            }
            Some(ssl) => {
                let opts = Opts::from(base.clone().ssl_opts(ssl));
                match open(opts.clone(), timeout).await {
                    Ok(c) => (c, opts, true),
                    Err(e) if spec.ssl_mode == SslMode::Prefer && e.kind != ErrorKind::Auth => {
                        let opts = Opts::from(base);
                        (open(opts.clone(), timeout).await?, opts, false)
                    }
                    Err(e) => return Err(e),
                }
            }
        };
        let mut me = MyConn { version: conn.server_version(), conn, opts, info: ServerInfo::default(), in_tx: false };
        let row: Option<(String, u64, Option<String>, String)> = me
            .conn
            .query_first("SELECT VERSION(), CONNECTION_ID(), DATABASE(), CURRENT_USER()")
            .await
            .map_err(my_err)?;
        let (version, id, db, user) = row.ok_or_else(|| DbError::other("server returned no session info"))?;
        let is_mariadb = version.to_ascii_lowercase().contains("mariadb");
        let short = version.split('-').next().unwrap_or(&version);
        me.info = ServerInfo {
            version: format!("{} {short}", if is_mariadb { "MariaDB" } else { "MySQL" }),
            user: Some(user),
            host: Some(match &spec.socket {
                Some(s) => s.display().to_string(),
                None => spec.host_or_default().to_string(),
            }),
            port: spec.socket.is_none().then(|| spec.port_or_default()),
            database: db,
            session_id: Some(id.to_string()),
            tls,
            is_mariadb,
        };
        if let Some(cs) = spec.param("charset") {
            if !cs.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(DbError::other(format!("invalid charset '{cs}'")));
            }
            me.conn.query_drop(format!("SET NAMES {cs}")).await.map_err(my_err)?;
        }
        if spec.readonly {
            me.conn.query_drop("SET SESSION TRANSACTION READ ONLY").await.map_err(my_err)?;
        }
        for cmd in &spec.init_commands {
            me.conn.query_drop(cmd.as_str()).await.map_err(my_err)?;
        }
        Ok(me)
    }

    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    fn canceller(&self) -> MyCancel {
        MyCancel { opts: self.opts.clone(), id: self.conn.id(), timeout: Some(Duration::from_secs(10)) }
    }

    pub async fn execute(&mut self, sql: &str, tx: &mpsc::Sender<ExecEvent>) -> DbResult<()> {
        let res = self.run(sql, tx).await;
        self.in_tx = match self.conn.last_ok_packet() {
            Some(ok) => ok.status_flags().contains(StatusFlags::SERVER_STATUS_IN_TRANS),
            None => match classify::transaction_effect(sql, B) {
                TxEffect::Begin => res.is_ok() || self.in_tx,
                TxEffect::End => false,
                TxEffect::None => self.in_tx,
            },
        };
        if res.is_ok()
            && let Some(db) = classify::use_database(sql, B)
        {
            self.info.database = Some(db);
        }
        res
    }

    async fn abandon(cancel: MyCancel, qr: QueryResult<'_, 'static, TextProtocol>) -> DbError {
        let _ = cancel.cancel().await;
        let _ = qr.drop_result().await;
        DbError::new(ErrorKind::Cancelled, "canceled: result consumer went away")
    }

    async fn run(&mut self, sql: &str, tx: &mpsc::Sender<ExecEvent>) -> DbResult<()> {
        let cancel = self.canceller();
        let mut qr = self.conn.query_iter(sql).await.map_err(my_err)?;
        let mut had_rows = false;
        let mut total = 0u64;
        loop {
            let cols = qr.columns().filter(|c| !c.is_empty());
            let convs: Vec<Conv> = match &cols {
                Some(cols) => {
                    had_rows = true;
                    let columns = cols.iter().map(|c| Column::new(c.name_str(), type_name(c))).collect();
                    if tx.send(ExecEvent::Columns(columns)).await.is_err() {
                        return Err(Self::abandon(cancel, qr).await);
                    }
                    cols.iter().map(conv_for).collect()
                }
                None => Vec::new(),
            };
            let mut batch: Vec<Row> = Vec::with_capacity(if cols.is_some() { BATCH_ROWS } else { 0 });
            let mut since = Instant::now();
            loop {
                let row = match qr.next().await {
                    Ok(Some(row)) => row,
                    Ok(None) => break,
                    Err(e) => return Err(my_err(e)),
                };
                batch.push(row.unwrap_raw().into_iter().zip(&convs).map(|(v, c)| my_value(v, *c)).collect());
                total += 1;
                if batch.len() >= BATCH_ROWS || since.elapsed() >= FLUSH_AFTER {
                    if !flush(&mut batch, tx).await {
                        return Err(Self::abandon(cancel, qr).await);
                    }
                    since = Instant::now();
                }
            }
            if !flush(&mut batch, tx).await {
                return Err(Self::abandon(cancel, qr).await);
            }
            if qr.is_empty() {
                break;
            }
        }
        let affected = qr.affected_rows();
        let last_insert_id = qr.last_insert_id().filter(|&id| id > 0);
        let warnings = qr.warnings() as u32;
        drop(qr);
        if warnings > 0 {
            let rows: Vec<(String, u32, String)> = self.conn.query("SHOW WARNINGS").await.unwrap_or_default();
            for (level, code, message) in rows {
                let _ = tx.send(ExecEvent::Notice(Notice { severity: level, message: format!("{message} ({code})") })).await;
            }
        }
        let summary = if had_rows {
            Summary {
                status: Some(format!("{} in set", plural(total, "row"))),
                rows_affected: Some(total),
                last_insert_id,
                warnings,
            }
        } else {
            Summary {
                status: Some(format!("Query OK, {} affected", plural(affected, "row"))),
                rows_affected: Some(affected),
                last_insert_id,
                warnings,
            }
        };
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
        CancelHandle::MySql(self.canceller())
    }

    pub fn in_transaction(&self) -> bool {
        self.in_tx
    }

    pub async fn change_database(&mut self, database: &str) -> DbResult<()> {
        self.conn.query_drop(format!("USE {}", quote_ident(database, B))).await.map_err(my_err)?;
        self.info.database = Some(database.to_string());
        Ok(())
    }

    pub async fn load_catalog(&mut self) -> DbResult<Catalog> {
        let sql = format!(
            "SELECT DATABASE();
             SHOW DATABASES;
             SELECT t.TABLE_NAME, t.TABLE_TYPE, t.TABLE_COMMENT, t.TABLE_ROWS, {COLUMN_FIELDS}
             FROM information_schema.TABLES t
             LEFT JOIN information_schema.COLUMNS c ON c.TABLE_SCHEMA = t.TABLE_SCHEMA AND c.TABLE_NAME = t.TABLE_NAME
             WHERE t.TABLE_SCHEMA = DATABASE()
             ORDER BY t.TABLE_NAME, c.ORDINAL_POSITION;
             SELECT r.ROUTINE_NAME, r.ROUTINE_TYPE, r.DTD_IDENTIFIER,
                    GROUP_CONCAT(CONCAT_WS(' ', IF(r.ROUTINE_TYPE = 'PROCEDURE', p.PARAMETER_MODE, NULL),
                                           p.PARAMETER_NAME, p.DTD_IDENTIFIER)
                                 ORDER BY p.ORDINAL_POSITION SEPARATOR ', ')
             FROM information_schema.ROUTINES r
             LEFT JOIN information_schema.PARAMETERS p
               ON p.SPECIFIC_SCHEMA = r.ROUTINE_SCHEMA AND p.SPECIFIC_NAME = r.SPECIFIC_NAME AND p.ORDINAL_POSITION > 0
             WHERE r.ROUTINE_SCHEMA = DATABASE()
             GROUP BY r.ROUTINE_NAME, r.ROUTINE_TYPE, r.DTD_IDENTIFIER, r.SPECIFIC_NAME
             ORDER BY 1;
             {FK_SELECT} AND k.TABLE_SCHEMA = DATABASE()
             ORDER BY k.TABLE_NAME, k.CONSTRAINT_NAME, k.ORDINAL_POSITION"
        );
        let sets = collect_sets(&mut self.conn, &sql, 5).await?;
        let users: Vec<String> = self
            .conn
            .query("SELECT DISTINCT CONCAT('''', User, '''@''', Host, '''') FROM mysql.user ORDER BY 1")
            .await
            .unwrap_or_default();

        let mut cat = Catalog::empty(B);
        cat.current_database = sets[0].first().and_then(|r| go(r, 0));
        cat.databases = sets[1].iter().map(|r| gs(r, 0)).collect();
        cat.search_path = cat.current_database.iter().cloned().collect();
        cat.schemas = cat
            .databases
            .iter()
            .map(|d| SchemaInfo { name: d.clone(), relations: Vec::new(), functions: Vec::new(), types: Vec::new() })
            .collect();
        if let Some(cur) = cat.current_database.clone() {
            let mut relations: Vec<Relation> = Vec::new();
            for r in &sets[2] {
                let name = gs(r, 0);
                if relations.last().is_none_or(|rel| rel.name != name) {
                    relations.push(Relation {
                        schema: cur.clone(),
                        name,
                        kind: rel_kind(&gs(r, 1)),
                        columns: Vec::new(),
                        comment: go(r, 2).filter(|c| !c.is_empty()),
                        row_estimate: gi(r, 3),
                    });
                }
                if go(r, 4).is_some() {
                    relations.last_mut().expect("pushed above").columns.push(column_info(r, 4));
                }
            }
            let functions = sets[3]
                .iter()
                .map(|r| FunctionInfo {
                    schema: cur.clone(),
                    name: gs(r, 0),
                    args: gs(r, 3),
                    return_type: gs(r, 2),
                    kind: if gs(r, 1) == "PROCEDURE" { FunctionKind::Procedure } else { FunctionKind::Function },
                })
                .collect();
            match cat.schemas.iter_mut().find(|s| s.name == cur) {
                Some(s) => {
                    s.relations = relations;
                    s.functions = functions;
                }
                None => cat.schemas.push(SchemaInfo { name: cur, relations, functions, types: Vec::new() }),
            }
        }
        cat.foreign_keys = fk_rows(&sets[4]);
        cat.users = users;
        Ok(cat)
    }

    pub async fn list_databases(&mut self) -> DbResult<Vec<String>> {
        self.conn.query("SHOW DATABASES").await.map_err(my_err)
    }

    pub async fn list_relations(&mut self, schema: &str) -> DbResult<Vec<Relation>> {
        let sql = format!(
            "SELECT TABLE_NAME, TABLE_TYPE, TABLE_COMMENT, TABLE_ROWS FROM information_schema.TABLES
             WHERE TABLE_SCHEMA = {} ORDER BY 1",
            lit(schema)
        );
        let rows: Vec<MyRow> = self.conn.query(sql).await.map_err(my_err)?;
        Ok(rows
            .iter()
            .map(|r| Relation {
                schema: schema.to_string(),
                name: gs(r, 0),
                kind: rel_kind(&gs(r, 1)),
                columns: Vec::new(),
                comment: go(r, 2).filter(|c| !c.is_empty()),
                row_estimate: gi(r, 3),
            })
            .collect())
    }

    fn schema_or_current(&self, schema: Option<&str>) -> DbResult<String> {
        schema
            .map(str::to_string)
            .or_else(|| self.info.database.clone())
            .ok_or_else(|| DbError::query("No database selected"))
    }

    pub async fn table_details(&mut self, schema: Option<&str>, table: &str) -> DbResult<TableDetails> {
        let db = self.schema_or_current(schema)?;
        let (d, t) = (lit(&db), lit(table));
        let sql = format!(
            "SELECT TABLE_TYPE, TABLE_ROWS, DATA_LENGTH + INDEX_LENGTH, TABLE_COMMENT
             FROM information_schema.TABLES WHERE TABLE_SCHEMA = {d} AND TABLE_NAME = {t};
             SELECT {COLUMN_FIELDS} FROM information_schema.COLUMNS c
             WHERE c.TABLE_SCHEMA = {d} AND c.TABLE_NAME = {t} ORDER BY c.ORDINAL_POSITION;
             SELECT * FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = {d} AND TABLE_NAME = {t}
             ORDER BY INDEX_NAME <> 'PRIMARY', INDEX_NAME, SEQ_IN_INDEX;
             {FK_SELECT} AND ((k.TABLE_SCHEMA = {d} AND k.TABLE_NAME = {t})
                           OR (k.REFERENCED_TABLE_SCHEMA = {d} AND k.REFERENCED_TABLE_NAME = {t}))
             ORDER BY k.TABLE_SCHEMA, k.TABLE_NAME, k.CONSTRAINT_NAME, k.ORDINAL_POSITION;
             SELECT tc.CONSTRAINT_NAME, tc.CONSTRAINT_TYPE, cc.CHECK_CLAUSE
             FROM information_schema.TABLE_CONSTRAINTS tc
             LEFT JOIN information_schema.CHECK_CONSTRAINTS cc
               ON cc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA AND cc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME
             WHERE tc.TABLE_SCHEMA = {d} AND tc.TABLE_NAME = {t}
             ORDER BY tc.CONSTRAINT_TYPE <> 'PRIMARY KEY', tc.CONSTRAINT_NAME;
             SELECT TRIGGER_NAME, ACTION_TIMING, EVENT_MANIPULATION, ACTION_STATEMENT
             FROM information_schema.TRIGGERS WHERE EVENT_OBJECT_SCHEMA = {d} AND EVENT_OBJECT_TABLE = {t}
             ORDER BY TRIGGER_NAME;
             SELECT VIEW_DEFINITION FROM information_schema.VIEWS WHERE TABLE_SCHEMA = {d} AND TABLE_NAME = {t}"
        );
        let sets = collect_sets(&mut self.conn, &sql, 7).await?;
        let info = sets[0].first().ok_or_else(|| DbError::query(format!("Table '{db}.{table}' doesn't exist")))?;
        let kind = rel_kind(&gs(info, 0));

        let mut indexes: Vec<IndexInfo> = Vec::new();
        if let Some(first) = sets[2].first() {
            let cols = first.columns_ref();
            let pos = |name: &str| cols.iter().position(|c| c.name_str().eq_ignore_ascii_case(name));
            let (name_i, non_unique_i, col_i, type_i, expr_i) =
                (pos("INDEX_NAME"), pos("NON_UNIQUE"), pos("COLUMN_NAME"), pos("INDEX_TYPE"), pos("EXPRESSION"));
            for r in &sets[2] {
                let name = name_i.map(|i| gs(r, i)).unwrap_or_default();
                let column = col_i
                    .and_then(|i| go(r, i))
                    .or_else(|| expr_i.and_then(|i| go(r, i)).map(|e| format!("({e})")))
                    .unwrap_or_default();
                match indexes.last_mut() {
                    Some(ix) if ix.name == name => ix.columns.push(column),
                    _ => indexes.push(IndexInfo {
                        primary: name == "PRIMARY",
                        unique: non_unique_i.and_then(|i| gi(r, i)) == Some(0),
                        method: type_i.and_then(|i| go(r, i)),
                        columns: vec![column],
                        definition: None,
                        name,
                    }),
                }
            }
        }
        let (mut foreign_keys, mut referenced_by) = (Vec::new(), Vec::new());
        for fk in fk_rows(&sets[3]) {
            let outgoing = fk.schema == db && fk.table == table;
            let incoming = fk.ref_schema == db && fk.ref_table == table;
            if outgoing && incoming {
                referenced_by.push(fk.clone());
            }
            if outgoing { foreign_keys.push(fk) } else if incoming { referenced_by.push(fk) }
        }
        let quoted = |cols: &[String]| cols.iter().map(|c| quote_ident(c, B)).collect::<Vec<_>>().join(", ");
        let constraints = sets[4]
            .iter()
            .map(|r| {
                let (name, ctype) = (gs(r, 0), gs(r, 1));
                let definition = match ctype.as_str() {
                    "PRIMARY KEY" | "UNIQUE" => {
                        let cols = indexes.iter().find(|i| i.name == name).map(|i| quoted(&i.columns)).unwrap_or_default();
                        format!("{ctype} ({cols})")
                    }
                    "FOREIGN KEY" => foreign_keys
                        .iter()
                        .find(|f: &&ForeignKey| f.name == name)
                        .map(|f| {
                            let mut s = format!(
                                "FOREIGN KEY ({}) REFERENCES {} ({})",
                                quoted(&f.columns),
                                qualified(Some(&f.ref_schema), &f.ref_table, B),
                                quoted(&f.ref_columns)
                            );
                            if let Some(a) = &f.on_delete {
                                s.push_str(&format!(" ON DELETE {a}"));
                            }
                            if let Some(a) = &f.on_update {
                                s.push_str(&format!(" ON UPDATE {a}"));
                            }
                            s
                        })
                        .unwrap_or_else(|| "FOREIGN KEY".into()),
                    "CHECK" => format!("CHECK ({})", gs(r, 2)),
                    _ => ctype.clone(),
                };
                ConstraintInfo { name, kind: ctype, definition }
            })
            .collect();
        let triggers = sets[5]
            .iter()
            .map(|r| {
                let (name, timing, event) = (gs(r, 0), gs(r, 1), gs(r, 2));
                TriggerInfo {
                    definition: format!(
                        "CREATE TRIGGER {} {timing} {event} ON {} FOR EACH ROW {}",
                        quote_ident(&name, B),
                        qualified(Some(&db), table, B),
                        gs(r, 3)
                    ),
                    event: format!("{timing} {event}"),
                    name,
                }
            })
            .collect();
        Ok(TableDetails {
            schema: db.clone(),
            name: table.to_string(),
            kind,
            columns: sets[1].iter().map(|r| column_info(r, 0)).collect(),
            indexes,
            foreign_keys,
            referenced_by,
            constraints,
            triggers,
            row_estimate: if kind.is_view() { None } else { gi(info, 1) },
            size_bytes: gi(info, 2),
            comment: go(info, 3).filter(|c| !c.is_empty()),
            view_definition: sets[6].first().and_then(|r| go(r, 0)),
        })
    }

    pub async fn object_ddl(&mut self, schema: Option<&str>, name: &str, kind: &str) -> DbResult<String> {
        let target = match schema {
            Some(s) => qualified(Some(s), name, B),
            None => quote_ident(name, B),
        };
        let what = match kind.trim().to_ascii_lowercase().as_str() {
            "view" => "VIEW",
            "function" => "FUNCTION",
            "procedure" => "PROCEDURE",
            "trigger" => "TRIGGER",
            "event" => "EVENT",
            _ => "TABLE",
        };
        let mut qr = self.conn.query_iter(format!("SHOW CREATE {what} {target}")).await.map_err(my_err)?;
        let rows: Vec<MyRow> = qr.collect().await.map_err(my_err)?;
        drop(qr);
        let row = rows.first().ok_or_else(|| DbError::query(format!("{what} {target} not found")))?;
        let idx = row
            .columns_ref()
            .iter()
            .position(|c| {
                let n = c.name_str();
                n.starts_with("Create ") || n == "SQL Original Statement"
            })
            .ok_or_else(|| DbError::other("unexpected SHOW CREATE output"))?;
        let ddl = go(row, idx).ok_or_else(|| DbError::query(format!("no definition available for {target}")))?;
        Ok(format!("{};\n", ddl.trim_end().trim_end_matches(';')))
    }

    pub async fn activity(&mut self) -> DbResult<ResultSet> {
        self.collect(
            "SELECT ID AS id, USER AS user, HOST AS host, DB AS db, COMMAND AS command, TIME AS time,
                    STATE AS state, INFO AS query
             FROM information_schema.PROCESSLIST WHERE ID <> CONNECTION_ID() ORDER BY ID",
        )
        .await
    }

    pub async fn kill_session(&mut self, id: &str) -> DbResult<()> {
        let id: u64 = id.trim().parse().map_err(|_| DbError::other(format!("invalid connection id '{id}'")))?;
        self.conn.query_drop(format!("KILL {id}")).await.map_err(my_err)
    }

    pub async fn explain(&mut self, sql: &str, analyze: bool) -> DbResult<PlanNode> {
        let body = sql.trim().trim_end_matches(';').trim_end();
        let tree = analyze && !self.info.is_mariadb && self.version >= (8, 0, 18);
        let stmt = match (analyze, self.info.is_mariadb) {
            (true, true) => format!("ANALYZE FORMAT=JSON {body}"),
            _ if tree => format!("EXPLAIN ANALYZE {body}"),
            _ => format!("EXPLAIN FORMAT=JSON {body}"),
        };
        let executes = analyze && (tree || self.info.is_mariadb);
        let wrap = executes && !classify::is_read_only(body, B);
        if wrap {
            let begin = if self.in_tx { "SAVEPOINT quarry_explain" } else { "START TRANSACTION" };
            self.conn.query_drop(begin).await.map_err(my_err)?;
        }
        let res: Result<Vec<Option<String>>, _> = self.conn.query(stmt).await;
        if wrap {
            let end = if self.in_tx { "ROLLBACK TO SAVEPOINT quarry_explain" } else { "ROLLBACK" };
            self.conn.query_drop(end).await.map_err(my_err)?;
        }
        let text = res.map_err(my_err)?.into_iter().flatten().collect::<Vec<_>>().join("\n");
        if tree {
            return Ok(parse_tree(&text));
        }
        let json: Json =
            serde_json::from_str(&text).map_err(|e| DbError::other(format!("unexpected EXPLAIN output: {e}")))?;
        let top = json.as_object().ok_or_else(|| DbError::other("unexpected EXPLAIN output"))?;
        Ok(match top.iter().next() {
            Some((k, Json::Object(o))) if top.len() == 1 => json_node(k, o),
            _ => json_node("query_block", top),
        })
    }
}

fn num(v: Option<&Json>) -> Option<f64> {
    match v? {
        Json::Number(n) => n.as_f64(),
        Json::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn json_label(key: &str, o: &Map<String, Json>) -> String {
    let s = |k: &str| o.get(k).and_then(Json::as_str);
    match key {
        "query_block" => match o.get("select_id") {
            Some(id) => format!("Query block #{}", json_text(id)),
            None => "Query block".into(),
        },
        "table" => {
            let table = s("table_name").unwrap_or("?");
            let access = s("access_type").unwrap_or("");
            let mut label = match access {
                "ALL" => format!("Table scan on {table}"),
                "index" => format!("Index scan on {table}"),
                "" => format!("Table {table}"),
                a => format!("{a} access on {table}"),
            };
            if let Some(k) = s("key") {
                label.push_str(&format!(" using {k}"));
            }
            label
        }
        "nested_loop" => "Nested loop".into(),
        "ordering_operation" if o.get("using_filesort").and_then(Json::as_bool) == Some(true) => "Sort".into(),
        "ordering_operation" => "Ordering".into(),
        "grouping_operation" => "Group".into(),
        "duplicates_removal" => "Distinct".into(),
        "union_result" => "Union".into(),
        "materialized_from_subquery" => "Materialize".into(),
        "windowing" => "Window".into(),
        other => {
            let t = other.replace('_', " ");
            let mut c = t.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

fn json_node(key: &str, o: &Map<String, Json>) -> PlanNode {
    let mut node = PlanNode { label: json_label(key, o), ..Default::default() };
    for (k, v) in o {
        match v {
            Json::Object(c) if k == "cost_info" => {
                node.total_cost = num(c.get("query_cost")).or(num(c.get("prefix_cost")));
                for (ck, cv) in c {
                    node.details.push((ck.clone(), json_text(cv)));
                }
            }
            Json::Object(c) => node.children.push(json_node(k, c)),
            Json::Array(items) if !items.is_empty() && items.iter().all(Json::is_object) => {
                let mut group = PlanNode { label: json_label(k, &Map::new()), ..Default::default() };
                for item in items.iter().filter_map(Json::as_object) {
                    match item.iter().next() {
                        Some((ik, Json::Object(inner))) if item.len() == 1 => group.children.push(json_node(ik, inner)),
                        _ => group.children.push(json_node(k, item)),
                    }
                }
                node.children.push(group);
            }
            _ => node.details.push((k.clone(), json_text(v))),
        }
    }
    node.plan_rows = num(o.get("rows_examined_per_scan")).or(num(o.get("rows")));
    node.actual_rows = num(o.get("r_rows"));
    node.actual_time_ms = num(o.get("r_total_time_ms"));
    node.loops = num(o.get("r_loops"));
    node
}

fn tree_stats(seg: &str, node: &mut PlanNode, actual: bool) {
    for part in seg.split_whitespace() {
        let Some((k, v)) = part.split_once('=') else { continue };
        let (first, last) = match v.split_once("..") {
            Some((a, b)) => (a.parse().ok(), b.parse().ok()),
            None => (v.parse().ok(), v.parse().ok()),
        };
        match (k, actual) {
            ("cost", false) => {
                node.startup_cost = if v.contains("..") { first } else { None };
                node.total_cost = last;
            }
            ("rows", false) => node.plan_rows = last,
            ("time", true) => node.actual_time_ms = last,
            ("rows", true) => node.actual_rows = last,
            ("loops", true) => node.loops = last,
            _ => {}
        }
    }
}

fn tree_line(body: &str) -> PlanNode {
    let mut node = PlanNode::default();
    let cut = ["  (cost=", " (actual time=", " (never executed)"]
        .iter()
        .filter_map(|m| body.find(m))
        .min()
        .unwrap_or(body.len());
    node.label = body[..cut].trim().to_string();
    let rest = &body[cut..];
    if let Some(i) = rest.find("(cost=") {
        let seg = &rest[i + 1..];
        tree_stats(&seg[..seg.find(')').unwrap_or(seg.len())], &mut node, false);
    }
    if let Some(i) = rest.find("(actual ") {
        let seg = &rest[i + 8..];
        tree_stats(&seg[..seg.find(')').unwrap_or(seg.len())], &mut node, true);
    }
    if rest.contains("(never executed)") {
        node.details.push(("never executed".into(), "true".into()));
    }
    node
}

/// Parses MySQL's `EXPLAIN ANALYZE` tree text (`-> ...` lines, nesting by indentation).
fn parse_tree(text: &str) -> PlanNode {
    fn attach(stack: &mut [(usize, PlanNode)], roots: &mut Vec<PlanNode>, node: PlanNode) {
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(node),
            None => roots.push(node),
        }
    }
    let mut stack: Vec<(usize, PlanNode)> = Vec::new();
    let mut roots = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let Some(body) = trimmed.strip_prefix("-> ") else {
            if let Some((_, n)) = stack.last_mut().filter(|_| !trimmed.is_empty()) {
                n.label.push(' ');
                n.label.push_str(trimmed.trim_end());
            }
            continue;
        };
        while stack.last().is_some_and(|(i, _)| *i >= indent) {
            let (_, done) = stack.pop().expect("checked");
            attach(&mut stack, &mut roots, done);
        }
        stack.push((indent, tree_line(body)));
    }
    while let Some((_, done)) = stack.pop() {
        attach(&mut stack, &mut roots, done);
    }
    if roots.len() == 1 {
        roots.pop().expect("one root")
    } else {
        PlanNode { label: "Query plan".into(), children: roots, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explain_analyze_tree_nests_by_indentation() {
        let text = "-> Nested loop inner join  (cost=0.70 rows=1) (actual time=0.05..0.06 rows=2 loops=1)
    -> Table scan on a  (cost=0.35 rows=1) (actual time=0.02..0.03 rows=2 loops=1)
    -> Filter: (b.id = a.id)  (cost=0.25..0.35 rows=1) (actual time=0.01..0.012 rows=1 loops=2)
        -> Index lookup on b using PRIMARY (id=a.id)  (never executed)";
        let root = parse_tree(text);
        assert_eq!(root.label, "Nested loop inner join");
        assert_eq!(root.actual_rows, Some(2.0));
        assert_eq!(root.children.len(), 2);
        let filter = &root.children[1];
        assert_eq!(filter.startup_cost, Some(0.25));
        assert_eq!(filter.total_cost, Some(0.35));
        assert_eq!(filter.loops, Some(2.0));
        assert_eq!(filter.children[0].label, "Index lookup on b using PRIMARY (id=a.id)");
    }

    #[test]
    fn json_plan_unwraps_nested_loops() {
        let j: Json = serde_json::json!({"query_block": {"select_id": 1, "cost_info": {"query_cost": "2.10"},
            "nested_loop": [
                {"table": {"table_name": "a", "access_type": "ALL", "rows_examined_per_scan": 3}},
                {"table": {"table_name": "b", "access_type": "eq_ref", "key": "PRIMARY"}}
            ]}});
        let (k, v) = j.as_object().unwrap().iter().next().unwrap();
        let n = json_node(k, v.as_object().unwrap());
        assert_eq!(n.label, "Query block #1");
        assert_eq!(n.total_cost, Some(2.1));
        let nl = &n.children[0];
        assert_eq!(nl.label, "Nested loop");
        assert_eq!(nl.children[0].label, "Table scan on a");
        assert_eq!(nl.children[0].plan_rows, Some(3.0));
        assert_eq!(nl.children[1].label, "eq_ref access on b using PRIMARY");
    }

    #[test]
    fn text_values_follow_column_conversion() {
        use mysql_async::Value as V;
        assert_eq!(my_value(Some(V::Bytes(b"-5".to_vec())), Conv::Int), Value::Int(-5));
        assert_eq!(my_value(Some(V::Bytes(b"18446744073709551615".to_vec())), Conv::UInt), Value::UInt(u64::MAX));
        assert_eq!(my_value(Some(V::Bytes(vec![1, 0])), Conv::Bit), Value::Int(256));
        assert_eq!(my_value(Some(V::Bytes(vec![0xff])), Conv::Text), Value::Bytes(vec![0xff]));
        assert_eq!(my_value(Some(V::Bytes(b"1.50".to_vec())), Conv::Text), Value::Text("1.50".into()));
        assert_eq!(my_value(Some(V::NULL), Conv::Int), Value::Null);
    }
}
