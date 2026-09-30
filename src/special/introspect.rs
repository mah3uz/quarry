use super::{RelFilter, Special, Titled};
use crate::db::{
    Backend, Column, Connection, DbResult, ForeignKey, RelKind, ResultSet, TableDetails, Value, quote_literal,
};

/// Runs an introspection command (`\dt`, `\d t`, `.schema`, `\l`, …) against `conn`.
/// Returns Ok(None) when `cmd` is not an introspection command.
pub async fn run(conn: &mut Connection, cmd: &Special) -> DbResult<Option<Vec<Titled>>> {
    let out = match cmd {
        Special::ListDatabases { pattern, verbose } => vec![list_databases(conn, pattern.as_deref(), *verbose).await?],
        Special::Describe { pattern: None, verbose } => {
            vec![list_relations(conn, RelFilter::All, None, *verbose).await?]
        }
        Special::Describe { pattern: Some(p), verbose } => describe(conn, p, *verbose).await?,
        Special::ListRelations { kind, pattern, verbose } => {
            vec![list_relations(conn, *kind, pattern.as_deref(), *verbose).await?]
        }
        Special::ListIndexes { pattern, verbose } => vec![list_indexes(conn, pattern.as_deref(), *verbose).await?],
        Special::ListSequences { pattern } => vec![list_sequences(conn, pattern.as_deref()).await?],
        Special::ListFunctions { pattern, verbose } => vec![list_functions(conn, pattern.as_deref(), *verbose).await?],
        Special::ListSchemas { pattern } => vec![list_schemas(conn, pattern.as_deref()).await?],
        Special::ListRoles { pattern } => vec![list_roles(conn, pattern.as_deref()).await?],
        Special::ListTypes { pattern } => vec![list_types(conn, pattern.as_deref()).await?],
        Special::ListExtensions { pattern } => vec![list_extensions(conn, pattern.as_deref()).await?],
        Special::ListPrivileges { pattern } => list_privileges(conn, pattern.as_deref()).await?,
        Special::ShowSource { name, kind } => vec![show_source(conn, name, kind).await?],
        Special::Schema { pattern } => vec![schema_ddl(conn, pattern.as_deref()).await?],
        Special::Status => vec![status(conn).await?],
        Special::ConnInfo => vec![conninfo(conn).await?],
        _ => return Ok(None),
    };
    Ok(Some(out))
}

// ---------- patterns ----------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Lit(char),
    Star,
    Question,
}

/// A psql-style `[schema.]name` pattern: `*` and `?` are wildcards, `"…"` (or `` `…` ``) quotes
/// literally, and unquoted text is folded to lower case on Postgres.
#[derive(Clone, Debug, Default, PartialEq)]
struct NamePattern {
    schema: Option<Vec<Tok>>,
    name: Option<Vec<Tok>>,
}

impl NamePattern {
    fn parse(p: &str, backend: Backend) -> NamePattern {
        let fold = backend == Backend::Postgres;
        let mut parts: Vec<Vec<Tok>> = vec![Vec::new()];
        let mut quote: Option<char> = None;
        let mut chars = p.trim().chars().peekable();
        while let Some(c) = chars.next() {
            let cur = parts.last_mut().expect("parts is never empty");
            match quote {
                Some(q) if c == q => {
                    if chars.peek() == Some(&q) {
                        chars.next();
                        cur.push(Tok::Lit(q));
                    } else {
                        quote = None;
                    }
                }
                Some(_) => cur.push(Tok::Lit(c)),
                None => match c {
                    '"' | '`' => quote = Some(c),
                    '.' => parts.push(Vec::new()),
                    '*' => cur.push(Tok::Star),
                    '?' => cur.push(Tok::Question),
                    c if fold => cur.extend(c.to_lowercase().map(Tok::Lit)),
                    c => cur.push(Tok::Lit(c)),
                },
            }
        }
        let nonempty = |v: Vec<Tok>| (!v.is_empty() && v != [Tok::Star]).then_some(v);
        let name = parts.pop().and_then(nonempty);
        let schema = parts.pop().and_then(nonempty);
        NamePattern { schema, name }
    }

    /// Literal `[schema.]name` when the pattern has no wildcards.
    fn exact(p: &str, backend: Backend) -> Option<(Option<String>, String)> {
        let np = NamePattern::parse(p, backend);
        let lit = |t: &Vec<Tok>| {
            t.iter().map(|t| if let Tok::Lit(c) = t { Some(*c) } else { None }).collect::<Option<String>>()
        };
        let name = lit(np.name.as_ref()?)?;
        let schema = match &np.schema {
            Some(s) => Some(lit(s)?),
            None => None,
        };
        Some((schema, name))
    }
}

fn regex(toks: &[Tok]) -> String {
    let mut s = String::from("^(");
    for t in toks {
        match t {
            Tok::Star => s.push_str(".*"),
            Tok::Question => s.push('.'),
            Tok::Lit(c) => {
                if "\\.^$|?*+()[]{}".contains(*c) {
                    s.push('\\');
                }
                s.push(*c);
            }
        }
    }
    s.push_str(")$");
    s
}

fn like(toks: &[Tok]) -> String {
    let mut s = String::new();
    for t in toks {
        match t {
            Tok::Star => s.push('%'),
            Tok::Question => s.push('_'),
            Tok::Lit(c) => {
                if matches!(c, '%' | '_' | '\\') {
                    s.push('\\');
                }
                s.push(*c);
            }
        }
    }
    s
}

/// SQL predicate matching `col` against a pattern part; the pattern is always a quoted literal.
fn matches(backend: Backend, col: &str, toks: &[Tok]) -> String {
    match backend {
        Backend::Postgres => format!("{col} ~ {}", quote_literal(&regex(toks), backend)),
        Backend::MySql => format!("{col} LIKE {}", quote_literal(&like(toks), backend)),
        Backend::Sqlite => format!("{col} LIKE {} ESCAPE '\\'", quote_literal(&like(toks), backend)),
    }
}

/// ` AND …` clauses for a schema/name pattern. Without a schema part, `default_schema` restricts
/// to visible objects (pg search_path, mysql current database).
fn pattern_filter(
    backend: Backend,
    pattern: Option<&str>,
    schema_col: Option<&str>,
    name_cols: &[&str],
    default_schema: &str,
) -> String {
    let np = pattern.map(|p| NamePattern::parse(p, backend)).unwrap_or_default();
    let mut out = String::new();
    match (&np.schema, schema_col) {
        (Some(s), Some(col)) => out.push_str(&format!(" AND {}", matches(backend, col, s))),
        _ if !default_schema.is_empty() => out.push_str(&format!(" AND {default_schema}")),
        _ => {}
    }
    if let Some(n) = &np.name {
        let ors: Vec<String> = name_cols.iter().map(|c| matches(backend, c, n)).collect();
        out.push_str(&format!(" AND ({})", ors.join(" OR ")));
    }
    out
}

fn has_pattern(pattern: Option<&str>) -> bool {
    pattern.is_some_and(|p| !p.trim().is_empty())
}

// ---------- result helpers ----------

fn text_of(v: Option<&Value>) -> String {
    v.map(|v| v.display().into_owned()).unwrap_or_default()
}

fn opt_text(v: Option<&Value>) -> Option<String> {
    v.filter(|v| !v.is_null()).map(|v| v.display().into_owned())
}

/// Catalog NULLs (no description, default ACL) display blank like psql, not as the NULL marker.
fn table(title: &str, mut rs: ResultSet) -> Titled {
    for v in rs.rows.iter_mut().flatten().filter(|v| v.is_null()) {
        *v = Value::Text(String::new());
    }
    Titled { title: Some(title.to_string()), result: rs, footer: None, text: None }
}

fn message(text: impl Into<String>) -> Titled {
    Titled { text: Some(text.into()), ..Default::default() }
}

fn not_found(rs: ResultSet, title: &str, plural: &str, singular: &str, pattern: Option<&str>) -> Titled {
    if !rs.rows.is_empty() {
        return table(title, rs);
    }
    match pattern.filter(|p| !p.trim().is_empty()) {
        Some(p) => message(format!("Did not find any {singular} named \"{p}\".")),
        None => message(format!("Did not find any {plural}.")),
    }
}

fn unsupported(what: &str, backend: Backend) -> Titled {
    message(format!("{what} are not supported by {backend}."))
}

fn kv_text(pairs: &[(&str, Option<String>)]) -> String {
    let width = pairs.iter().filter(|(_, v)| v.is_some()).map(|(k, _)| k.len()).max().unwrap_or(0) + 1;
    let mut out = String::new();
    for (k, v) in pairs {
        if let Some(v) = v {
            out.push_str(&format!("{:<width$} {v}\n", format!("{k}:")));
        }
    }
    out
}

/// pg_size_pretty-style byte formatting.
pub fn pretty_size(bytes: i64) -> String {
    let units = ["bytes", "kB", "MB", "GB", "TB", "PB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size.abs() >= 10.0 * 1024.0 - 0.5 && unit + 1 < units.len() {
        size /= 1024.0;
        unit += 1;
    }
    format!("{} {}", size.round() as i64, units[unit])
}

fn prettify_sizes(rs: &mut ResultSet, column: &str) {
    let Some(idx) = rs.columns.iter().position(|c| c.name == column) else {
        return;
    };
    for row in &mut rs.rows {
        if let Some(v) = row.get_mut(idx) {
            let n = match v {
                Value::Int(i) => Some(*i),
                Value::UInt(u) => i64::try_from(*u).ok(),
                Value::Text(t) => t.trim().parse::<f64>().ok().map(|f| f as i64),
                _ => None,
            };
            if let Some(n) = n {
                *v = Value::Text(pretty_size(n));
            }
        }
    }
    rs.columns[idx] = Column::new(column, "text");
}

async fn mysql_current_db(conn: &mut Connection) -> DbResult<Option<String>> {
    let rs = conn.query("SELECT DATABASE()").await?;
    Ok(opt_text(rs.rows.first().and_then(|r| r.first())))
}

const NO_DATABASE: &str = "No database selected: `use <database>` first, or qualify the pattern (db.table).";

// ---------- \l ----------

async fn list_databases(conn: &mut Connection, pattern: Option<&str>, verbose: bool) -> DbResult<Titled> {
    let b = conn.backend();
    let rs = match b {
        Backend::Postgres => {
            let extra = if verbose {
                r#", CASE WHEN pg_catalog.has_database_privilege(d.datname, 'CONNECT')
                     THEN pg_catalog.pg_size_pretty(pg_catalog.pg_database_size(d.datname)) ELSE 'No Access' END AS "Size",
                   t.spcname AS "Tablespace",
                   pg_catalog.shobj_description(d.oid, 'pg_database') AS "Description""#
            } else {
                ""
            };
            let filter = pattern_filter(b, pattern, None, &["d.datname"], "");
            conn.query(&format!(
                r#"SELECT d.datname AS "Name", pg_catalog.pg_get_userbyid(d.datdba) AS "Owner",
                   pg_catalog.pg_encoding_to_char(d.encoding) AS "Encoding", d.datcollate AS "Collate",
                   d.datctype AS "Ctype", pg_catalog.array_to_string(d.datacl, E'\n') AS "Access privileges"{extra}
                   FROM pg_catalog.pg_database d JOIN pg_catalog.pg_tablespace t ON d.dattablespace = t.oid
                   WHERE true{filter} ORDER BY 1"#
            ))
            .await?
        }
        Backend::MySql => {
            let extra = if verbose {
                ", (SELECT COALESCE(SUM(t.DATA_LENGTH + t.INDEX_LENGTH), 0) FROM information_schema.TABLES t \
                 WHERE t.TABLE_SCHEMA = s.SCHEMA_NAME) AS `Size`, \
                 (SELECT COUNT(*) FROM information_schema.TABLES t WHERE t.TABLE_SCHEMA = s.SCHEMA_NAME) AS `Tables`"
            } else {
                ""
            };
            let filter = pattern_filter(b, pattern, None, &["s.SCHEMA_NAME"], "");
            let mut rs = conn
                .query(&format!(
                    "SELECT s.SCHEMA_NAME AS `Name`, s.DEFAULT_CHARACTER_SET_NAME AS `Encoding`, \
                     s.DEFAULT_COLLATION_NAME AS `Collation`{extra} FROM information_schema.SCHEMATA s \
                     WHERE 1=1{filter} ORDER BY 1"
                ))
                .await?;
            prettify_sizes(&mut rs, "Size");
            rs
        }
        Backend::Sqlite => {
            let filter = pattern_filter(b, pattern, None, &["name"], "");
            conn.query(&format!(
                "SELECT name AS \"Name\", file AS \"File\" FROM pragma_database_list WHERE 1=1{filter} ORDER BY seq"
            ))
            .await?
        }
    };
    Ok(not_found(rs, "List of databases", "databases", "database", pattern))
}

// ---------- \dt \dv \dm \dE \ds \d ----------

const PG_RELKIND: &str = r#"CASE c.relkind WHEN 'r' THEN 'table' WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized view'
    WHEN 'i' THEN 'index' WHEN 'S' THEN 'sequence' WHEN 't' THEN 'TOAST table' WHEN 'f' THEN 'foreign table'
    WHEN 'p' THEN 'partitioned table' WHEN 'I' THEN 'partitioned index' END"#;

const PG_NOT_SYSTEM: &str = "n.nspname <> 'pg_catalog' AND n.nspname !~ '^pg_toast' AND n.nspname <> 'information_schema'";

fn pg_visibility(pattern: Option<&str>, visible_fn: &str) -> String {
    if has_pattern(pattern) { visible_fn.to_string() } else { format!("{PG_NOT_SYSTEM} AND {visible_fn}") }
}

fn rel_words(kind: RelFilter) -> (&'static str, &'static str) {
    match kind {
        RelFilter::Tables => ("tables", "table"),
        RelFilter::Views => ("views", "view"),
        RelFilter::MaterializedViews => ("materialized views", "materialized view"),
        RelFilter::ForeignTables => ("foreign tables", "foreign table"),
        RelFilter::All => ("relations", "relation"),
    }
}

async fn pg_relations(conn: &mut Connection, relkinds: &str, pattern: Option<&str>, verbose: bool) -> DbResult<ResultSet> {
    let extra = if verbose {
        r#", CASE c.relpersistence WHEN 'p' THEN 'permanent' WHEN 't' THEN 'temporary' WHEN 'u' THEN 'unlogged' END AS "Persistence",
           am.amname AS "Access method",
           pg_catalog.pg_size_pretty(pg_catalog.pg_table_size(c.oid)) AS "Size",
           pg_catalog.obj_description(c.oid, 'pg_class') AS "Description""#
    } else {
        ""
    };
    let filter =
        pattern_filter(Backend::Postgres, pattern, Some("n.nspname"), &["c.relname"], &pg_visibility(pattern, "pg_catalog.pg_table_is_visible(c.oid)"));
    conn.query(&format!(
        r#"SELECT n.nspname AS "Schema", c.relname AS "Name", {PG_RELKIND} AS "Type",
           pg_catalog.pg_get_userbyid(c.relowner) AS "Owner"{extra}
           FROM pg_catalog.pg_class c
           LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
           LEFT JOIN pg_catalog.pg_am am ON am.oid = c.relam
           WHERE c.relkind IN ({relkinds}){filter}
           ORDER BY 1, 2"#
    ))
    .await
}

async fn list_relations(conn: &mut Connection, kind: RelFilter, pattern: Option<&str>, verbose: bool) -> DbResult<Titled> {
    let b = conn.backend();
    let (plural, singular) = rel_words(kind);
    let rs = match b {
        Backend::Postgres => {
            let kinds = match kind {
                RelFilter::Tables => "'r','p'",
                RelFilter::Views => "'v'",
                RelFilter::MaterializedViews => "'m'",
                RelFilter::ForeignTables => "'f'",
                RelFilter::All => "'r','p','v','m','S','f'",
            };
            pg_relations(conn, kinds, pattern, verbose).await?
        }
        Backend::MySql => {
            let types = match kind {
                RelFilter::Tables => "'BASE TABLE'",
                RelFilter::Views => "'VIEW', 'SYSTEM VIEW'",
                RelFilter::All => "'BASE TABLE', 'VIEW', 'SYSTEM VIEW', 'SEQUENCE'",
                other => return Ok(unsupported_kind(other, b)),
            };
            let extra = if verbose {
                ", TABLE_ROWS AS `Rows (estimated)`, DATA_LENGTH + INDEX_LENGTH AS `Size`, TABLE_COMMENT AS `Comment`"
            } else {
                ""
            };
            let filter = pattern_filter(b, pattern, Some("TABLE_SCHEMA"), &["TABLE_NAME"], "TABLE_SCHEMA = DATABASE()");
            let mut rs = conn
                .query(&format!(
                    "SELECT TABLE_SCHEMA AS `Schema`, TABLE_NAME AS `Name`, \
                     CASE TABLE_TYPE WHEN 'BASE TABLE' THEN 'table' ELSE LOWER(TABLE_TYPE) END AS `Type`, \
                     ENGINE AS `Engine`{extra} FROM information_schema.TABLES \
                     WHERE TABLE_TYPE IN ({types}){filter} ORDER BY 1, 2"
                ))
                .await?;
            prettify_sizes(&mut rs, "Size");
            if rs.rows.is_empty() && NamePattern::parse(pattern.unwrap_or(""), b).schema.is_none() && mysql_current_db(conn).await?.is_none() {
                return Ok(message(NO_DATABASE));
            }
            rs
        }
        Backend::Sqlite => {
            let types = match kind {
                RelFilter::Tables => "'table', 'virtual'",
                RelFilter::Views => "'view'",
                RelFilter::All => "'table', 'view', 'virtual', 'shadow'",
                other => return Ok(unsupported_kind(other, b)),
            };
            let hide_internal = if has_pattern(pattern) { "" } else { " AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'" };
            let filter = pattern_filter(b, pattern, Some("schema"), &["name"], "");
            let base = format!(
                "SELECT schema AS \"Schema\", name AS \"Name\", type AS \"Type\"{{extra}} FROM pragma_table_list \
                 WHERE type IN ({types}){hide_internal}{filter} ORDER BY 1, 2"
            );
            if verbose {
                let with_size = base.replace(
                    "{extra}",
                    ", ncol AS \"Columns\", CASE wr WHEN 1 THEN 'yes' ELSE '' END AS \"Without rowid\", \
                     CASE strict WHEN 1 THEN 'yes' ELSE '' END AS \"Strict\", \
                     (SELECT SUM(pgsize) FROM dbstat d WHERE d.name = pragma_table_list.name \
                      AND d.schema = pragma_table_list.schema) AS \"Size\"",
                );
                match conn.query(&with_size).await {
                    Ok(mut rs) => {
                        prettify_sizes(&mut rs, "Size");
                        rs
                    }
                    Err(_) => conn.query(&base.replace("{extra}", ", ncol AS \"Columns\"")).await?,
                }
            } else {
                conn.query(&base.replace("{extra}", "")).await?
            }
        }
    };
    Ok(not_found(rs, "List of relations", plural, singular, pattern))
}

fn unsupported_kind(kind: RelFilter, backend: Backend) -> Titled {
    let what = if kind == RelFilter::MaterializedViews { "Materialized views" } else { "Foreign tables" };
    unsupported(what, backend)
}

async fn list_sequences(conn: &mut Connection, pattern: Option<&str>) -> DbResult<Titled> {
    let b = conn.backend();
    let rs = match b {
        Backend::Postgres => pg_relations(conn, "'S'", pattern, false).await?,
        Backend::MySql if conn.info().is_mariadb => {
            let filter = pattern_filter(b, pattern, Some("TABLE_SCHEMA"), &["TABLE_NAME"], "TABLE_SCHEMA = DATABASE()");
            conn.query(&format!(
                "SELECT TABLE_SCHEMA AS `Schema`, TABLE_NAME AS `Name`, 'sequence' AS `Type` \
                 FROM information_schema.TABLES WHERE TABLE_TYPE = 'SEQUENCE'{filter} ORDER BY 1, 2"
            ))
            .await?
        }
        Backend::MySql => return Ok(message("MySQL has no sequences; AUTO_INCREMENT counters are shown by \\dt+ and \\d table.")),
        Backend::Sqlite => {
            let filter = pattern_filter(b, pattern, None, &["name"], "");
            match conn.query(&format!("SELECT name AS \"Table\", seq AS \"Last value\" FROM sqlite_sequence WHERE 1=1{filter} ORDER BY 1")).await {
                Ok(rs) => rs,
                Err(_) => return Ok(message("No AUTOINCREMENT counters (SQLite has no sequences; sqlite_sequence does not exist yet).")),
            }
        }
    };
    Ok(not_found(rs, "List of sequences", "sequences", "sequence", pattern))
}

// ---------- \di ----------

async fn list_indexes(conn: &mut Connection, pattern: Option<&str>, verbose: bool) -> DbResult<Titled> {
    let b = conn.backend();
    let rs = match b {
        Backend::Postgres => {
            let extra = if verbose {
                r#", pg_catalog.pg_size_pretty(pg_catalog.pg_relation_size(c.oid)) AS "Size",
                   pg_catalog.obj_description(c.oid, 'pg_class') AS "Description""#
            } else {
                ""
            };
            let filter = pattern_filter(
                b,
                pattern,
                Some("n.nspname"),
                &["c.relname", "c2.relname"],
                &pg_visibility(pattern, "pg_catalog.pg_table_is_visible(c.oid)"),
            );
            conn.query(&format!(
                r#"SELECT n.nspname AS "Schema", c.relname AS "Name", {PG_RELKIND} AS "Type",
                   pg_catalog.pg_get_userbyid(c.relowner) AS "Owner", c2.relname AS "Table"{extra}
                   FROM pg_catalog.pg_class c
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                   JOIN pg_catalog.pg_index i ON i.indexrelid = c.oid
                   JOIN pg_catalog.pg_class c2 ON i.indrelid = c2.oid
                   WHERE c.relkind IN ('i','I'){filter}
                   ORDER BY 1, 2"#
            ))
            .await?
        }
        Backend::MySql => {
            let filter =
                pattern_filter(b, pattern, Some("TABLE_SCHEMA"), &["INDEX_NAME", "TABLE_NAME"], "TABLE_SCHEMA = DATABASE()");
            let extra = if verbose { ", MAX(INDEX_COMMENT) AS `Comment`" } else { "" };
            conn.query(&format!(
                "SELECT TABLE_SCHEMA AS `Schema`, INDEX_NAME AS `Name`, TABLE_NAME AS `Table`, \
                 GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX SEPARATOR ', ') AS `Columns`, \
                 CASE WHEN NON_UNIQUE = 0 THEN 'yes' ELSE 'no' END AS `Unique`, LOWER(INDEX_TYPE) AS `Method`{extra} \
                 FROM information_schema.STATISTICS WHERE 1=1{filter} \
                 GROUP BY TABLE_SCHEMA, TABLE_NAME, INDEX_NAME, NON_UNIQUE, INDEX_TYPE ORDER BY 1, 3, 2"
            ))
            .await?
        }
        Backend::Sqlite => {
            let filter = pattern_filter(b, pattern, None, &["il.name", "m.name"], "");
            conn.query(&format!(
                "SELECT il.name AS \"Name\", m.name AS \"Table\", \
                 (SELECT group_concat(ii.name, ', ') FROM pragma_index_info(il.name) ii) AS \"Columns\", \
                 CASE il.\"unique\" WHEN 1 THEN 'yes' ELSE 'no' END AS \"Unique\", \
                 CASE il.origin WHEN 'pk' THEN 'primary key' WHEN 'u' THEN 'unique constraint' ELSE 'CREATE INDEX' END AS \"Origin\" \
                 FROM sqlite_schema m JOIN pragma_index_list(m.name) il \
                 WHERE m.type = 'table'{filter} ORDER BY 2, 1"
            ))
            .await?
        }
    };
    Ok(not_found(rs, "List of indexes", "indexes", "index", pattern))
}

// ---------- \df ----------

async fn list_functions(conn: &mut Connection, pattern: Option<&str>, verbose: bool) -> DbResult<Titled> {
    let b = conn.backend();
    let rs = match b {
        Backend::Postgres => {
            let (extra, join) = if verbose {
                (
                    r#", CASE p.provolatile WHEN 'i' THEN 'immutable' WHEN 's' THEN 'stable' WHEN 'v' THEN 'volatile' END AS "Volatility",
                       pg_catalog.pg_get_userbyid(p.proowner) AS "Owner", l.lanname AS "Language",
                       pg_catalog.obj_description(p.oid, 'pg_proc') AS "Description""#,
                    " LEFT JOIN pg_catalog.pg_language l ON l.oid = p.prolang",
                )
            } else {
                ("", "")
            };
            let visible = if has_pattern(pattern) {
                "pg_catalog.pg_function_is_visible(p.oid)".to_string()
            } else {
                "pg_catalog.pg_function_is_visible(p.oid) AND n.nspname <> 'pg_catalog' AND n.nspname <> 'information_schema'"
                    .to_string()
            };
            let filter = pattern_filter(b, pattern, Some("n.nspname"), &["p.proname"], &visible);
            conn.query(&format!(
                r#"SELECT n.nspname AS "Schema", p.proname AS "Name",
                   pg_catalog.pg_get_function_result(p.oid) AS "Result data type",
                   pg_catalog.pg_get_function_arguments(p.oid) AS "Argument data types",
                   CASE p.prokind WHEN 'a' THEN 'agg' WHEN 'w' THEN 'window' WHEN 'p' THEN 'proc' ELSE 'func' END AS "Type"{extra}
                   FROM pg_catalog.pg_proc p
                   LEFT JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace{join}
                   WHERE true{filter}
                   ORDER BY 1, 2, 4"#
            ))
            .await?
        }
        Backend::MySql => {
            let extra = if verbose {
                ", r.SECURITY_TYPE AS `Security`, r.DEFINER AS `Definer`, r.ROUTINE_COMMENT AS `Comment`"
            } else {
                ""
            };
            let filter =
                pattern_filter(b, pattern, Some("r.ROUTINE_SCHEMA"), &["r.ROUTINE_NAME"], "r.ROUTINE_SCHEMA = DATABASE()");
            conn.query(&format!(
                "SELECT r.ROUTINE_SCHEMA AS `Schema`, r.ROUTINE_NAME AS `Name`, \
                 COALESCE(r.DTD_IDENTIFIER, '') AS `Result data type`, \
                 COALESCE((SELECT GROUP_CONCAT(CONCAT_WS(' ', NULLIF(p.PARAMETER_MODE, 'IN'), p.PARAMETER_NAME, p.DTD_IDENTIFIER) \
                    ORDER BY p.ORDINAL_POSITION SEPARATOR ', ') FROM information_schema.PARAMETERS p \
                    WHERE p.SPECIFIC_SCHEMA = r.ROUTINE_SCHEMA AND p.SPECIFIC_NAME = r.SPECIFIC_NAME \
                    AND p.ORDINAL_POSITION > 0), '') AS `Argument data types`, \
                 LOWER(r.ROUTINE_TYPE) AS `Type`{extra} FROM information_schema.ROUTINES r \
                 WHERE 1=1{filter} ORDER BY 1, 2"
            ))
            .await?
        }
        Backend::Sqlite => {
            let filter = pattern_filter(b, pattern, None, &["name"], "");
            let sql = format!(
                "SELECT name AS \"Name\", CASE type WHEN 's' THEN 'scalar' WHEN 'a' THEN 'aggregate' WHEN 'w' THEN 'window' \
                 ELSE type END AS \"Type\", narg AS \"Arguments\", CASE builtin WHEN 1 THEN 'built-in' ELSE 'application' END \
                 AS \"Origin\" FROM pragma_function_list WHERE 1=1{filter} GROUP BY name, type, narg ORDER BY 1, 3"
            );
            match conn.query(&sql).await {
                Ok(rs) => rs,
                Err(_) => return Ok(message("This SQLite build does not expose its function list.")),
            }
        }
    };
    Ok(not_found(rs, "List of functions", "functions", "function", pattern))
}

// ---------- \dn \du \dT \dx \dp ----------

async fn list_schemas(conn: &mut Connection, pattern: Option<&str>) -> DbResult<Titled> {
    let b = conn.backend();
    let rs = match b {
        Backend::Postgres => {
            let default = if has_pattern(pattern) { "" } else { "n.nspname !~ '^pg_' AND n.nspname <> 'information_schema'" };
            let filter = pattern_filter(b, pattern, None, &["n.nspname"], default);
            conn.query(&format!(
                r#"SELECT n.nspname AS "Name", pg_catalog.pg_get_userbyid(n.nspowner) AS "Owner"
                   FROM pg_catalog.pg_namespace n WHERE true{filter} ORDER BY 1"#
            ))
            .await?
        }
        Backend::MySql => {
            let filter = pattern_filter(b, pattern, None, &["SCHEMA_NAME"], "");
            conn.query(&format!(
                "SELECT SCHEMA_NAME AS `Name`, DEFAULT_CHARACTER_SET_NAME AS `Encoding`, \
                 DEFAULT_COLLATION_NAME AS `Collation` FROM information_schema.SCHEMATA WHERE 1=1{filter} ORDER BY 1"
            ))
            .await?
        }
        Backend::Sqlite => {
            let filter = pattern_filter(b, pattern, None, &["name"], "");
            conn.query(&format!(
                "SELECT name AS \"Name\", file AS \"File\" FROM pragma_database_list WHERE 1=1{filter} ORDER BY seq"
            ))
            .await?
        }
    };
    Ok(not_found(rs, "List of schemas", "schemas", "schema", pattern))
}

async fn list_roles(conn: &mut Connection, pattern: Option<&str>) -> DbResult<Titled> {
    let b = conn.backend();
    let rs = match b {
        Backend::Postgres => {
            let default = if has_pattern(pattern) { "" } else { "r.rolname !~ '^pg_'" };
            let filter = pattern_filter(b, pattern, None, &["r.rolname"], default);
            conn.query(&format!(
                r#"SELECT r.rolname AS "Role name",
                   pg_catalog.concat_ws(', ',
                     CASE WHEN r.rolsuper THEN 'Superuser' END,
                     CASE WHEN NOT r.rolinherit THEN 'No inheritance' END,
                     CASE WHEN r.rolcreaterole THEN 'Create role' END,
                     CASE WHEN r.rolcreatedb THEN 'Create DB' END,
                     CASE WHEN NOT r.rolcanlogin THEN 'Cannot login' END,
                     CASE WHEN r.rolreplication THEN 'Replication' END,
                     CASE WHEN r.rolbypassrls THEN 'Bypass RLS' END,
                     CASE WHEN r.rolconnlimit <> -1 THEN r.rolconnlimit || ' connections' END,
                     CASE WHEN r.rolvaliduntil IS NOT NULL THEN 'Password valid until ' || r.rolvaliduntil END
                   ) AS "Attributes"
                   FROM pg_catalog.pg_roles r WHERE true{filter} ORDER BY 1"#
            ))
            .await?
        }
        Backend::MySql => {
            let filter = pattern_filter(b, pattern, None, &["User"], "");
            conn.query(&format!("SELECT User AS `User`, Host AS `Host` FROM mysql.user WHERE 1=1{filter} ORDER BY 1, 2"))
                .await?
        }
        Backend::Sqlite => return Ok(unsupported("Roles", b)),
    };
    Ok(not_found(rs, "List of roles", "roles", "role", pattern))
}

async fn list_types(conn: &mut Connection, pattern: Option<&str>) -> DbResult<Titled> {
    let b = conn.backend();
    if b != Backend::Postgres {
        return Ok(unsupported("User-defined types", b));
    }
    let default = pg_visibility(pattern, "pg_catalog.pg_type_is_visible(t.oid)");
    let filter = pattern_filter(b, pattern, Some("n.nspname"), &["t.typname", "pg_catalog.format_type(t.oid, NULL)"], &default);
    let rs = conn
        .query(&format!(
            r#"SELECT n.nspname AS "Schema", pg_catalog.format_type(t.oid, NULL) AS "Name",
               pg_catalog.obj_description(t.oid, 'pg_type') AS "Description"
               FROM pg_catalog.pg_type t
               LEFT JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
               WHERE (t.typrelid = 0 OR (SELECT c.relkind = 'c' FROM pg_catalog.pg_class c WHERE c.oid = t.typrelid))
                 AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_type el WHERE el.oid = t.typelem AND el.typarray = t.oid){filter}
               ORDER BY 1, 2"#
        ))
        .await?;
    Ok(not_found(rs, "List of data types", "data types", "data type", pattern))
}

async fn list_extensions(conn: &mut Connection, pattern: Option<&str>) -> DbResult<Titled> {
    let b = conn.backend();
    let (rs, title) = match b {
        Backend::Postgres => {
            let filter = pattern_filter(b, pattern, None, &["e.extname"], "");
            let rs = conn
                .query(&format!(
                    r#"SELECT e.extname AS "Name", e.extversion AS "Version", n.nspname AS "Schema",
                       c.description AS "Description"
                       FROM pg_catalog.pg_extension e
                       LEFT JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace
                       LEFT JOIN pg_catalog.pg_description c ON c.objoid = e.oid
                         AND c.classoid = 'pg_catalog.pg_extension'::pg_catalog.regclass
                       WHERE true{filter} ORDER BY 1"#
                ))
                .await?;
            (rs, "List of installed extensions")
        }
        Backend::MySql => {
            let filter = pattern_filter(b, pattern, None, &["PLUGIN_NAME"], "");
            let rs = conn
                .query(&format!(
                    "SELECT PLUGIN_NAME AS `Name`, PLUGIN_VERSION AS `Version`, PLUGIN_STATUS AS `Status`, \
                     PLUGIN_TYPE AS `Type`, PLUGIN_LIBRARY AS `Library` FROM information_schema.PLUGINS \
                     WHERE 1=1{filter} ORDER BY 1"
                ))
                .await?;
            (rs, "List of plugins")
        }
        Backend::Sqlite => {
            return Ok(message("SQLite extensions are loaded per connection with `.load path`; there is no catalog of them."));
        }
    };
    Ok(not_found(rs, title, "extensions", "extension", pattern))
}

async fn list_privileges(conn: &mut Connection, pattern: Option<&str>) -> DbResult<Vec<Titled>> {
    let b = conn.backend();
    match b {
        Backend::Postgres => {
            let default = pg_visibility(pattern, "pg_catalog.pg_table_is_visible(c.oid)").replace(
                "n.nspname !~ '^pg_toast'",
                "n.nspname !~ '^pg_'",
            );
            let filter = pattern_filter(b, pattern, Some("n.nspname"), &["c.relname"], &default);
            let rs = conn
                .query(&format!(
                    r#"SELECT n.nspname AS "Schema", c.relname AS "Name", {PG_RELKIND} AS "Type",
                       pg_catalog.array_to_string(c.relacl, E'\n') AS "Access privileges",
                       pg_catalog.array_to_string(ARRAY(
                         SELECT a.attname || E':\n  ' || pg_catalog.array_to_string(a.attacl, E'\n  ')
                         FROM pg_catalog.pg_attribute a
                         WHERE a.attrelid = c.oid AND NOT a.attisdropped AND a.attacl IS NOT NULL
                       ), E'\n') AS "Column privileges"
                       FROM pg_catalog.pg_class c
                       LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                       WHERE c.relkind IN ('r','v','m','S','f','p'){filter}
                       ORDER BY 1, 2"#
                ))
                .await?;
            Ok(vec![not_found(rs, "Access privileges", "relations", "relation", pattern)])
        }
        Backend::MySql => {
            let filter =
                pattern_filter(b, pattern, Some("TABLE_SCHEMA"), &["TABLE_NAME"], "TABLE_SCHEMA = DATABASE()");
            let rs = conn
                .query(&format!(
                    "SELECT TABLE_SCHEMA AS `Schema`, TABLE_NAME AS `Name`, GRANTEE AS `Grantee`, \
                     GROUP_CONCAT(PRIVILEGE_TYPE ORDER BY PRIVILEGE_TYPE SEPARATOR ', ') AS `Privileges` \
                     FROM information_schema.TABLE_PRIVILEGES WHERE 1=1{filter} \
                     GROUP BY TABLE_SCHEMA, TABLE_NAME, GRANTEE ORDER BY 1, 2, 3"
                ))
                .await?;
            let mut out = vec![not_found(rs, "Table privileges", "table grants", "table grant", pattern)];
            let grants = conn.query("SHOW GRANTS").await?;
            out.push(table("Grants for current user", grants));
            Ok(out)
        }
        Backend::Sqlite => Ok(vec![unsupported("Privileges", b)]),
    }
}

// ---------- \d name ----------

struct Match {
    schema: String,
    name: String,
    /// pg relkind (`r`, `v`, `i`, `S`…) / `table` / `view`.
    kind: String,
    oid: Option<String>,
}

async fn find_relations(conn: &mut Connection, pattern: &str) -> DbResult<Vec<Match>> {
    let b = conn.backend();
    let rs = match b {
        Backend::Postgres => {
            let filter = pattern_filter(b, Some(pattern), Some("n.nspname"), &["c.relname"], "pg_catalog.pg_table_is_visible(c.oid)");
            conn.query(&format!(
                "SELECT n.nspname, c.relname, c.relkind::text, c.oid::text FROM pg_catalog.pg_class c \
                 LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 WHERE c.relkind IN ('r','p','v','m','f','i','I','S'){filter} ORDER BY 1, 2"
            ))
            .await?
        }
        Backend::MySql => {
            let filter = pattern_filter(b, Some(pattern), Some("TABLE_SCHEMA"), &["TABLE_NAME"], "TABLE_SCHEMA = DATABASE()");
            conn.query(&format!(
                "SELECT TABLE_SCHEMA, TABLE_NAME, CASE WHEN TABLE_TYPE LIKE '%VIEW' THEN 'view' ELSE 'table' END, NULL \
                 FROM information_schema.TABLES WHERE 1=1{filter} ORDER BY 1, 2"
            ))
            .await?
        }
        Backend::Sqlite => {
            let filter = pattern_filter(b, Some(pattern), Some("schema"), &["name"], "");
            conn.query(&format!(
                "SELECT schema, name, type, NULL FROM pragma_table_list WHERE type IN ('table', 'view', 'virtual'){filter} \
                 ORDER BY 1, 2"
            ))
            .await?
        }
    };
    Ok(rs
        .rows
        .iter()
        .map(|r| Match {
            schema: text_of(r.first()),
            name: text_of(r.get(1)),
            kind: text_of(r.get(2)),
            oid: opt_text(r.get(3)).filter(|o| o.chars().all(|c| c.is_ascii_digit())),
        })
        .collect())
}

async fn describe(conn: &mut Connection, pattern: &str, verbose: bool) -> DbResult<Vec<Titled>> {
    let b = conn.backend();
    let found = find_relations(conn, pattern).await?;
    if found.is_empty() {
        if b == Backend::MySql && NamePattern::parse(pattern, b).schema.is_none() && mysql_current_db(conn).await?.is_none() {
            return Ok(vec![message(NO_DATABASE)]);
        }
        return Ok(vec![message(format!("Did not find any relation named \"{pattern}\"."))]);
    }
    let mut out = Vec::with_capacity(found.len());
    for m in found {
        match (b, m.kind.as_str(), &m.oid) {
            (Backend::Postgres, "i" | "I", Some(oid)) => out.push(pg_describe_index(conn, &m, oid).await?),
            (Backend::Postgres, "S", Some(oid)) => out.push(pg_describe_sequence(conn, &m, oid).await?),
            _ => {
                let details = conn.table_details(Some(&m.schema), &m.name).await?;
                let storage = match (b, verbose, &m.oid) {
                    (Backend::Postgres, true, Some(oid)) => pg_storage(conn, oid).await?,
                    _ => Vec::new(),
                };
                out.push(format_table_details(b, &details, verbose, &storage));
            }
        }
    }
    Ok(out)
}

async fn pg_storage(conn: &mut Connection, oid: &str) -> DbResult<Vec<(String, String)>> {
    let rs = conn
        .query(&format!(
            "SELECT a.attname, CASE a.attstorage WHEN 'p' THEN 'plain' WHEN 'e' THEN 'external' WHEN 'm' THEN 'main' \
             WHEN 'x' THEN 'extended' END FROM pg_catalog.pg_attribute a \
             WHERE a.attrelid = {oid} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
        ))
        .await?;
    Ok(rs.rows.iter().map(|r| (text_of(r.first()), text_of(r.get(1)))).collect())
}

async fn pg_describe_index(conn: &mut Connection, m: &Match, oid: &str) -> DbResult<Titled> {
    let rs = conn
        .query(&format!(
            r#"SELECT a.attname AS "Column", pg_catalog.format_type(a.atttypid, a.atttypmod) AS "Type",
               CASE WHEN a.attnum <= i.indnkeyatts THEN 'yes' ELSE 'no' END AS "Key?",
               pg_catalog.pg_get_indexdef(a.attrelid, a.attnum, true) AS "Definition"
               FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_index i ON i.indexrelid = a.attrelid
               WHERE a.attrelid = {oid} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"#
        ))
        .await?;
    let info = conn
        .query(&format!(
            "SELECT i.indisprimary::text, i.indisunique::text, am.amname, tn.nspname, t.relname, \
             pg_catalog.pg_get_expr(i.indpred, i.indrelid, true) \
             FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid \
             JOIN pg_catalog.pg_am am ON am.oid = c.relam JOIN pg_catalog.pg_class t ON t.oid = i.indrelid \
             JOIN pg_catalog.pg_namespace tn ON tn.oid = t.relnamespace WHERE i.indexrelid = {oid}"
        ))
        .await?;
    let footer = info.rows.first().map(|r| {
        let truthy = |v: Option<&Value>| matches!(text_of(v).as_str(), "true" | "t");
        let kind = if truthy(r.first()) {
            "primary key, "
        } else if truthy(r.get(1)) {
            "unique, "
        } else {
            ""
        };
        let pred = opt_text(r.get(5)).map(|p| format!(", predicate ({p})")).unwrap_or_default();
        format!("{kind}{}, for table \"{}.{}\"{pred}", text_of(r.get(2)), text_of(r.get(3)), text_of(r.get(4)))
    });
    Ok(Titled {
        title: Some(format!("Index \"{}.{}\"", m.schema, m.name)),
        result: rs,
        footer,
        text: None,
    })
}

async fn pg_describe_sequence(conn: &mut Connection, m: &Match, oid: &str) -> DbResult<Titled> {
    let rs = conn
        .query(&format!(
            r#"SELECT pg_catalog.format_type(s.seqtypid, NULL) AS "Type", s.seqstart AS "Start",
               s.seqmin AS "Minimum", s.seqmax AS "Maximum", s.seqincrement AS "Increment",
               CASE WHEN s.seqcycle THEN 'yes' ELSE 'no' END AS "Cycles?", s.seqcache AS "Cache"
               FROM pg_catalog.pg_sequence s WHERE s.seqrelid = {oid}"#
        ))
        .await?;
    Ok(table(&format!("Sequence \"{}.{}\"", m.schema, m.name), rs))
}

fn kind_title(kind: RelKind) -> &'static str {
    match kind {
        RelKind::Table => "Table",
        RelKind::View => "View",
        RelKind::MaterializedView => "Materialized view",
        RelKind::ForeignTable => "Foreign table",
        RelKind::PartitionedTable => "Partitioned table",
        RelKind::SystemTable => "System table",
    }
}

fn index_line(idx: &crate::db::IndexInfo) -> String {
    let kind = if idx.primary {
        "PRIMARY KEY, "
    } else if idx.unique {
        "UNIQUE, "
    } else {
        ""
    };
    let body = match idx.definition.as_deref().and_then(|d| d.split_once(" USING ")) {
        Some((_, rest)) => rest.to_string(),
        None => {
            let cols = format!("({})", idx.columns.join(", "));
            match &idx.method {
                Some(m) => format!("{} {cols}", m.to_ascii_lowercase()),
                None => cols,
            }
        }
    };
    format!("    \"{}\" {kind}{body}", idx.name)
}

fn fk_actions(fk: &ForeignKey) -> String {
    let mut s = String::new();
    for (label, action) in [("ON UPDATE", &fk.on_update), ("ON DELETE", &fk.on_delete)] {
        if let Some(a) = action.as_deref().filter(|a| !a.eq_ignore_ascii_case("NO ACTION") && !a.is_empty()) {
            s.push_str(&format!(" {label} {a}"));
        }
    }
    s
}

fn qualify(schema: &str, name: &str, home_schema: &str) -> String {
    if schema.is_empty() || schema == home_schema { name.to_string() } else { format!("{schema}.{name}") }
}

fn trigger_line(backend: Backend, t: &crate::db::TriggerInfo) -> String {
    let def = t.definition.trim();
    let stripped = ["CREATE OR REPLACE TRIGGER ", "CREATE TRIGGER "]
        .iter()
        .find_map(|p| def.strip_prefix(p).or_else(|| def.strip_prefix(&p.to_ascii_lowercase()[..])));
    match stripped {
        Some(rest) if backend == Backend::Postgres => format!("    {rest}"),
        _ => format!("    \"{}\" {}", t.name, t.event),
    }
}

/// psql-style `\d table`: column table, then titled sections in the footer.
fn format_table_details(backend: Backend, d: &TableDetails, verbose: bool, storage: &[(String, String)]) -> Titled {
    let mut columns = vec![
        Column::new("Column", "text"),
        Column::new("Type", "text"),
        Column::new("Nullable", "text"),
        Column::new("Default", "text"),
    ];
    if verbose && !storage.is_empty() {
        columns.push(Column::new("Storage", "text"));
    }
    if verbose {
        columns.push(Column::new("Description", "text"));
    }
    let auto_label = match backend {
        Backend::Postgres => "generated as identity",
        Backend::MySql => "auto_increment",
        Backend::Sqlite => "autoincrement",
    };
    let rows = d
        .columns
        .iter()
        .map(|c| {
            let default = match (&c.default, c.auto) {
                (Some(dv), _) => Value::Text(dv.clone()),
                (None, true) => Value::Text(auto_label.into()),
                (None, false) => Value::Text(String::new()),
            };
            let mut row = vec![
                Value::Text(c.name.clone()),
                Value::Text(c.data_type.clone()),
                Value::Text(if c.nullable { String::new() } else { "not null".into() }),
                default,
            ];
            if verbose && !storage.is_empty() {
                let s = storage.iter().find(|(n, _)| *n == c.name).map(|(_, s)| s.clone()).unwrap_or_default();
                row.push(Value::Text(s));
            }
            if verbose {
                row.push(Value::Text(c.comment.clone().unwrap_or_default()));
            }
            row
        })
        .collect();

    let mut footer = String::new();
    let mut section = |title: &str, lines: Vec<String>| {
        if !lines.is_empty() {
            footer.push_str(title);
            footer.push_str(":\n");
            for l in lines {
                footer.push_str(&l);
                footer.push('\n');
            }
        }
    };
    section("Indexes", d.indexes.iter().map(index_line).collect());
    section(
        "Check constraints",
        d.constraints
            .iter()
            .filter(|c| c.kind.eq_ignore_ascii_case("CHECK"))
            .map(|c| {
                let def = c.definition.trim();
                if def.to_ascii_uppercase().starts_with("CHECK") {
                    format!("    \"{}\" {def}", c.name)
                } else {
                    format!("    \"{}\" CHECK ({def})", c.name)
                }
            })
            .collect(),
    );
    section(
        "Foreign-key constraints",
        d.foreign_keys
            .iter()
            .map(|fk| {
                format!(
                    "    \"{}\" FOREIGN KEY ({}) REFERENCES {}({}){}",
                    fk.name,
                    fk.columns.join(", "),
                    qualify(&fk.ref_schema, &fk.ref_table, &d.schema),
                    fk.ref_columns.join(", "),
                    fk_actions(fk)
                )
            })
            .collect(),
    );
    section(
        "Referenced by",
        d.referenced_by
            .iter()
            .map(|fk| {
                format!(
                    "    TABLE \"{}\" CONSTRAINT \"{}\" FOREIGN KEY ({}) REFERENCES {}({}){}",
                    qualify(&fk.schema, &fk.table, &d.schema),
                    fk.name,
                    fk.columns.join(", "),
                    qualify(&fk.ref_schema, &fk.ref_table, &d.schema),
                    fk.ref_columns.join(", "),
                    fk_actions(fk)
                )
            })
            .collect(),
    );
    section("Triggers", d.triggers.iter().map(|t| trigger_line(backend, t)).collect());
    if verbose {
        if let Some(def) = d.view_definition.as_deref().filter(|_| d.kind.is_view()) {
            footer.push_str("View definition:\n");
            footer.push_str(def.trim_end());
            footer.push('\n');
        }
        if let Some(n) = d.row_estimate {
            footer.push_str(&format!("Rows (estimated): {n}\n"));
        }
        if let Some(sz) = d.size_bytes {
            footer.push_str(&format!("Size: {}\n", pretty_size(sz)));
        }
        if let Some(c) = d.comment.as_deref().filter(|c| !c.is_empty()) {
            footer.push_str(&format!("Comment: {c}\n"));
        }
    }
    let name = if d.schema.is_empty() { d.name.clone() } else { format!("{}.{}", d.schema, d.name) };
    Titled {
        title: Some(format!("{} \"{name}\"", kind_title(d.kind))),
        result: ResultSet::new(columns, rows),
        footer: (!footer.is_empty()).then_some(footer),
        text: None,
    }
}

// ---------- \sf \sv .schema ----------

async fn show_source(conn: &mut Connection, name: &str, kind: &str) -> DbResult<Titled> {
    let b = conn.backend();
    let is_view = kind == "view";
    match b {
        Backend::Postgres if is_view => {
            let rs = conn
                .query(&format!(
                    "SELECT pg_catalog.quote_ident(n.nspname), pg_catalog.quote_ident(c.relname), c.relkind::text, \
                     pg_catalog.pg_get_viewdef(c.oid, true) \
                     FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                     WHERE c.oid = {}::pg_catalog.regclass AND c.relkind IN ('v','m')",
                    quote_literal(name, b)
                ))
                .await?;
            let Some(r) = rs.rows.first() else {
                return Ok(message(format!("\"{name}\" is not a view.")));
            };
            let create = if text_of(r.get(2)) == "m" { "CREATE MATERIALIZED VIEW" } else { "CREATE OR REPLACE VIEW" };
            Ok(message(format!(
                "{create} {}.{} AS\n{}",
                text_of(r.first()),
                text_of(r.get(1)),
                text_of(r.get(3)).trim_end()
            )))
        }
        Backend::Postgres => {
            let cast = if name.contains('(') { "regprocedure" } else { "regproc" };
            let rs = conn
                .query(&format!(
                    "SELECT pg_catalog.pg_get_functiondef({}::pg_catalog.{cast}::pg_catalog.oid)",
                    quote_literal(name, b)
                ))
                .await?;
            Ok(message(text_of(rs.rows.first().and_then(|r| r.first())).trim_end().to_string()))
        }
        Backend::Sqlite if !is_view => Ok(unsupported("Stored functions", b)),
        _ => {
            let (schema, obj) = NamePattern::exact(name, b).unwrap_or((None, name.to_string()));
            let ddl = match conn.object_ddl(schema.as_deref(), &obj, kind).await {
                Err(_) if b == Backend::MySql && !is_view => conn.object_ddl(schema.as_deref(), &obj, "procedure").await?,
                other => other?,
            };
            Ok(message(ddl))
        }
    }
}

async fn schema_ddl(conn: &mut Connection, pattern: Option<&str>) -> DbResult<Titled> {
    let b = conn.backend();
    if b == Backend::Sqlite {
        let np = pattern.map(|p| NamePattern::parse(p, b)).unwrap_or_default();
        let schema_table = match &np.schema {
            Some(s) => {
                let name: String = s.iter().filter_map(|t| if let Tok::Lit(c) = t { Some(*c) } else { None }).collect();
                format!("{}.sqlite_schema", crate::db::quote_ident(&name, b))
            }
            None => "sqlite_schema".to_string(),
        };
        let filter = match &np.name {
            Some(n) => format!(" AND ({} OR {})", matches(b, "name", n), matches(b, "tbl_name", n)),
            None => String::new(),
        };
        let rs = conn
            .query(&format!(
                "SELECT sql FROM {schema_table} WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'{filter} \
                 ORDER BY tbl_name, type DESC, name"
            ))
            .await?;
        let stmts: Vec<String> = rs.rows.iter().map(|r| format!("{};", text_of(r.first()).trim_end())).collect();
        return Ok(if stmts.is_empty() { not_found_text(pattern) } else { message(stmts.join("\n")) });
    }
    let found = match pattern {
        Some(p) => find_relations(conn, p).await?,
        None => find_relations(conn, "*").await?,
    };
    let mut parts = Vec::new();
    for m in found {
        let kind = match (b, m.kind.as_str()) {
            (Backend::Postgres, "v") | (_, "view") => "view",
            (Backend::Postgres, "m") => "materialized view",
            (Backend::Postgres, "i" | "I") => "index",
            (Backend::Postgres, "S") => "sequence",
            _ => "table",
        };
        let ddl = conn.object_ddl(Some(&m.schema), &m.name, kind).await?;
        let ddl = ddl.trim_end();
        parts.push(if ddl.ends_with(';') { ddl.to_string() } else { format!("{ddl};") });
    }
    Ok(if parts.is_empty() { not_found_text(pattern) } else { message(parts.join("\n\n")) })
}

fn not_found_text(pattern: Option<&str>) -> Titled {
    match pattern {
        Some(p) => message(format!("Did not find any relation named \"{p}\".")),
        None => message("Did not find any relations."),
    }
}

// ---------- \s \conninfo ----------

fn yes_no(b: bool) -> Option<String> {
    Some(if b { "yes" } else { "no" }.into())
}

async fn status(conn: &mut Connection) -> DbResult<Titled> {
    let b = conn.backend();
    let info = conn.info().clone();
    let tx = Some(if conn.in_transaction() { "in transaction" } else { "idle" }.to_string());
    let text = match b {
        Backend::Postgres => {
            let rs = conn
                .query(
                    "SELECT pg_catalog.version(), pg_catalog.current_database(), current_user, session_user, \
                     pg_catalog.pg_backend_pid()::text, pg_catalog.host(pg_catalog.inet_server_addr()), \
                     pg_catalog.inet_server_port()::text, \
                     pg_catalog.date_trunc('second', pg_catalog.now() - pg_catalog.pg_postmaster_start_time())::text, \
                     pg_catalog.current_setting('server_encoding'), pg_catalog.current_setting('client_encoding'), \
                     pg_catalog.current_setting('TimeZone'), pg_catalog.current_setting('search_path'), \
                     pg_catalog.current_setting('transaction_read_only')",
                )
                .await?;
            let r = rs.rows.first().cloned().unwrap_or_default();
            let g = |i: usize| opt_text(r.get(i));
            let server = match (g(5), g(6)) {
                (Some(a), Some(p)) => Some(format!("{a} port {p}")),
                _ => Some(format!("{} (unix socket)", info.host.clone().unwrap_or_else(|| "local".into()))),
            };
            kv_text(&[
                ("Server version", g(0)),
                ("Server", server),
                ("Database", g(1)),
                ("User", g(2).map(|u| if Some(&u) == g(3).as_ref() { u } else { format!("{u} (session: {})", g(3).unwrap_or_default()) })),
                ("Backend PID", g(4)),
                ("Uptime", g(7)),
                ("Server encoding", g(8)),
                ("Client encoding", g(9)),
                ("Time zone", g(10)),
                ("Search path", g(11)),
                ("Transaction", tx),
                ("Read-only", g(12)),
                ("SSL", yes_no(info.tls)),
            ])
        }
        Backend::MySql => {
            let rs = conn
                .query(
                    "SELECT VERSION(), @@version_comment, DATABASE(), CURRENT_USER(), USER(), CONNECTION_ID(), \
                     @@hostname, @@port, @@character_set_server, @@character_set_database, @@character_set_client, \
                     @@character_set_connection, @@collation_connection, @@time_zone, @@autocommit",
                )
                .await?;
            let r = rs.rows.first().cloned().unwrap_or_default();
            let g = |i: usize| opt_text(r.get(i));
            let uptime = conn.query("SHOW GLOBAL STATUS LIKE 'Uptime'").await.ok().and_then(|rs| {
                rs.rows.first().and_then(|r| opt_text(r.get(1))).and_then(|s| s.parse::<u64>().ok()).map(format_uptime)
            });
            let cipher = conn
                .query("SHOW SESSION STATUS LIKE 'Ssl_cipher'")
                .await
                .ok()
                .and_then(|rs| rs.rows.first().and_then(|r| opt_text(r.get(1))))
                .filter(|c| !c.is_empty());
            let flavor = if info.is_mariadb { "MariaDB" } else { "MySQL" };
            kv_text(&[
                ("Server version", g(0).map(|v| format!("{flavor} {v} ({})", g(1).unwrap_or_default()))),
                ("Server", g(6).map(|h| format!("{h} port {}", g(7).unwrap_or_default()))),
                ("Connection id", g(5)),
                ("Current database", Some(g(2).unwrap_or_else(|| "(none)".into()))),
                ("Current user", g(4)),
                ("Authenticated as", g(3)),
                ("Uptime", uptime),
                ("Server charset", g(8)),
                ("Db charset", g(9)),
                ("Client charset", g(10)),
                ("Conn. charset", g(11).map(|c| format!("{c} ({})", g(12).unwrap_or_default()))),
                ("Time zone", g(13)),
                ("Autocommit", g(14).map(|a| if a == "1" { "on".into() } else { "off".into() })),
                ("Transaction", tx),
                ("SSL", Some(cipher.map(|c| format!("cipher in use is {c}")).unwrap_or_else(|| "not in use".into()))),
            ])
        }
        Backend::Sqlite => {
            let one = async |conn: &mut Connection, sql: &str| -> Option<String> {
                conn.query(sql).await.ok().and_then(|rs| rs.rows.first().and_then(|r| opt_text(r.first())))
            };
            let version = one(conn, "SELECT sqlite_version()").await;
            let file = one(conn, "SELECT file FROM pragma_database_list WHERE name = 'main'").await;
            let encoding = one(conn, "PRAGMA encoding").await;
            let journal = one(conn, "PRAGMA journal_mode").await;
            let fks = one(conn, "PRAGMA foreign_keys").await;
            let size = one(conn, "SELECT page_count * page_size FROM pragma_page_count, pragma_page_size").await;
            let attached = conn
                .query("SELECT name FROM pragma_database_list ORDER BY seq")
                .await
                .ok()
                .map(|rs| rs.rows.iter().map(|r| text_of(r.first())).collect::<Vec<_>>().join(", "));
            kv_text(&[
                ("SQLite version", version),
                ("Database file", file.map(|f| if f.is_empty() { ":memory:".into() } else { f })),
                ("Size", size.and_then(|s| s.parse::<i64>().ok()).map(pretty_size)),
                ("Encoding", encoding),
                ("Journal mode", journal),
                ("Foreign keys", fks.map(|f| if f == "1" { "on".into() } else { "off".into() })),
                ("Schemas", attached),
                ("Transaction", tx),
            ])
        }
    };
    Ok(message(text))
}

fn format_uptime(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 { format!("{d} days {h:02}:{m:02}:{s:02}") } else { format!("{h:02}:{m:02}:{s:02}") }
}

async fn conninfo(conn: &mut Connection) -> DbResult<Titled> {
    let b = conn.backend();
    let info = conn.info().clone();
    if b == Backend::Sqlite {
        let rs = conn.query("SELECT file FROM pragma_database_list WHERE name = 'main'").await?;
        let file = opt_text(rs.rows.first().and_then(|r| r.first())).filter(|f| !f.is_empty());
        return Ok(message(format!(
            "You are connected to SQLite database \"{}\" ({}).",
            file.unwrap_or_else(|| ":memory:".into()),
            info.version
        )));
    }
    let db = info.database.clone().unwrap_or_else(|| "(none)".into());
    let user = info.user.clone().unwrap_or_default();
    let via = match (&info.host, info.port) {
        (Some(h), _) if h.starts_with('/') => format!("via socket \"{h}\""),
        (Some(h), Some(p)) => format!("on host \"{h}\" at port \"{p}\""),
        (Some(h), None) => format!("on host \"{h}\""),
        (None, _) => "via the default socket".into(),
    };
    let mut text = format!("You are connected to database \"{db}\" as user \"{user}\" {via}.");
    if info.tls {
        text.push_str("\nSSL connection (TLS).");
    }
    if let Some(id) = &info.session_id {
        text.push_str(&format!("\nSession id: {id}. Server: {}.", info.version));
    }
    Ok(message(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psql_patterns_translate_to_anchored_regex_with_folding_and_quotes() {
        let p = NamePattern::parse("Public.User*", Backend::Postgres);
        assert_eq!(p.schema.as_deref().map(regex).as_deref(), Some("^(public)$"));
        assert_eq!(p.name.as_deref().map(regex).as_deref(), Some("^(user.*)$"));
        let q = NamePattern::parse("\"My.Table\"", Backend::Postgres);
        assert_eq!((q.schema, q.name.as_deref().map(regex)), (None, Some("^(My\\.Table)$".into())));
        let r = NamePattern::parse("a+b?", Backend::Postgres);
        assert_eq!(r.name.as_deref().map(regex).as_deref(), Some("^(a\\+b.)$"));
        assert_eq!(NamePattern::parse("s.*", Backend::Postgres).name, None, "`schema.*` means every name");
    }

    #[test]
    fn like_patterns_escape_wildcards_and_keep_case_off_postgres() {
        let p = NamePattern::parse("shop.Order_*", Backend::MySql);
        assert_eq!(p.name.as_deref().map(like).as_deref(), Some("Order\\_%"));
        assert_eq!(NamePattern::parse("`we.ird`", Backend::MySql).name.as_deref().map(like).as_deref(), Some("we.ird"));
        assert_eq!(NamePattern::parse("100%", Backend::Sqlite).name.as_deref().map(like).as_deref(), Some("100\\%"));
    }

    #[test]
    fn user_patterns_are_always_quoted_literals() {
        let evil = "x'); DROP TABLE t; --";
        let f = pattern_filter(Backend::Postgres, Some(evil), Some("n.nspname"), &["c.relname"], "vis");
        assert_eq!(f, " AND vis AND (c.relname ~ '^(x''\\); drop table t; --)$')");
        let f = pattern_filter(Backend::MySql, Some("a\\b'"), Some("s"), &["n"], "s = DATABASE()");
        assert_eq!(f, " AND s = DATABASE() AND (n LIKE 'a\\\\\\\\b''')");
        let f = pattern_filter(Backend::Sqlite, Some("m.t?"), Some("schema"), &["name"], "");
        assert_eq!(f, " AND schema LIKE 'm' ESCAPE '\\' AND (name LIKE 't_' ESCAPE '\\')");
    }

    #[test]
    fn exact_names_split_schema() {
        assert_eq!(NamePattern::exact("app.\"Users\"", Backend::Postgres), Some((Some("app".into()), "Users".into())));
        assert_eq!(NamePattern::exact("f", Backend::MySql), Some((None, "f".into())));
        assert_eq!(NamePattern::exact("f*", Backend::MySql), None);
    }

    #[test]
    fn sizes_like_pg_size_pretty() {
        assert_eq!(pretty_size(8192), "8192 bytes");
        assert_eq!(pretty_size(16384), "16 kB");
        assert_eq!(pretty_size(10 * 1024 * 1024), "10 MB");
        assert_eq!(format_uptime(90061), "1 days 01:01:01");
    }

    fn details() -> TableDetails {
        use crate::db::{ColumnInfo, ConstraintInfo, IndexInfo, TriggerInfo};
        let col = |name: &str, ty: &str, nullable: bool, default: Option<&str>, pk: bool| ColumnInfo {
            name: name.into(),
            data_type: ty.into(),
            nullable,
            default: default.map(String::from),
            primary_key: pk,
            auto: false,
            comment: None,
        };
        TableDetails {
            schema: "public".into(),
            name: "orders".into(),
            kind: RelKind::Table,
            columns: vec![
                col("id", "integer", false, Some("nextval('orders_id_seq'::regclass)"), true),
                col("user_id", "integer", true, None, false),
                col("total", "numeric(10,2)", false, Some("0"), false),
            ],
            indexes: vec![
                IndexInfo {
                    name: "orders_pkey".into(),
                    columns: vec!["id".into()],
                    unique: true,
                    primary: true,
                    method: Some("btree".into()),
                    definition: Some("CREATE UNIQUE INDEX orders_pkey ON public.orders USING btree (id)".into()),
                },
                IndexInfo {
                    name: "orders_user_idx".into(),
                    columns: vec!["user_id".into()],
                    unique: false,
                    primary: false,
                    method: Some("BTREE".into()),
                    definition: None,
                },
            ],
            foreign_keys: vec![ForeignKey {
                name: "orders_user_id_fkey".into(),
                schema: "public".into(),
                table: "orders".into(),
                columns: vec!["user_id".into()],
                ref_schema: "public".into(),
                ref_table: "users".into(),
                ref_columns: vec!["id".into()],
                on_update: Some("NO ACTION".into()),
                on_delete: Some("CASCADE".into()),
            }],
            referenced_by: vec![ForeignKey {
                name: "items_order_fkey".into(),
                schema: "sales".into(),
                table: "items".into(),
                columns: vec!["order_id".into()],
                ref_schema: "public".into(),
                ref_table: "orders".into(),
                ref_columns: vec!["id".into()],
                on_update: None,
                on_delete: None,
            }],
            constraints: vec![ConstraintInfo { name: "total_pos".into(), kind: "CHECK".into(), definition: "total >= 0".into() }],
            triggers: vec![TriggerInfo {
                name: "audit".into(),
                event: "AFTER INSERT".into(),
                definition: "CREATE TRIGGER audit AFTER INSERT ON public.orders FOR EACH ROW EXECUTE FUNCTION log()".into(),
            }],
            row_estimate: Some(42),
            size_bytes: Some(16384),
            comment: Some("customer orders".into()),
            view_definition: None,
        }
    }

    #[test]
    fn describe_table_has_psql_sections() {
        let t = format_table_details(Backend::Postgres, &details(), false, &[]);
        assert_eq!(t.title.as_deref(), Some("Table \"public.orders\""));
        let names: Vec<&str> = t.result.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Column", "Type", "Nullable", "Default"]);
        assert_eq!(t.result.rows[0][2], Value::Text("not null".into()));
        assert_eq!(
            t.footer.as_deref().unwrap(),
            "Indexes:\n    \"orders_pkey\" PRIMARY KEY, btree (id)\n    \"orders_user_idx\" btree (user_id)\n\
             Check constraints:\n    \"total_pos\" CHECK (total >= 0)\n\
             Foreign-key constraints:\n    \"orders_user_id_fkey\" FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE\n\
             Referenced by:\n    TABLE \"sales.items\" CONSTRAINT \"items_order_fkey\" FOREIGN KEY (order_id) REFERENCES orders(id)\n\
             Triggers:\n    audit AFTER INSERT ON public.orders FOR EACH ROW EXECUTE FUNCTION log()\n"
        );
    }

    #[test]
    fn describe_verbose_adds_storage_description_size_and_comment() {
        let storage = vec![("id".to_string(), "plain".to_string())];
        let t = format_table_details(Backend::Postgres, &details(), true, &storage);
        let names: Vec<&str> = t.result.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Column", "Type", "Nullable", "Default", "Storage", "Description"]);
        assert_eq!(t.result.rows[0][4], Value::Text("plain".into()));
        let footer = t.footer.unwrap();
        assert!(footer.ends_with("Rows (estimated): 42\nSize: 16 kB\nComment: customer orders\n"), "{footer}");
        let my = format_table_details(Backend::MySql, &details(), true, &[]);
        assert!(!my.result.columns.iter().any(|c| c.name == "Storage"));
        assert!(my.footer.unwrap().contains("    \"audit\" AFTER INSERT\n"), "non-pg triggers show name and event");
    }
}
