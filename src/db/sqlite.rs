use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::types::ValueRef;
use rusqlite::{ErrorCode, InterruptHandle, OpenFlags, params};
use tokio::sync::{mpsc, oneshot};

use super::*;
use crate::conn::ConnSpec;
use crate::sql::classify;
use crate::sql::lexer::{TokenKind, tokenize};

const B: Backend = Backend::Sqlite;
const FLUSH_AFTER: Duration = Duration::from_millis(50);

type Job = Box<dyn FnOnce(&mut Lite) + Send>;

/// State owned by the connection's dedicated thread (rusqlite connections are !Sync).
struct Lite {
    conn: rusqlite::Connection,
    interrupt: Arc<Mutex<InterruptHandle>>,
}

pub struct LiteConn {
    jobs: std::sync::mpsc::Sender<Job>,
    info: ServerInfo,
    cancel: LiteCancel,
    in_tx: Arc<AtomicBool>,
    readonly: bool,
}

#[derive(Clone)]
pub struct LiteCancel {
    handle: Arc<Mutex<InterruptHandle>>,
}

impl LiteCancel {
    pub fn cancel(&self) -> DbResult<()> {
        self.handle.lock().unwrap_or_else(|p| p.into_inner()).interrupt();
        Ok(())
    }
}

fn lite_err(e: rusqlite::Error) -> DbError {
    lite_err_in(e, None)
}

/// `statement` lets input-error offsets (reported against a trailing fragment) map back onto it.
fn lite_err_in(e: rusqlite::Error, statement: Option<&str>) -> DbError {
    use rusqlite::Error as E;
    let kind_of = |code: ErrorCode| match code {
        ErrorCode::OperationInterrupted => ErrorKind::Cancelled,
        ErrorCode::CannotOpen | ErrorCode::NotADatabase => ErrorKind::Connection,
        _ => ErrorKind::Query,
    };
    match e {
        E::SqliteFailure(f, msg) => DbError {
            code: Some(f.extended_code.to_string()),
            ..DbError::new(kind_of(f.code), msg.unwrap_or_else(|| f.to_string()))
        },
        E::SqlInputError { error, msg, sql, offset } => DbError {
            code: Some(error.extended_code.to_string()),
            position: usize::try_from(offset).ok().and_then(|o| {
                let full = statement.filter(|s| s.ends_with(sql.as_str())).unwrap_or(&sql);
                full.get(..full.len() - sql.len() + o).map(|prefix| prefix.chars().count() + 1)
            }),
            ..DbError::new(kind_of(error.code), msg)
        },
        other => DbError::query(other.to_string()),
    }
}

fn closed() -> DbError {
    DbError::new(ErrorKind::Connection, "SQLite connection thread has stopped")
}

fn cancelled() -> DbError {
    DbError::new(ErrorKind::Cancelled, "canceled: result consumer went away")
}

fn open(path: &str, readonly: bool) -> DbResult<rusqlite::Connection> {
    let conn = if path.is_empty() || path == ":memory:" {
        rusqlite::Connection::open_in_memory()
    } else {
        let mode = if readonly {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        };
        rusqlite::Connection::open_with_flags(path, mode | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX)
    }
    .map_err(|e| DbError { kind: ErrorKind::Connection, ..lite_err(e) })?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(lite_err)?;
    if readonly {
        conn.pragma_update(None, "query_only", true).map_err(lite_err)?;
    }
    Ok(conn)
}

fn lite_value(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(f) => Value::Float(f),
        ValueRef::Text(t) => match std::str::from_utf8(t) {
            Ok(s) => Value::Text(s.to_string()),
            Err(_) => Value::Bytes(t.to_vec()),
        },
        ValueRef::Blob(b) => Value::Bytes(b.to_vec()),
    }
}

fn storage_class(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "INTEGER",
        Value::Float(_) => "REAL",
        Value::Text(_) => "TEXT",
        Value::Bytes(_) => "BLOB",
        _ => "",
    }
}

fn verb(sql: &str) -> String {
    classify::main_verb(sql, B).unwrap_or_default()
}

fn run(conn: &rusqlite::Connection, sql: &str, tx: &mpsc::Sender<ExecEvent>) -> DbResult<()> {
    let done = |status: String, rows: Option<u64>| {
        let summary = Summary { status: Some(status), rows_affected: rows, ..Default::default() };
        let _ = tx.blocking_send(ExecEvent::Done(summary));
    };
    if tokenize(sql, B).iter().all(|t| t.is_trivia() || t.kind == TokenKind::Semicolon) {
        done("OK".into(), None);
        return Ok(());
    }
    let verb = verb(sql);
    let dml = matches!(verb.as_str(), "INSERT" | "UPDATE" | "DELETE" | "REPLACE");
    let mut stmt = conn.prepare(sql).map_err(|e| lite_err_in(e, Some(sql)))?;
    let ncol = stmt.column_count();
    if ncol == 0 {
        stmt.raw_execute().map_err(lite_err)?;
        drop(stmt);
        if dml {
            let n = conn.changes();
            done(format!("{verb} {n}"), Some(n));
        } else {
            done("OK".into(), None);
        }
        return Ok(());
    }
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();
    let decls: Vec<Option<String>> = stmt.columns().iter().map(|c| c.decl_type().map(str::to_string)).collect();
    let columns_for = |first: Option<&Row>| -> Vec<Column> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let ty = decls[i].clone().unwrap_or_else(|| {
                    first.and_then(|r| r.get(i)).map(storage_class).unwrap_or_default().to_string()
                });
                Column::new(n.clone(), ty)
            })
            .collect()
    };
    let mut rows = stmt.raw_query();
    let mut batch: Vec<Row> = Vec::with_capacity(BATCH_ROWS);
    let mut sent_columns = false;
    let mut total = 0u64;
    let mut since = Instant::now();
    while let Some(row) = rows.next().map_err(lite_err)? {
        let values: Row = (0..ncol).map(|i| lite_value(row.get_ref_unwrap(i))).collect();
        if !sent_columns {
            if tx.blocking_send(ExecEvent::Columns(columns_for(Some(&values)))).is_err() {
                return Err(cancelled());
            }
            sent_columns = true;
        }
        batch.push(values);
        total += 1;
        if batch.len() >= BATCH_ROWS || since.elapsed() >= FLUSH_AFTER {
            let full = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_ROWS));
            if tx.blocking_send(ExecEvent::Rows(full)).is_err() {
                return Err(cancelled());
            }
            since = Instant::now();
        }
    }
    drop(rows);
    drop(stmt);
    if !sent_columns && tx.blocking_send(ExecEvent::Columns(columns_for(None))).is_err() {
        return Err(cancelled());
    }
    if !batch.is_empty() && tx.blocking_send(ExecEvent::Rows(batch)).is_err() {
        return Err(cancelled());
    }
    if dml {
        let n = conn.changes();
        done(format!("{verb} {n}"), Some(n));
    } else {
        let label = if matches!(verb.as_str(), "VALUES" | "WITH" | "") { "SELECT" } else { verb.as_str() };
        done(format!("{label} {total}"), Some(total));
    }
    Ok(())
}

fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn schemas(conn: &rusqlite::Connection) -> DbResult<Vec<String>> {
    let mut st = conn.prepare("SELECT name FROM pragma_database_list ORDER BY seq").map_err(lite_err)?;
    let names = st.query_map([], |r| r.get(0)).and_then(Iterator::collect).map_err(lite_err)?;
    Ok(names)
}

/// Unqualified-name resolution order used by SQLite itself: temp, main, then attached databases.
fn search_order(all: &[String]) -> Vec<String> {
    let mut out: Vec<String> = all.iter().filter(|s| *s == "temp").cloned().collect();
    out.extend(all.iter().filter(|s| *s == "main").cloned());
    out.extend(all.iter().filter(|s| *s != "temp" && *s != "main").cloned());
    out
}

fn rel_kind(ty: &str, name: &str) -> RelKind {
    match ty {
        "view" => RelKind::View,
        _ if name.starts_with("sqlite_") => RelKind::SystemTable,
        _ => RelKind::Table,
    }
}

fn column_info(r: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<ColumnInfo> {
    Ok(ColumnInfo {
        name: r.get(at)?,
        data_type: r.get::<_, Option<String>>(at + 1)?.unwrap_or_default(),
        nullable: r.get::<_, i64>(at + 2)? == 0,
        default: r.get(at + 3)?,
        primary_key: r.get::<_, i64>(at + 4)? > 0,
        auto: false,
        comment: None,
    })
}

/// A lone `INTEGER PRIMARY KEY` column aliases the rowid, i.e. it auto-assigns.
fn mark_rowid_alias(columns: &mut [ColumnInfo]) {
    let mut pk = columns.iter_mut().filter(|c| c.primary_key);
    if let (Some(only), None) = (pk.next(), pk.next()) {
        only.auto = only.data_type.eq_ignore_ascii_case("integer");
    }
}

fn relations(conn: &rusqlite::Connection, schema: &str, with_columns: bool) -> DbResult<Vec<Relation>> {
    let s = ident(schema);
    let mut out: Vec<Relation> = Vec::new();
    if !with_columns {
        let mut st = conn
            .prepare(&format!("SELECT name, type FROM {s}.sqlite_schema WHERE type IN ('table', 'view') ORDER BY name"))
            .map_err(lite_err)?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(lite_err)?;
        for row in rows {
            let (name, ty) = row.map_err(lite_err)?;
            out.push(Relation {
                schema: schema.to_string(),
                kind: rel_kind(&ty, &name),
                name,
                columns: Vec::new(),
                comment: None,
                row_estimate: None,
            });
        }
        return Ok(out);
    }
    let mut st = conn
        .prepare(&format!(
            "SELECT m.name, m.type, p.name, p.type, p.\"notnull\", p.dflt_value, p.pk, p.hidden
             FROM {s}.sqlite_schema m LEFT JOIN pragma_table_xinfo(m.name, ?1) p
             WHERE m.type IN ('table', 'view')
             ORDER BY m.name, p.cid"
        ))
        .map_err(lite_err)?;
    let mut rows = st.query(params![schema]).map_err(lite_err)?;
    while let Some(r) = rows.next().map_err(lite_err)? {
        let name: String = r.get(0).map_err(lite_err)?;
        if out.last().is_none_or(|rel| rel.name != name) {
            if let Some(prev) = out.last_mut() {
                mark_rowid_alias(&mut prev.columns);
            }
            let ty: String = r.get(1).map_err(lite_err)?;
            out.push(Relation {
                schema: schema.to_string(),
                kind: rel_kind(&ty, &name),
                name,
                columns: Vec::new(),
                comment: None,
                row_estimate: None,
            });
        }
        let hidden: Option<i64> = r.get(7).map_err(lite_err)?;
        if r.get_ref(2).map_err(lite_err)? == ValueRef::Null || hidden == Some(1) {
            continue;
        }
        let col = column_info(r, 2).map_err(lite_err)?;
        out.last_mut().expect("pushed above").columns.push(col);
    }
    if let Some(prev) = out.last_mut() {
        mark_rowid_alias(&mut prev.columns);
    }
    Ok(out)
}

fn action(a: String) -> Option<String> {
    (a != "NO ACTION").then_some(a)
}

/// FKs of `schema`; with `target`, only those referencing that table.
fn foreign_keys(
    conn: &rusqlite::Connection,
    schema: &str,
    only_table: Option<&str>,
    target: Option<&str>,
) -> DbResult<Vec<ForeignKey>> {
    let s = ident(schema);
    let mut st = conn
        .prepare(&format!(
            "SELECT m.name, f.id, f.\"table\", f.\"from\", f.\"to\", f.on_update, f.on_delete
             FROM {s}.sqlite_schema m JOIN pragma_foreign_key_list(m.name, ?1) f
             WHERE m.type = 'table'
               AND (?2 IS NULL OR m.name = ?2 COLLATE NOCASE)
               AND (?3 IS NULL OR f.\"table\" = ?3 COLLATE NOCASE)
             ORDER BY m.name, f.id, f.seq"
        ))
        .map_err(lite_err)?;
    let mut rows = st.query(params![schema, only_table, target]).map_err(lite_err)?;
    let mut out: Vec<(i64, ForeignKey)> = Vec::new();
    while let Some(r) = rows.next().map_err(lite_err)? {
        let get = |i| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default).map_err(lite_err);
        let table = get(0)?;
        let id: i64 = r.get(1).map_err(lite_err)?;
        let (from, to) = (get(3)?, get(4)?);
        match out.last_mut() {
            Some((last_id, fk)) if *last_id == id && fk.table == table => {
                fk.columns.push(from);
                fk.ref_columns.push(to);
            }
            _ => {
                let ref_table = get(2)?;
                out.push((
                    id,
                    ForeignKey {
                        name: format!("{table}_fk{id}"),
                        schema: schema.to_string(),
                        table,
                        columns: vec![from],
                        ref_schema: schema.to_string(),
                        ref_table,
                        ref_columns: vec![to],
                        on_update: action(get(5)?),
                        on_delete: action(get(6)?),
                    },
                ))
            }
        }
    }
    drop(rows);
    let mut fks: Vec<ForeignKey> = out.into_iter().map(|(_, fk)| fk).collect();
    for fk in &mut fks {
        // `REFERENCES t` without a column list targets t's primary key.
        if fk.ref_columns.iter().all(String::is_empty) {
            let mut st = conn
                .prepare("SELECT name FROM pragma_table_info(?1, ?2) WHERE pk > 0 ORDER BY pk")
                .map_err(lite_err)?;
            let pk: Vec<String> = st
                .query_map(params![fk.ref_table, schema], |r| r.get(0))
                .and_then(Iterator::collect)
                .map_err(lite_err)?;
            if pk.len() == fk.columns.len() {
                fk.ref_columns = pk;
            }
        }
    }
    Ok(fks)
}

fn functions(conn: &rusqlite::Connection) -> Vec<FunctionInfo> {
    let Ok(mut st) = conn.prepare("SELECT name, type, narg FROM pragma_function_list ORDER BY name") else {
        return Vec::new();
    };
    let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
    else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    rows.flatten()
        .filter(|(name, _, narg)| seen.insert((name.clone(), *narg)))
        .map(|(name, ty, narg)| FunctionInfo {
            schema: "main".into(),
            name,
            args: if narg < 0 { "...".into() } else { vec!["?"; narg as usize].join(", ") },
            return_type: String::new(),
            kind: match ty.as_str() {
                "a" => FunctionKind::Aggregate,
                "w" => FunctionKind::Window,
                _ => FunctionKind::Function,
            },
        })
        .collect()
}

fn catalog(conn: &rusqlite::Connection) -> DbResult<Catalog> {
    let names = schemas(conn)?;
    let mut cat = Catalog::empty(B);
    for name in &names {
        let relations = relations(conn, name, true)?;
        if name == "temp" && relations.is_empty() {
            continue;
        }
        cat.foreign_keys.extend(foreign_keys(conn, name, None, None)?);
        cat.schemas.push(SchemaInfo { name: name.clone(), relations, functions: Vec::new(), types: Vec::new() });
    }
    if let Some(main) = cat.schemas.iter_mut().find(|s| s.name == "main") {
        main.functions = functions(conn);
    }
    let present: Vec<String> = cat.schemas.iter().map(|s| s.name.clone()).collect();
    cat.search_path = search_order(&present);
    cat.databases = names;
    cat.current_database = Some("main".into());
    Ok(cat)
}

/// Finds which attached database holds `name` (SQLite's own resolution order when unqualified).
fn locate(conn: &rusqlite::Connection, schema: Option<&str>, name: &str) -> DbResult<(String, String, String, Option<String>)> {
    let candidates = match schema {
        Some(s) => vec![s.to_string()],
        None => search_order(&schemas(conn)?),
    };
    for s in candidates {
        let found = conn
            .query_row(
                &format!("SELECT name, type, sql FROM {}.sqlite_schema WHERE name = ?1 COLLATE NOCASE", ident(&s)),
                params![name],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)),
            )
            .ok();
        if let Some((n, ty, sql)) = found {
            return Ok((s, n, ty, sql));
        }
    }
    Err(DbError::query(format!("no such table: {name}")))
}

fn trigger_event(sql: &str) -> String {
    let words: Vec<String> = tokenize(sql, B)
        .into_iter()
        .filter(|t| t.is_word())
        .take(24)
        .map(|t| t.text(sql).to_ascii_uppercase())
        .collect();
    let timing = if words.iter().any(|w| w == "INSTEAD") {
        "INSTEAD OF"
    } else if words.iter().any(|w| w == "AFTER") {
        "AFTER"
    } else {
        "BEFORE"
    };
    let event = words.iter().find(|w| matches!(w.as_str(), "INSERT" | "UPDATE" | "DELETE")).cloned().unwrap_or_default();
    format!("{timing} {event}")
}

/// The SELECT of `CREATE VIEW name [(cols)] AS select`.
fn view_body(sql: &str) -> String {
    let mut depth = 0;
    for t in tokenize(sql, B) {
        match t.kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => depth -= 1,
            _ if depth == 0 && t.is_word() && t.text(sql).eq_ignore_ascii_case("as") => {
                return sql[t.end..].trim().to_string();
            }
            _ => {}
        }
    }
    sql.to_string()
}

fn details(conn: &rusqlite::Connection, schema: Option<&str>, table: &str) -> DbResult<TableDetails> {
    let (schema, name, ty, sql) = locate(conn, schema, table)?;
    let s = ident(&schema);
    let kind = rel_kind(&ty, &name);

    let mut st = conn
        .prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_xinfo(?1, ?2) WHERE hidden <> 1 ORDER BY cid")
        .map_err(lite_err)?;
    let mut columns: Vec<ColumnInfo> =
        st.query_map(params![name, schema], |r| column_info(r, 0)).and_then(Iterator::collect).map_err(lite_err)?;
    mark_rowid_alias(&mut columns);
    let mut st = conn
        .prepare("SELECT name, pk FROM pragma_table_info(?1, ?2) WHERE pk > 0 ORDER BY pk")
        .map_err(lite_err)?;
    let pk: Vec<String> = st.query_map(params![name, schema], |r| r.get(0)).and_then(Iterator::collect).map_err(lite_err)?;

    let mut st = conn
        .prepare(&format!(
            "SELECT il.name, il.\"unique\", il.origin, (SELECT sql FROM {s}.sqlite_schema WHERE type = 'index' AND name = il.name)
             FROM pragma_index_list(?1, ?2) il ORDER BY il.seq"
        ))
        .map_err(lite_err)?;
    let raw: Vec<(String, bool, String, Option<String>)> = st
        .query_map(params![name, schema], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?, r.get(3)?)))
        .and_then(Iterator::collect)
        .map_err(lite_err)?;
    let mut st = conn
        .prepare("SELECT coalesce(name, '<expr>') FROM pragma_index_xinfo(?1, ?2) WHERE key = 1 ORDER BY seqno")
        .map_err(lite_err)?;
    let mut indexes = Vec::with_capacity(raw.len());
    for (iname, unique, origin, def) in raw {
        let cols: Vec<String> =
            st.query_map(params![iname, schema], |r| r.get(0)).and_then(Iterator::collect).map_err(lite_err)?;
        indexes.push(IndexInfo {
            primary: origin == "pk",
            unique,
            method: None,
            definition: def,
            columns: cols,
            name: iname,
        });
    }

    let foreign_keys = foreign_keys(conn, &schema, Some(&name), None)?;
    let referenced_by = foreign_keys_referencing(conn, &schema, &name)?;
    let quoted = |cols: &[String]| cols.iter().map(|c| quote_ident(c, B)).collect::<Vec<_>>().join(", ");
    let mut constraints = Vec::new();
    if !pk.is_empty() {
        constraints.push(ConstraintInfo {
            name: format!("{name}_pkey"),
            kind: "PRIMARY KEY".into(),
            definition: format!("PRIMARY KEY ({})", quoted(&pk)),
        });
    }
    for ix in indexes.iter().filter(|i| i.definition.is_none() && i.unique && !i.primary) {
        constraints.push(ConstraintInfo {
            name: ix.name.clone(),
            kind: "UNIQUE".into(),
            definition: format!("UNIQUE ({})", quoted(&ix.columns)),
        });
    }
    for fk in &foreign_keys {
        constraints.push(ConstraintInfo {
            name: fk.name.clone(),
            kind: "FOREIGN KEY".into(),
            definition: format!(
                "FOREIGN KEY ({}) REFERENCES {} ({})",
                quoted(&fk.columns),
                quote_ident(&fk.ref_table, B),
                quoted(&fk.ref_columns)
            ),
        });
    }

    let mut st = conn
        .prepare(&format!(
            "SELECT name, sql FROM {s}.sqlite_schema WHERE type = 'trigger' AND tbl_name = ?1 COLLATE NOCASE ORDER BY name"
        ))
        .map_err(lite_err)?;
    let triggers = st
        .query_map(params![name], |r| {
            let sql: String = r.get::<_, Option<String>>(1)?.unwrap_or_default();
            Ok(TriggerInfo { name: r.get(0)?, event: trigger_event(&sql), definition: sql })
        })
        .and_then(Iterator::collect)
        .map_err(lite_err)?;

    let row_estimate = conn
        .query_row(
            &format!("SELECT stat FROM {s}.sqlite_stat1 WHERE tbl = ?1 ORDER BY idx IS NULL DESC LIMIT 1"),
            params![name],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|stat| stat.split_whitespace().next().and_then(|n| n.parse().ok()));
    let size_bytes = conn
        .query_row("SELECT SUM(pgsize) FROM dbstat(?2) WHERE name = ?1", params![name, schema], |r| r.get(0))
        .ok()
        .flatten();

    Ok(TableDetails {
        view_definition: (kind == RelKind::View).then(|| view_body(sql.as_deref().unwrap_or(""))),
        schema,
        name,
        kind,
        columns,
        indexes,
        foreign_keys,
        referenced_by,
        constraints,
        triggers,
        row_estimate,
        size_bytes,
        comment: None,
    })
}

fn foreign_keys_referencing(conn: &rusqlite::Connection, schema: &str, table: &str) -> DbResult<Vec<ForeignKey>> {
    foreign_keys(conn, schema, None, Some(table))
}

fn object_ddl(conn: &rusqlite::Connection, schema: Option<&str>, name: &str) -> DbResult<String> {
    let (schema, name, ty, sql) = locate(conn, schema, name)?;
    let mut out = format!("{};\n", sql.unwrap_or_default().trim_end().trim_end_matches(';'));
    if ty == "table" {
        let mut st = conn
            .prepare(&format!(
                "SELECT sql FROM {}.sqlite_schema
                 WHERE tbl_name = ?1 AND type IN ('index', 'trigger') AND sql IS NOT NULL
                 ORDER BY type, name",
                ident(&schema)
            ))
            .map_err(lite_err)?;
        let extra: Vec<String> = st.query_map(params![name], |r| r.get(0)).and_then(Iterator::collect).map_err(lite_err)?;
        for e in extra {
            out.push_str(&format!("\n{};\n", e.trim_end().trim_end_matches(';')));
        }
    }
    Ok(out)
}

fn explain(conn: &rusqlite::Connection, sql: &str) -> DbResult<PlanNode> {
    let body = sql.trim().trim_end_matches(';');
    let mut st = conn.prepare(&format!("EXPLAIN QUERY PLAN {body}")).map_err(lite_err)?;
    let rows: Vec<(i64, i64, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(3)?)))
        .and_then(Iterator::collect)
        .map_err(lite_err)?;
    fn build(parent: i64, rows: &[(i64, i64, String)]) -> Vec<PlanNode> {
        rows.iter()
            .filter(|(_, p, _)| *p == parent)
            .map(|(id, _, detail)| PlanNode {
                label: detail.clone(),
                children: if *id == parent { Vec::new() } else { build(*id, rows) },
                ..Default::default()
            })
            .collect()
    }
    Ok(PlanNode { label: "QUERY PLAN".into(), children: build(0, &rows), ..Default::default() })
}

impl LiteConn {
    pub async fn connect(spec: &ConnSpec) -> DbResult<Self> {
        let path = spec.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| ":memory:".into());
        let readonly = spec.readonly;
        let init = spec.init_commands.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (jobs, jobs_rx) = std::sync::mpsc::channel::<Job>();
        let in_tx = Arc::new(AtomicBool::new(false));
        let thread_in_tx = in_tx.clone();
        let thread_path = path.clone();
        std::thread::Builder::new()
            .name("quarry-sqlite".into())
            .spawn(move || {
                let opened = open(&thread_path, readonly).and_then(|conn| {
                    for cmd in &init {
                        conn.execute_batch(cmd).map_err(lite_err)?;
                    }
                    Ok(conn)
                });
                let mut lite = match opened {
                    Ok(conn) => {
                        let interrupt = Arc::new(Mutex::new(conn.get_interrupt_handle()));
                        let _ = ready_tx.send(Ok(interrupt.clone()));
                        Lite { conn, interrupt }
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                while let Ok(job) = jobs_rx.recv() {
                    job(&mut lite);
                    thread_in_tx.store(!lite.conn.is_autocommit(), Ordering::Relaxed);
                }
            })
            .map_err(|e| DbError::other(format!("cannot start SQLite thread: {e}")))?;
        let handle = ready_rx.await.map_err(|_| closed())??;
        let info = ServerInfo {
            version: format!("SQLite {}", rusqlite::version()),
            database: Some(path),
            ..Default::default()
        };
        Ok(LiteConn { jobs, info, cancel: LiteCancel { handle }, in_tx, readonly })
    }

    async fn call<R: Send + 'static>(&self, f: impl FnOnce(&mut Lite) -> R + Send + 'static) -> DbResult<R> {
        let (tx, rx) = oneshot::channel();
        self.jobs
            .send(Box::new(move |lite: &mut Lite| {
                let _ = tx.send(f(lite));
            }))
            .map_err(|_| closed())?;
        rx.await.map_err(|_| closed())
    }

    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    pub async fn execute(&mut self, sql: &str, tx: &mpsc::Sender<ExecEvent>) -> DbResult<()> {
        let sql = sql.to_string();
        let tx = tx.clone();
        self.call(move |lite| run(&lite.conn, &sql, &tx)).await?
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle::Sqlite(self.cancel.clone())
    }

    pub fn in_transaction(&self) -> bool {
        self.in_tx.load(Ordering::Relaxed)
    }

    pub async fn change_database(&mut self, database: &str) -> DbResult<()> {
        let path = crate::conn::url::expand_tilde(database).display().to_string();
        let readonly = self.readonly;
        let p = path.clone();
        self.call(move |lite| {
            let conn = open(&p, readonly)?;
            *lite.interrupt.lock().unwrap_or_else(|e| e.into_inner()) = conn.get_interrupt_handle();
            lite.conn = conn;
            Ok::<_, DbError>(())
        })
        .await??;
        self.info.database = Some(path);
        Ok(())
    }

    pub async fn load_catalog(&mut self) -> DbResult<Catalog> {
        self.call(|lite| catalog(&lite.conn)).await?
    }

    pub async fn list_databases(&mut self) -> DbResult<Vec<String>> {
        self.call(|lite| schemas(&lite.conn)).await?
    }

    pub async fn list_relations(&mut self, schema: &str) -> DbResult<Vec<Relation>> {
        let schema = schema.to_string();
        self.call(move |lite| relations(&lite.conn, &schema, false)).await?
    }

    pub async fn table_details(&mut self, schema: Option<&str>, table: &str) -> DbResult<TableDetails> {
        let (schema, table) = (schema.map(str::to_string), table.to_string());
        self.call(move |lite| details(&lite.conn, schema.as_deref(), &table)).await?
    }

    pub async fn object_ddl(&mut self, schema: Option<&str>, name: &str, _kind: &str) -> DbResult<String> {
        let (schema, name) = (schema.map(str::to_string), name.to_string());
        self.call(move |lite| object_ddl(&lite.conn, schema.as_deref(), &name)).await?
    }

    pub async fn activity(&mut self) -> DbResult<ResultSet> {
        Err(DbError::other("session activity is not supported for SQLite"))
    }

    pub async fn kill_session(&mut self, _id: &str) -> DbResult<()> {
        Err(DbError::other("killing sessions is not supported for SQLite"))
    }

    pub async fn load_extension(&mut self, path: &str) -> DbResult<()> {
        let path = crate::conn::url::expand_tilde(path);
        self.call(move |lite| {
            // SAFETY: loading is only enabled for this one call; the user explicitly chose the library.
            unsafe {
                lite.conn.load_extension_enable().map_err(lite_err)?;
                let res = lite.conn.load_extension(&path, None::<&str>).map_err(lite_err);
                lite.conn.load_extension_disable().map_err(lite_err)?;
                res
            }
        })
        .await?
    }

    pub async fn explain(&mut self, sql: &str, _analyze: bool) -> DbResult<PlanNode> {
        let sql = sql.to_string();
        self.call(move |lite| explain(&lite.conn, &sql)).await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_body_skips_column_list() {
        assert_eq!(view_body("CREATE VIEW v(a, b) AS SELECT 1, 2"), "SELECT 1, 2");
        assert_eq!(view_body("create view \"as\" as select x as y from t"), "select x as y from t");
    }

    #[test]
    fn trigger_event_reads_timing_and_action() {
        assert_eq!(trigger_event("CREATE TRIGGER t AFTER UPDATE OF a ON x BEGIN SELECT 1; END"), "AFTER UPDATE");
        assert_eq!(trigger_event("create trigger t instead of delete on v begin select 1; end"), "INSTEAD OF DELETE");
    }

    #[test]
    fn unqualified_names_resolve_temp_first() {
        let all = ["main", "temp", "aux"].map(String::from);
        assert_eq!(search_order(&all), ["temp", "main", "aux"]);
    }
}
