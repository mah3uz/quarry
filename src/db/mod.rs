pub mod catalog;
pub mod mysql;
pub mod postgres;
pub mod sqlite;
pub mod tls;

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

pub use catalog::*;

use crate::conn::ConnSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Postgres,
    #[serde(alias = "mariadb")]
    MySql,
    Sqlite,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Postgres => "PostgreSQL",
            Backend::MySql => "MySQL",
            Backend::Sqlite => "SQLite",
        }
    }

    pub fn default_port(self) -> Option<u16> {
        match self {
            Backend::Postgres => Some(5432),
            Backend::MySql => Some(3306),
            Backend::Sqlite => None,
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    /// Also carries exact numerics (NUMERIC/DECIMAL), dates, JSON, UUIDs… as the server printed them.
    Text(String),
    Bytes(Vec<u8>),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Display text without any null placeholder substitution (`Null` → empty).
    pub fn display(&self) -> Cow<'_, str> {
        match self {
            Value::Null => Cow::Borrowed(""),
            Value::Bool(b) => Cow::Borrowed(if *b { "true" } else { "false" }),
            Value::Int(i) => Cow::Owned(i.to_string()),
            Value::UInt(u) => Cow::Owned(u.to_string()),
            Value::Float(f) => Cow::Owned(format_float(*f)),
            Value::Text(s) => Cow::Borrowed(s),
            Value::Bytes(b) => match std::str::from_utf8(b) {
                Ok(s) if !s.chars().any(|c| c.is_control() && !c.is_whitespace()) => Cow::Borrowed(s),
                _ => Cow::Owned(hex_bytes(b)),
            },
        }
    }

    /// SQL literal for generated statements (row edits, `sql-insert` output).
    pub fn to_sql_literal(&self, backend: Backend) -> String {
        match self {
            Value::Null => "NULL".into(),
            Value::Bool(b) => match backend {
                Backend::Postgres => if *b { "TRUE" } else { "FALSE" }.into(),
                _ => if *b { "1" } else { "0" }.into(),
            },
            Value::Int(i) => i.to_string(),
            Value::UInt(u) => u.to_string(),
            Value::Float(f) if f.is_finite() => format_float(*f),
            Value::Float(f) => quote_literal(&f.to_string(), backend),
            Value::Text(s) => quote_literal(s, backend),
            Value::Bytes(b) => match backend {
                Backend::Postgres => format!("'\\x{}'::bytea", hex_plain(b)),
                _ => format!("X'{}'", hex_plain(b)),
            },
        }
    }
}

fn format_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e16 {
        format!("{f:.1}")
    } else {
        f.to_string()
    }
}

fn hex_plain(b: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(b.len() * 2);
    for byte in b {
        let _ = write!(s, "{byte:02x}");
    }
    s
}

pub fn hex_bytes(b: &[u8]) -> String {
    format!("0x{}", hex_plain(b))
}

/// Quotes a string literal. MySQL also needs backslashes doubled (unless NO_BACKSLASH_ESCAPES).
pub fn quote_literal(s: &str, backend: Backend) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' if backend == Backend::MySql => out.push_str("\\\\"),
            '\0' if backend == Backend::MySql => out.push_str("\\0"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Quotes an identifier only when needed (keeps generated SQL readable).
pub fn quote_ident(name: &str, backend: Backend) -> String {
    let plain = !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !is_reserved(name, backend);
    // Postgres folds unquoted names to lower case, so mixed case must stay quoted there.
    let case_safe = backend != Backend::Postgres || !name.chars().any(|c| c.is_ascii_uppercase());
    if plain && case_safe {
        return name.to_string();
    }
    match backend {
        Backend::MySql => format!("`{}`", name.replace('`', "``")),
        _ => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

const RESERVED: &[&str] = &[
    "ALL", "ALTER", "ANALYZE", "AND", "ANY", "ARRAY", "AS", "ASC", "BETWEEN", "BOTH", "BY", "CASE", "CAST", "CHECK",
    "COLLATE", "COLUMN", "CONSTRAINT", "CREATE", "CROSS", "CURRENT_DATE", "CURRENT_ROLE", "CURRENT_TIME",
    "CURRENT_TIMESTAMP", "CURRENT_USER", "DEFAULT", "DEFERRABLE", "DELETE", "DESC", "DISTINCT", "DO", "DROP", "ELSE",
    "END", "EXCEPT", "EXISTS", "FALSE", "FETCH", "FOR", "FOREIGN", "FROM", "FULL", "GRANT", "GROUP", "HAVING", "IN",
    "INDEX", "INNER", "INSERT", "INTERSECT", "INTO", "IS", "JOIN", "KEY", "LATERAL", "LEADING", "LEFT", "LIKE", "LIMIT",
    "LOCALTIME", "LOCALTIMESTAMP", "NATURAL", "NOT", "NULL", "OFFSET", "ON", "ONLY", "OR", "ORDER", "OUTER", "PRIMARY",
    "REFERENCES", "RETURNING", "RIGHT", "SELECT", "SESSION_USER", "SET", "SOME", "TABLE", "THEN", "TO", "TRAILING",
    "TRUE", "UNION", "UNIQUE", "UPDATE", "USER", "USING", "VALUES", "WHEN", "WHERE", "WINDOW", "WITH",
];

const MYSQL_RESERVED: &[&str] = &[
    "CHANGE", "CONDITION", "DATABASE", "DATABASES", "DELAYED", "DESCRIBE", "DIV", "DUAL", "EXPLAIN", "FORCE",
    "FULLTEXT", "IF", "IGNORE", "INTERVAL", "KEYS", "KILL", "LINES", "LOAD", "LOCK", "MATCH", "MOD", "OPTION",
    "OUTFILE", "PARTITION", "RANGE", "READ", "REGEXP", "RENAME", "REPLACE", "REQUIRE", "RLIKE", "SCHEMA", "SHOW",
    "SPATIAL", "SQL", "STRAIGHT_JOIN", "TERMINATED", "UNLOCK", "UNSIGNED", "USAGE", "USE", "WRITE", "XOR", "ZEROFILL",
    "RANK", "ROW", "ROWS",
];

const SQLITE_RESERVED: &[&str] = &["AUTOINCREMENT", "GLOB", "INDEXED", "ISNULL", "NOTNULL", "PRAGMA", "RAISE", "REGEXP", "VACUUM"];

fn is_reserved(name: &str, backend: Backend) -> bool {
    let up = name.to_ascii_uppercase();
    let up = up.as_str();
    RESERVED.contains(&up)
        || match backend {
            Backend::MySql => MYSQL_RESERVED.contains(&up),
            Backend::Sqlite => SQLITE_RESERVED.contains(&up),
            Backend::Postgres => false,
        }
}

/// `schema.table` with each part quoted as needed; schema omitted when `None`.
pub fn qualified(schema: Option<&str>, name: &str, backend: Backend) -> String {
    match schema {
        Some(s) if !s.is_empty() => format!("{}.{}", quote_ident(s, backend), quote_ident(name, backend)),
        _ => quote_ident(name, backend),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Integer,
    Float,
    Decimal,
    Bool,
    Text,
    Date,
    Time,
    Timestamp,
    Interval,
    Json,
    Uuid,
    Binary,
    Array,
    Other,
}

impl TypeKind {
    pub fn is_numeric(self) -> bool {
        matches!(self, TypeKind::Integer | TypeKind::Float | TypeKind::Decimal)
    }

    pub fn is_temporal(self) -> bool {
        matches!(self, TypeKind::Date | TypeKind::Time | TypeKind::Timestamp | TypeKind::Interval)
    }

    /// Best-effort mapping from a declared/server type name (any backend, any case).
    pub fn from_type_name(name: &str) -> TypeKind {
        let n = name.trim().to_ascii_lowercase();
        let base = n.split(['(', ' ']).next().unwrap_or("");
        if n.ends_with("[]") || base.starts_with('_') {
            return TypeKind::Array;
        }
        match base {
            "int" | "int2" | "int4" | "int8" | "integer" | "smallint" | "bigint" | "tinyint" | "mediumint"
            | "serial" | "bigserial" | "smallserial" | "oid" | "year" | "long" | "longlong" | "short" | "int24"
            | "bit" => TypeKind::Integer,
            "real" | "float" | "float4" | "float8" | "double" => TypeKind::Float,
            "numeric" | "decimal" | "dec" | "money" | "newdecimal" => TypeKind::Decimal,
            "bool" | "boolean" => TypeKind::Bool,
            "date" | "newdate" => TypeKind::Date,
            "time" | "timetz" => TypeKind::Time,
            "timestamp" | "timestamptz" | "datetime" | "datetime2" => TypeKind::Timestamp,
            "interval" => TypeKind::Interval,
            "json" | "jsonb" => TypeKind::Json,
            "uuid" => TypeKind::Uuid,
            "bytea" | "blob" | "tinyblob" | "mediumblob" | "longblob" | "binary" | "varbinary" | "geometry" => {
                TypeKind::Binary
            }
            "text" | "varchar" | "char" | "character" | "bpchar" | "name" | "citext" | "tinytext" | "mediumtext"
            | "longtext" | "string" | "var_string" | "enum" | "set" | "nvarchar" | "nchar" | "clob" => TypeKind::Text,
            _ if base.contains("int") => TypeKind::Integer,
            _ if base.contains("char") || base.contains("text") || base.contains("clob") => TypeKind::Text,
            _ if base.contains("real") || base.contains("floa") || base.contains("doub") => TypeKind::Float,
            _ => TypeKind::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    /// Server type name as best known (`int4`, `VARCHAR`, `` for untyped sqlite expressions).
    pub type_name: String,
    pub kind: TypeKind,
}

impl Column {
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        let type_name = type_name.into();
        Column { name: name.into(), kind: TypeKind::from_type_name(&type_name), type_name }
    }
}

pub type Row = Vec<Value>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// Command tag / status text, e.g. `INSERT 0 3`, `SELECT 10`, `Query OK`.
    pub status: Option<String>,
    pub rows_affected: Option<u64>,
    pub last_insert_id: Option<u64>,
    pub warnings: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub severity: String,
    pub message: String,
}

/// Streamed output of executing one statement. Order: `Columns` (only for row-returning statements),
/// zero or more `Rows` batches, then exactly one `Done`. `Notice` may appear anywhere.
/// A statement producing several result sets (MySQL `CALL`) emits a new `Columns` before each
/// subsequent set's rows; `Done` summarises the whole statement.
#[derive(Clone, Debug, PartialEq)]
pub enum ExecEvent {
    Columns(Vec<Column>),
    Rows(Vec<Row>),
    Done(Summary),
    Notice(Notice),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    pub summary: Summary,
}

impl ResultSet {
    pub fn new(columns: Vec<Column>, rows: Vec<Row>) -> Self {
        let n = rows.len() as u64;
        ResultSet { columns, rows, summary: Summary { status: None, rows_affected: Some(n), ..Default::default() } }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Bad/missing password — caller may prompt and retry.
    Auth,
    /// Could not reach/handshake with the server; connection may be dead.
    Connection,
    Query,
    Cancelled,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DbError {
    pub kind: ErrorKind,
    pub message: String,
    /// SQLSTATE (pg) / error number (mysql) / extended code (sqlite).
    pub code: Option<String>,
    pub detail: Option<String>,
    pub hint: Option<String>,
    /// 1-based character position in the statement (pg).
    pub position: Option<usize>,
}

impl DbError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        DbError { kind, message: message.into(), code: None, detail: None, hint: None, position: None }
    }

    pub fn query(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Query, message)
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Other, message)
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.code {
            Some(c) => write!(f, "{} ({c})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for DbError {}

pub type DbResult<T> = Result<T, DbError>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerInfo {
    /// e.g. `PostgreSQL 18.6`, `MySQL 8.4.11`, `MariaDB 11.4`, `SQLite 3.50.1`
    pub version: String,
    pub user: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    /// Backend session id (pg pid, mysql connection id).
    pub session_id: Option<String>,
    pub tls: bool,
    pub is_mariadb: bool,
}

/// Cheap, cloneable handle that aborts the statement currently running on its connection.
#[derive(Clone)]
pub enum CancelHandle {
    Postgres(postgres::PgCancel),
    MySql(mysql::MyCancel),
    Sqlite(sqlite::LiteCancel),
}

impl CancelHandle {
    pub async fn cancel(&self) -> DbResult<()> {
        match self {
            CancelHandle::Postgres(c) => c.cancel().await,
            CancelHandle::MySql(c) => c.cancel().await,
            CancelHandle::Sqlite(c) => c.cancel(),
        }
    }
}

/// A node of a query plan, normalized across backends for the TUI explain view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanNode {
    pub label: String,
    pub details: Vec<(String, String)>,
    pub startup_cost: Option<f64>,
    pub total_cost: Option<f64>,
    pub plan_rows: Option<f64>,
    pub actual_rows: Option<f64>,
    /// Actual total time in ms (EXPLAIN ANALYZE).
    pub actual_time_ms: Option<f64>,
    pub loops: Option<f64>,
    pub children: Vec<PlanNode>,
}

pub enum Connection {
    Postgres(postgres::PgConn),
    MySql(mysql::MyConn),
    Sqlite(sqlite::LiteConn),
}

macro_rules! dispatch {
    ($self:ident, $c:ident => $e:expr) => {
        match $self {
            Connection::Postgres($c) => $e,
            Connection::MySql($c) => $e,
            Connection::Sqlite($c) => $e,
        }
    };
}

impl Connection {
    /// Opens a connection. Applies `spec.readonly` (server-side where supported) and runs
    /// `spec.init_commands`. SSH tunnelling is resolved by the caller before this point.
    pub async fn connect(spec: &ConnSpec) -> DbResult<Connection> {
        match spec.backend {
            Backend::Postgres => postgres::PgConn::connect(spec).await.map(Connection::Postgres),
            Backend::MySql => mysql::MyConn::connect(spec).await.map(Connection::MySql),
            Backend::Sqlite => sqlite::LiteConn::connect(spec).await.map(Connection::Sqlite),
        }
    }

    pub fn backend(&self) -> Backend {
        match self {
            Connection::Postgres(_) => Backend::Postgres,
            Connection::MySql(_) => Backend::MySql,
            Connection::Sqlite(_) => Backend::Sqlite,
        }
    }

    pub fn info(&self) -> &ServerInfo {
        dispatch!(self, c => c.info())
    }

    /// Executes ONE statement (already split by `sql::split`), streaming events into `tx`.
    /// If the receiver is dropped the driver stops reading rows as soon as practical.
    pub async fn execute(&mut self, sql: &str, tx: &mpsc::Sender<ExecEvent>) -> DbResult<()> {
        dispatch!(self, c => c.execute(sql, tx).await)
    }

    /// Executes one statement and collects everything. For metadata / small queries.
    pub async fn query(&mut self, sql: &str) -> DbResult<ResultSet> {
        let (tx, mut rx) = mpsc::channel(64);
        let fut = async move {
            let tx = tx;
            self.execute(sql, &tx).await
        };
        let collect = async {
            let mut rs = ResultSet::default();
            while let Some(ev) = rx.recv().await {
                match ev {
                    ExecEvent::Columns(c) => rs.columns = c,
                    ExecEvent::Rows(mut r) => rs.rows.append(&mut r),
                    ExecEvent::Done(s) => rs.summary = s,
                    ExecEvent::Notice(_) => {}
                }
            }
            rs
        };
        let (res, rs) = tokio::join!(fut, collect);
        res.map(|_| rs)
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        dispatch!(self, c => c.cancel_handle())
    }

    /// Whether an explicit transaction is open (for the prompt / status bar).
    pub fn in_transaction(&self) -> bool {
        dispatch!(self, c => c.in_transaction())
    }

    /// Switch database: MySQL `USE`, Postgres reconnect, SQLite opens another file.
    pub async fn change_database(&mut self, database: &str) -> DbResult<()> {
        dispatch!(self, c => c.change_database(database).await)
    }

    /// Everything completion and the sidebar need, in as few round-trips as possible.
    pub async fn load_catalog(&mut self) -> DbResult<Catalog> {
        dispatch!(self, c => c.load_catalog().await)
    }

    pub async fn list_databases(&mut self) -> DbResult<Vec<String>> {
        dispatch!(self, c => c.list_databases().await)
    }

    /// Relations (without columns) of one schema/database — lazy sidebar expansion.
    pub async fn list_relations(&mut self, schema: &str) -> DbResult<Vec<Relation>> {
        dispatch!(self, c => c.list_relations(schema).await)
    }

    pub async fn table_details(&mut self, schema: Option<&str>, table: &str) -> DbResult<TableDetails> {
        dispatch!(self, c => c.table_details(schema, table).await)
    }

    /// CREATE statement for a table / view / function / index etc. `kind` like `table`, `view`, `function`.
    pub async fn object_ddl(&mut self, schema: Option<&str>, name: &str, kind: &str) -> DbResult<String> {
        dispatch!(self, c => c.object_ddl(schema, name, kind).await)
    }

    /// Server sessions (pg_stat_activity / PROCESSLIST). First column is the session id.
    pub async fn activity(&mut self) -> DbResult<ResultSet> {
        dispatch!(self, c => c.activity().await)
    }

    pub async fn kill_session(&mut self, id: &str) -> DbResult<()> {
        dispatch!(self, c => c.kill_session(id).await)
    }

    /// Runs EXPLAIN (ANALYZE if requested) and normalizes to a plan tree.
    pub async fn explain(&mut self, sql: &str, analyze: bool) -> DbResult<PlanNode> {
        dispatch!(self, c => c.explain(sql, analyze).await)
    }

    /// Loads a SQLite extension (enabled only for the duration of the call). Other backends: error.
    pub async fn load_extension(&mut self, path: &str) -> DbResult<()> {
        match self {
            Connection::Sqlite(c) => c.load_extension(path).await,
            other => Err(DbError::other(format!("loading extensions is not supported for {}", other.backend()))),
        }
    }
}

/// Gathers a statement's events into a `ResultSet` (driver-internal counterpart of `Connection::query`).
pub(crate) async fn collect_events(mut rx: mpsc::Receiver<ExecEvent>) -> ResultSet {
    let mut rs = ResultSet::default();
    while let Some(ev) = rx.recv().await {
        match ev {
            ExecEvent::Columns(c) => rs.columns = c,
            ExecEvent::Rows(mut r) => rs.rows.append(&mut r),
            ExecEvent::Done(s) => rs.summary = s,
            ExecEvent::Notice(_) => {}
        }
    }
    rs
}

/// Scalar JSON value as plain text (arrays comma-joined) for EXPLAIN plan details.
pub(crate) fn json_text(v: &serde_json::Value) -> String {
    use serde_json::Value as Json;
    match v {
        Json::String(s) => s.clone(),
        Json::Array(a) => a.iter().map(json_text).collect::<Vec<_>>().join(", "),
        Json::Null => String::new(),
        other => other.to_string(),
    }
}

/// Rows per `ExecEvent::Rows` batch.
pub(crate) const BATCH_ROWS: usize = 256;

#[cfg(test)]
mod quote_tests {
    use super::*;

    #[test]
    fn only_reserved_or_unsafe_identifiers_are_quoted() {
        assert_eq!(quote_ident("public", Backend::Postgres), "public");
        assert_eq!(quote_ident("type", Backend::Postgres), "type");
        assert_eq!(quote_ident("user", Backend::Postgres), "\"user\"");
        assert_eq!(quote_ident("UserId", Backend::Postgres), "\"UserId\"", "pg folds case");
        assert_eq!(quote_ident("UserId", Backend::MySql), "UserId");
        assert_eq!(quote_ident("order", Backend::MySql), "`order`");
        assert_eq!(quote_ident("my col", Backend::Sqlite), "\"my col\"");
        assert_eq!(quote_ident("a\"b", Backend::Sqlite), "\"a\"\"b\"");
    }
}
