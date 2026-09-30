//! Driver tests against real servers, each in its own `quarry_test_*` database or temp SQLite file (unreachable servers are skipped).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use quarry::conn::{ConnSpec, SslMode};
use quarry::db::*;
use tokio::sync::mpsc;

async fn events(conn: &mut Connection, sql: &str) -> (DbResult<()>, Vec<ExecEvent>) {
    let (tx, mut rx) = mpsc::channel(1024);
    let run = async move {
        let tx = tx;
        conn.execute(sql, &tx).await
    };
    let collect = async {
        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            out.push(ev);
        }
        out
    };
    tokio::join!(run, collect)
}

/// Compile-time check: the REPL/TUI run driver calls on spawned tasks, so every future must be Send.
#[allow(dead_code)]
fn driver_futures_are_send(c: &mut Connection, tx: &mpsc::Sender<ExecEvent>) {
    fn send<T: Send>(_: T) {}
    fn shared<T: Send + Sync>() {}
    shared::<Connection>();
    shared::<CancelHandle>();
    send(c.execute("", tx));
    send(c.load_catalog());
    send(c.table_details(None, ""));
    send(c.object_ddl(None, "", ""));
    send(c.explain("", true));
    send(c.change_database(""));
    send(c.activity());
    send(c.cancel_handle().cancel());
}

async fn q(conn: &mut Connection, sql: &str) -> ResultSet {
    conn.query(sql).await.unwrap_or_else(|e| panic!("{sql}\n  failed: {e:?}"))
}

async fn scalar(conn: &mut Connection, sql: &str) -> Value {
    q(conn, sql).await.rows.remove(0).remove(0)
}

fn kinds(rs: &ResultSet) -> Vec<TypeKind> {
    rs.columns.iter().map(|c| c.kind).collect()
}

/// Checks the event protocol: Columns first, only row batches of bounded size in between, one Done last.
fn assert_stream_shape(evs: &[ExecEvent]) -> (usize, usize) {
    let data: Vec<&ExecEvent> = evs.iter().filter(|e| !matches!(e, ExecEvent::Notice(_))).collect();
    assert!(matches!(data.first(), Some(ExecEvent::Columns(_))), "stream must start with Columns: {data:?}");
    assert!(matches!(data.last(), Some(ExecEvent::Done(_))), "stream must end with Done");
    assert_eq!(data.iter().filter(|e| matches!(e, ExecEvent::Done(_))).count(), 1);
    let batches: Vec<usize> = data
        .iter()
        .filter_map(|e| match e {
            ExecEvent::Rows(r) => Some(r.len()),
            _ => None,
        })
        .collect();
    assert!(batches.iter().all(|&n| n > 0 && n <= 256), "batches must be 1..=256 rows: {batches:?}");
    (batches.len(), batches.iter().sum())
}

fn done(evs: &[ExecEvent]) -> &Summary {
    evs.iter()
        .find_map(|e| match e {
            ExecEvent::Done(s) => Some(s),
            _ => None,
        })
        .expect("Done event")
}

/// Drops the receiver after the first row batch; returns how long `execute` took to give up.
async fn abandon_after_first_batch(conn: &mut Connection, sql: &str) -> (DbResult<()>, Duration) {
    let (tx, mut rx) = mpsc::channel(4);
    let started = Instant::now();
    let run = async move {
        let tx = tx;
        conn.execute(sql, &tx).await
    };
    let consume = async move {
        while let Some(ev) = rx.recv().await {
            if matches!(ev, ExecEvent::Rows(_)) {
                break;
            }
        }
        drop(rx);
    };
    let (res, ()) = tokio::join!(run, consume);
    (res, started.elapsed())
}

async fn cancel_after(conn: &mut Connection, sql: &str, delay: Duration) -> (DbResult<ResultSet>, Duration) {
    let cancel = conn.cancel_handle();
    let started = Instant::now();
    let (res, cancelled) = tokio::join!(conn.query(sql), async {
        tokio::time::sleep(delay).await;
        cancel.cancel().await
    });
    cancelled.expect("cancel request");
    (res, started.elapsed())
}

fn server_spec(env: &str, default: &str) -> ConnSpec {
    let url = std::env::var(env).unwrap_or_else(|_| default.to_string());
    let mut spec = ConnSpec::parse(&url).expect("valid test URL");
    spec.connect_timeout = Some(Duration::from_secs(3));
    spec
}

async fn connect_or_skip(spec: &ConnSpec, what: &str) -> Option<Connection> {
    match Connection::connect(spec).await {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("SKIP: {what} not reachable at {} ({e})", spec.display_url());
            None
        }
    }
}

/// A throwaway server database named `quarry_test_<tag>`.
struct TestDb {
    admin: ConnSpec,
    spec: ConnSpec,
    name: String,
}

impl TestDb {
    async fn create(admin: ConnSpec, tag: &str) -> Option<(TestDb, Connection)> {
        let mut conn = connect_or_skip(&admin, admin.backend.name()).await?;
        let name = format!("quarry_test_{tag}");
        let (drop, create) = match admin.backend {
            Backend::Postgres => {
                (format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"), format!("CREATE DATABASE {name}"))
            }
            _ => (format!("DROP DATABASE IF EXISTS {name}"), format!("CREATE DATABASE {name}")),
        };
        q(&mut conn, &drop).await;
        q(&mut conn, &create).await;
        let spec = ConnSpec { database: Some(name.clone()), ..admin.clone() };
        let db = Connection::connect(&spec).await.expect("connect to test database");
        Some((TestDb { admin, spec, name }, db))
    }

    async fn connect(&self) -> Connection {
        Connection::connect(&self.spec).await.expect("connect to test database")
    }

    async fn drop(self, conn: Connection) {
        drop(conn);
        let mut admin = Connection::connect(&self.admin).await.expect("admin connection");
        let force = if self.admin.backend == Backend::Postgres { " WITH (FORCE)" } else { "" };
        q(&mut admin, &format!("DROP DATABASE IF EXISTS {}{force}", self.name)).await;
    }
}

fn pg_admin() -> ConnSpec {
    server_spec("QUARRY_TEST_PG", "postgres://postgres@127.0.0.1:5432/postgres")
}

fn my_admin() -> ConnSpec {
    server_spec("QUARRY_TEST_MYSQL", "mysql://root@127.0.0.1:3306")
}

#[tokio::test]
async fn pg_values_keep_their_types_for_alignment_and_exactness() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_values").await else { return };
    let rs = q(
        &mut c,
        "SELECT 1::int4 AS i, 9007199254740993::int8 AS big, 1.5::float8 AS f, 12.50::numeric(6,2) AS n,
                true AS b, '\\x00ff'::bytea AS raw, 'x'::text AS t, NULL::int AS nul, DATE '2024-01-02' AS d,
                '{1,2}'::int[] AS arr",
    )
    .await;
    assert_eq!(
        rs.rows[0],
        vec![
            Value::Int(1),
            // beyond f64 precision: must stay an exact integer
            Value::Int(9007199254740993),
            Value::Float(1.5),
            // numerics stay text so no digits are lost or invented
            Value::Text("12.50".into()),
            Value::Bool(true),
            Value::Bytes(vec![0, 255]),
            Value::Text("x".into()),
            Value::Null,
            Value::Text("2024-01-02".into()),
            Value::Text("{1,2}".into()),
        ]
    );
    use TypeKind::*;
    assert_eq!(kinds(&rs), [Integer, Integer, Float, Decimal, Bool, Binary, Text, Integer, Date, Array]);
    assert_eq!(rs.summary.status.as_deref(), Some("SELECT 1"));
    db.drop(c).await;
}

#[tokio::test]
async fn pg_streams_large_results_in_bounded_batches() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_stream").await else { return };
    let (res, evs) = events(&mut c, "SELECT g FROM generate_series(1, 1000) g").await;
    res.unwrap();
    let (batches, rows) = assert_stream_shape(&evs);
    assert_eq!(rows, 1000);
    assert!(batches >= 4, "1000 rows must arrive in several batches, got {batches}");
    assert_eq!(done(&evs).status.as_deref(), Some("SELECT 1000"));
    db.drop(c).await;
}

#[tokio::test]
async fn pg_reports_command_tags_and_forwards_notices() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_tags").await else { return };
    let rs = q(&mut c, "CREATE TABLE t (id serial PRIMARY KEY, v text)").await;
    assert_eq!(rs.summary.status.as_deref(), Some("CREATE TABLE"));
    let rs = q(&mut c, "INSERT INTO t (v) VALUES ('a'), ('b'), ('c')").await;
    assert_eq!(rs.summary.status.as_deref(), Some("INSERT 0 3"));
    assert_eq!(rs.summary.rows_affected, Some(3));
    let rs = q(&mut c, "UPDATE t SET v = 'z' WHERE id > 1 RETURNING id").await;
    assert_eq!((rs.rows.len(), rs.summary.status.as_deref()), (2, Some("UPDATE 2")));

    let (res, evs) = events(&mut c, "DO $$ BEGIN RAISE NOTICE 'hello %', 42; RAISE WARNING 'careful'; END $$").await;
    res.unwrap();
    let notices: Vec<&Notice> = evs
        .iter()
        .filter_map(|e| match e {
            ExecEvent::Notice(n) => Some(n),
            _ => None,
        })
        .collect();
    assert_eq!(notices.len(), 2, "{evs:?}");
    assert_eq!((notices[0].severity.as_str(), notices[0].message.as_str()), ("NOTICE", "hello 42"));
    assert_eq!(notices[1].severity, "WARNING");
    assert!(matches!(evs.last(), Some(ExecEvent::Done(_))), "notices must precede Done");
    db.drop(c).await;
}

#[tokio::test]
async fn pg_errors_carry_sqlstate_and_position() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_errors").await else { return };
    let e = c.query("SELECT * FROM missing_table").await.unwrap_err();
    assert_eq!((e.kind, e.code.as_deref(), e.position), (ErrorKind::Query, Some("42P01"), Some(15)));
    let e = c.query("SELECT 1/0").await.unwrap_err();
    assert_eq!(e.code.as_deref(), Some("22012"));
    let e = c.query("SELEC 1").await.unwrap_err();
    assert_eq!((e.code.as_deref(), e.position), (Some("42601"), Some(1)));
    assert_eq!(scalar(&mut c, "SELECT 2").await, Value::Int(2), "connection stays usable after errors");
    db.drop(c).await;
}

#[tokio::test]
async fn pg_cancel_aborts_running_statement() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_cancel").await else { return };
    let (res, took) = cancel_after(&mut c, "SELECT pg_sleep(10)", Duration::from_millis(300)).await;
    let e = res.unwrap_err();
    assert_eq!((e.kind, e.code.as_deref()), (ErrorKind::Cancelled, Some("57014")));
    assert!(took < Duration::from_secs(2), "cancel took {took:?}");
    assert_eq!(scalar(&mut c, "SELECT 1").await, Value::Int(1));
    db.drop(c).await;
}

#[tokio::test]
async fn pg_dropping_the_receiver_stops_server_work() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_abandon").await else { return };
    // ~10s of server work if it ran to completion
    let sql = "SELECT g, repeat('x', 200), pg_sleep(0.002) FROM generate_series(1, 5000) g";
    let (res, took) = abandon_after_first_batch(&mut c, sql).await;
    assert_eq!(res.unwrap_err().kind, ErrorKind::Cancelled);
    assert!(took < Duration::from_secs(4), "abandoning took {took:?}");
    assert_eq!(scalar(&mut c, "SELECT 3").await, Value::Int(3), "next statement must not be hit by the cancel");
    db.drop(c).await;
}

#[tokio::test]
async fn pg_readonly_session_rejects_writes() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_readonly").await else { return };
    q(&mut c, "CREATE TABLE t (a int)").await;
    let mut ro = Connection::connect(&ConnSpec { readonly: true, ..db.spec.clone() }).await.unwrap();
    let e = ro.query("INSERT INTO t VALUES (1)").await.unwrap_err();
    assert_eq!(e.code.as_deref(), Some("25006"));
    assert_eq!(q(&mut ro, "SELECT count(*) FROM t").await.rows[0][0], Value::Int(0));
    drop(ro);
    db.drop(c).await;
}

#[tokio::test]
async fn pg_tls_state_matches_the_server_session() {
    let admin = pg_admin();
    let Some(mut c) = connect_or_skip(&admin, "PostgreSQL").await else { return };
    let server_ssl = scalar(&mut c, "SHOW ssl").await == Value::Text("on".into());
    assert_eq!(c.info().tls, server_ssl, "prefer uses TLS exactly when the server offers it");
    let plain = Connection::connect(&ConnSpec { ssl_mode: SslMode::Disable, ..admin.clone() }).await.unwrap();
    assert!(!plain.info().tls);
    let required = Connection::connect(&ConnSpec { ssl_mode: SslMode::Require, ..admin }).await;
    match required {
        Ok(r) => assert!(server_ssl && r.info().tls),
        Err(e) => assert!(!server_ssl && e.kind == ErrorKind::Connection, "{e:?}"),
    }
}

#[tokio::test]
async fn pg_auth_failures_are_classified_so_callers_can_prompt() {
    let admin = pg_admin();
    let Some(_probe) = connect_or_skip(&admin, "PostgreSQL").await else { return };
    let spec = ConnSpec { user: Some("quarry_no_such_role".into()), ..admin };
    let e = Connection::connect(&spec).await.err().expect("unknown role must fail");
    assert_eq!(e.kind, ErrorKind::Auth, "{e:?}");

    let e = Connection::connect(&ConnSpec { port: Some(1), ..pg_admin() }).await.err().expect("closed port");
    assert_eq!(e.kind, ErrorKind::Connection, "{e:?}");
}

#[tokio::test]
async fn pg_tracks_transactions_and_reports_real_errors_inside_them() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_tx").await else { return };
    assert!(!c.in_transaction());
    q(&mut c, "BEGIN").await;
    assert!(c.in_transaction());
    let e = c.query("SELECT * FROM nope").await.unwrap_err();
    assert_eq!(e.code.as_deref(), Some("42P01"), "describe must not mask the real error with 25P02");
    assert!(c.in_transaction(), "a failed statement leaves the transaction open (aborted)");
    q(&mut c, "ROLLBACK").await;
    assert!(!c.in_transaction());
    db.drop(c).await;
}

const PG_SCHEMA: &[&str] = &[
    "CREATE TYPE mood AS ENUM ('sad', 'happy')",
    "CREATE TABLE authors (id serial PRIMARY KEY, name text NOT NULL, mood mood)",
    "COMMENT ON TABLE authors IS 'writers'",
    "CREATE TABLE books (id int GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
        author_id int REFERENCES authors (id) ON DELETE CASCADE,
        title text CHECK (title <> ''), price numeric(8,2) DEFAULT 0)",
    "CREATE INDEX books_title_idx ON books (lower(title))",
    "CREATE VIEW cheap_books AS SELECT * FROM books WHERE price < 10",
    "CREATE FUNCTION add_one(x int) RETURNS int LANGUAGE sql AS 'SELECT x + 1'",
    "CREATE FUNCTION touch() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NEW; END$$",
    "CREATE TRIGGER books_touch BEFORE INSERT OR UPDATE ON books FOR EACH ROW EXECUTE FUNCTION touch()",
    "INSERT INTO authors (name) VALUES ('ann'), ('bob')",
    "INSERT INTO books (author_id, title, price) VALUES (1, 'one', 5), (2, 'two', 20)",
];

#[tokio::test]
async fn pg_catalog_describes_schema_for_completion() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_catalog").await else { return };
    for sql in PG_SCHEMA {
        q(&mut c, sql).await;
    }
    let started = Instant::now();
    let cat = c.load_catalog().await.unwrap();
    eprintln!("pg load_catalog: {:?}", started.elapsed());
    assert_eq!(cat.current_database.as_deref(), Some(db.name.as_str()));
    assert!(cat.databases.contains(&db.name));
    assert_eq!(cat.search_path.first().map(String::as_str), Some("public"));
    assert_eq!(cat.search_path.last().map(String::as_str), Some("pg_catalog"));

    let authors = cat.find_relation(None, "authors").expect("authors");
    assert_eq!((authors.kind, authors.comment.as_deref()), (RelKind::Table, Some("writers")));
    let id = &authors.columns[0];
    assert!(id.primary_key && id.auto && !id.nullable, "{id:?}");
    assert!(!authors.columns[1].nullable);
    let books = cat.find_relation(None, "books").unwrap();
    assert!(books.columns[0].auto, "identity columns are auto-assigned");
    assert_eq!(books.columns[3].data_type, "numeric(8,2)");
    assert_eq!(cat.find_relation(Some("public"), "cheap_books").unwrap().kind, RelKind::View);
    assert!(cat.find_relation(Some("pg_catalog"), "pg_class").is_some());

    let public = cat.schema("public").unwrap();
    let add_one = public.functions.iter().find(|f| f.name == "add_one").unwrap();
    assert_eq!((add_one.args.as_str(), add_one.return_type.as_str()), ("x integer", "integer"));
    assert_eq!(public.functions.iter().find(|f| f.name == "touch").unwrap().kind, FunctionKind::Trigger);
    assert!(public.types.contains(&"mood".to_string()));
    assert!(cat.functions().any(|f| f.schema == "pg_catalog" && f.name == "now"));

    let fk = cat.foreign_keys.iter().find(|f| f.table == "books").expect("books FK");
    assert_eq!((fk.columns.as_slice(), fk.ref_table.as_str()), (&["author_id".to_string()][..], "authors"));
    assert_eq!((fk.ref_columns[0].as_str(), fk.on_delete.as_deref()), ("id", Some("CASCADE")));
    assert!(cat.users.contains(&"postgres".to_string()));

    let rels = c.list_relations("public").await.unwrap();
    let names: Vec<&str> = rels.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["authors", "books", "cheap_books"]);
    assert!(rels.iter().all(|r| r.columns.is_empty()));
    db.drop(c).await;
}

#[tokio::test]
async fn pg_table_details_and_ddl_cover_structure() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_details").await else { return };
    for sql in PG_SCHEMA {
        q(&mut c, sql).await;
    }
    let d = c.table_details(None, "books").await.unwrap();
    assert_eq!((d.schema.as_str(), d.kind), ("public", RelKind::Table));
    assert_eq!(d.primary_key(), ["id"]);
    let pkey = d.indexes.iter().find(|i| i.primary).unwrap();
    assert_eq!((pkey.name.as_str(), pkey.method.as_deref()), ("books_pkey", Some("btree")));
    let expr = d.indexes.iter().find(|i| i.name == "books_title_idx").unwrap();
    assert_eq!(expr.columns, ["lower(title)"]);
    assert_eq!(d.foreign_keys.len(), 1);
    let kinds: Vec<&str> = d.constraints.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(kinds, ["PRIMARY KEY", "CHECK", "FOREIGN KEY"]);
    assert_eq!(d.triggers[0].event, "BEFORE INSERT OR UPDATE");
    assert!(d.size_bytes.unwrap() > 0);

    let a = c.table_details(Some("public"), "authors").await.unwrap();
    assert_eq!(a.referenced_by.len(), 1);
    assert_eq!(a.referenced_by[0].table, "books");
    assert_eq!(a.comment.as_deref(), Some("writers"));

    let v = c.table_details(None, "cheap_books").await.unwrap();
    assert!(v.view_definition.unwrap().contains("price < "));

    let books = qualified(Some("public"), "books", Backend::Postgres);
    let authors = qualified(Some("public"), "authors", Backend::Postgres);
    let ddl = c.object_ddl(None, "books", "table").await.unwrap();
    assert!(ddl.starts_with(&format!("CREATE TABLE {books} (")), "{ddl}");
    assert!(ddl.contains("id integer GENERATED ALWAYS AS IDENTITY NOT NULL"), "{ddl}");
    assert!(ddl.contains("DEFAULT 0"), "{ddl}");
    assert!(ddl.contains("CREATE INDEX books_title_idx"), "{ddl}");
    assert!(!ddl.contains("CREATE UNIQUE INDEX books_pkey"), "constraint indexes come from the constraint: {ddl}");
    assert!(ddl.contains("CREATE TRIGGER books_touch"), "{ddl}");
    let ddl = c.object_ddl(None, "authors", "table").await.unwrap();
    assert!(ddl.contains(&format!("COMMENT ON TABLE {authors} IS 'writers';")), "{ddl}");

    // The reconstructed DDL must be executable: rebuild authors in another schema from it.
    q(&mut c, "CREATE SCHEMA copy").await;
    q(&mut c, "SET search_path = copy, public").await;
    let rebuilt = ddl.replace(&authors, "copy.authors");
    for stmt in quarry::sql::split::split(&rebuilt, Backend::Postgres, ";") {
        q(&mut c, &stmt.text).await;
    }
    let copy = c.table_details(Some("copy"), "authors").await.unwrap();
    assert_eq!(copy.columns, a.columns);
    q(&mut c, "SET search_path = public").await;

    let view = c.object_ddl(None, "cheap_books", "view").await.unwrap();
    assert!(view.starts_with(&format!("CREATE VIEW {} AS\n", qualified(Some("public"), "cheap_books", Backend::Postgres))));
    let func = c.object_ddl(None, "add_one", "function").await.unwrap();
    assert!(func.starts_with("CREATE OR REPLACE FUNCTION public.add_one(x integer)"), "{func}");
    let ty = c.object_ddl(None, "mood", "type").await.unwrap();
    assert_eq!(ty, "CREATE TYPE mood AS ENUM ('sad', 'happy');\n");
    let seq = c.object_ddl(None, "authors_id_seq", "sequence").await.unwrap();
    assert!(seq.contains("authors_id_seq AS integer START WITH 1"), "{seq}");
    db.drop(c).await;
}

#[tokio::test]
async fn pg_explain_builds_tree_and_analyze_has_no_side_effects() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_explain").await else { return };
    for sql in PG_SCHEMA {
        q(&mut c, sql).await;
    }
    let plan = c.explain("SELECT * FROM books b JOIN authors a ON a.id = b.author_id", false).await.unwrap();
    assert!(plan.label.contains("Join") || plan.label.contains("Nested Loop"), "{plan:?}");
    assert!(plan.total_cost.is_some() && plan.actual_time_ms.is_none());
    assert_eq!(plan.children.len(), 2);

    let plan = c.explain("DELETE FROM books", true).await.unwrap();
    assert!(plan.label.starts_with("Delete on books"), "{plan:?}");
    assert!(plan.actual_time_ms.is_some());
    assert!(plan.details.iter().any(|(k, _)| k == "Execution Time"));
    assert_eq!(scalar(&mut c, "SELECT count(*) FROM books").await, Value::Int(2), "ANALYZE must be rolled back");
    assert!(!c.in_transaction());

    q(&mut c, "BEGIN").await;
    q(&mut c, "INSERT INTO authors (name) VALUES ('cy')").await;
    c.explain("DELETE FROM authors", true).await.unwrap();
    assert!(c.in_transaction(), "explain inside a transaction must keep it open");
    assert_eq!(scalar(&mut c, "SELECT count(*) FROM authors").await, Value::Int(3), "only the savepoint is undone");
    q(&mut c, "ROLLBACK").await;
    db.drop(c).await;
}

#[tokio::test]
async fn pg_activity_lists_and_kills_other_sessions() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_activity").await else { return };
    let mut victim = db.connect().await;
    let pid = victim.info().session_id.clone().unwrap();
    let rs = c.activity().await.unwrap();
    assert_eq!(rs.columns[0].name, "pid");
    let mine = c.info().session_id.clone().unwrap();
    let pids: Vec<String> = rs.rows.iter().map(|r| r[0].display().into_owned()).collect();
    assert!(pids.contains(&pid) && !pids.contains(&mine), "{pids:?}");
    c.kill_session(&pid).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let e = victim.query("SELECT 1").await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Connection, "{e:?}");
    drop(victim);
    db.drop(c).await;
}

#[tokio::test]
async fn pg_change_database_reconnects_and_keeps_old_on_failure() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_switch").await else { return };
    c.change_database("postgres").await.unwrap();
    assert_eq!(c.info().database.as_deref(), Some("postgres"));
    assert!(c.change_database("quarry_test_does_not_exist").await.is_err());
    assert_eq!(scalar(&mut c, "SELECT current_database()").await, Value::Text("postgres".into()));
    assert!(c.load_extension("/nope.so").await.is_err());
    db.drop(c).await;
}

#[tokio::test]
async fn my_values_follow_column_types_and_flags() {
    let Some((db, mut c)) = TestDb::create(my_admin(), "my_values").await else { return };
    q(
        &mut c,
        "CREATE TABLE typed (i int, u bigint unsigned, f double, d decimal(6,2), t varchar(10), raw varbinary(4),
                             dt date, bits bit(8), j json, e enum('a','b'))",
    )
    .await;
    q(
        &mut c,
        "INSERT INTO typed VALUES (-1, 18446744073709551615, 1.5, 12.50, 'x', x'00ff', '2024-01-02', b'101',
                                   '{\"k\": 1}', 'b')",
    )
    .await;
    let rs = q(&mut c, "SELECT *, NULL AS n FROM typed").await;
    assert_eq!(
        rs.rows[0],
        vec![
            Value::Int(-1),
            Value::UInt(u64::MAX),
            Value::Float(1.5),
            Value::Text("12.50".into()),
            Value::Text("x".into()),
            Value::Bytes(vec![0, 255]),
            Value::Text("2024-01-02".into()),
            Value::Int(5),
            Value::Text("{\"k\": 1}".into()),
            Value::Text("b".into()),
            Value::Null,
        ]
    );
    let names: Vec<&str> = rs.columns.iter().map(|c| c.type_name.as_str()).collect();
    assert_eq!(
        names,
        ["INT", "BIGINT UNSIGNED", "DOUBLE", "DECIMAL", "VARCHAR", "VARBINARY", "DATE", "BIT", "JSON", "ENUM", "NULL"]
    );
    use TypeKind::*;
    assert_eq!(kinds(&rs)[..10], [Integer, Integer, Float, Decimal, Text, Binary, Date, Integer, Json, Text]);
    assert_eq!(rs.summary.status.as_deref(), Some("1 row in set"));
    assert_eq!(scalar(&mut c, "SELECT @@character_set_client").await, Value::Text("utf8mb4".into()));
    assert_eq!(scalar(&mut c, "SELECT '🦀'").await, Value::Text("🦀".into()), "4-byte UTF-8 survives");
    db.drop(c).await;
}

#[tokio::test]
async fn my_streams_and_reports_status_warnings() {
    let Some((db, mut c)) = TestDb::create(my_admin(), "my_stream").await else { return };
    let sql = "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 1000) SELECT n FROM s";
    let (res, evs) = events(&mut c, sql).await;
    res.unwrap();
    let (batches, rows) = assert_stream_shape(&evs);
    assert_eq!((rows, batches >= 4), (1000, true));
    assert_eq!(done(&evs).status.as_deref(), Some("1000 rows in set"));

    q(&mut c, "CREATE TABLE t (id int AUTO_INCREMENT PRIMARY KEY, v int)").await;
    let rs = q(&mut c, "INSERT INTO t (v) VALUES (1), (2), (3)").await;
    assert_eq!(rs.summary.status.as_deref(), Some("Query OK, 3 rows affected"));
    assert_eq!((rs.summary.rows_affected, rs.summary.last_insert_id), (Some(3), Some(1)));

    let (res, evs) = events(&mut c, "SELECT CAST('12abc' AS SIGNED)").await;
    res.unwrap();
    assert_eq!(done(&evs).warnings, 1);
    let warning = evs.iter().find_map(|e| match e {
        ExecEvent::Notice(n) => Some(n),
        _ => None,
    });
    let warning = warning.expect("warnings are forwarded as notices");
    assert_eq!(warning.severity, "Warning");
    assert!(warning.message.contains("Truncated") && warning.message.ends_with("(1292)"), "{warning:?}");
    db.drop(c).await;
}

#[tokio::test]
async fn my_call_with_several_result_sets_starts_each_with_columns() {
    let Some((db, mut c)) = TestDb::create(my_admin(), "my_call").await else { return };
    q(&mut c, "CREATE PROCEDURE two_sets() BEGIN SELECT 1 AS a; SELECT 'x' AS b, 2 AS c; END").await;
    let (res, evs) = events(&mut c, "CALL two_sets()").await;
    res.unwrap();
    let columns: Vec<Vec<String>> = evs
        .iter()
        .filter_map(|e| match e {
            ExecEvent::Columns(c) => Some(c.iter().map(|c| c.name.clone()).collect()),
            _ => None,
        })
        .collect();
    assert_eq!(columns, [vec!["a"], vec!["b", "c"]]);
    let rows: Vec<&Vec<Row>> = evs
        .iter()
        .filter_map(|e| match e {
            ExecEvent::Rows(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(rows[0][0], vec![Value::Int(1)]);
    assert_eq!(rows[1][0], vec![Value::Text("x".into()), Value::Int(2)]);
    assert!(matches!(evs.last(), Some(ExecEvent::Done(_))));
    assert_eq!(scalar(&mut c, "SELECT 5").await, Value::Int(5), "connection is clean after multi-result CALL");
    db.drop(c).await;
}

#[tokio::test]
async fn my_errors_and_auth_are_classified() {
    let admin = my_admin();
    let Some(_probe) = connect_or_skip(&admin, "MySQL").await else { return };
    let Some((db, mut c)) = TestDb::create(admin.clone(), "my_errors").await else { return };
    let e = c.query("SELECT * FROM missing").await.unwrap_err();
    assert_eq!((e.kind, e.code.as_deref()), (ErrorKind::Query, Some("1146")));
    // a non-existent account: exercises 1045 without risking any lockout on real accounts
    let spec = ConnSpec { user: Some("quarry_no_such_user".into()), password: Some("x".into()), ..admin };
    let e = Connection::connect(&spec).await.err().expect("bad credentials must fail");
    assert_eq!((e.kind, e.code.as_deref()), (ErrorKind::Auth, Some("1045")));
    db.drop(c).await;
}

#[tokio::test]
async fn my_cancel_kills_running_query() {
    let Some((db, mut c)) = TestDb::create(my_admin(), "my_cancel").await else { return };
    // KILL QUERY makes SLEEP return early (value 1) rather than fail
    let (res, took) = cancel_after(&mut c, "SELECT SLEEP(10)", Duration::from_millis(300)).await;
    assert!(took < Duration::from_secs(2), "cancel took {took:?}");
    match res {
        Ok(rs) => assert_eq!(rs.rows[0][0], Value::Int(1)),
        Err(e) => assert_eq!(e.kind, ErrorKind::Cancelled),
    }
    let huge = "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 1000)
                SELECT COUNT(*) FROM s a, s b, s c";
    let (res, took) = cancel_after(&mut c, huge, Duration::from_millis(300)).await;
    let e = res.unwrap_err();
    assert_eq!((e.kind, e.code.as_deref()), (ErrorKind::Cancelled, Some("1317")));
    assert!(took < Duration::from_secs(2), "cancel took {took:?}");
    assert_eq!(scalar(&mut c, "SELECT 7").await, Value::Int(7));
    db.drop(c).await;
}

#[tokio::test]
async fn my_dropping_the_receiver_stops_server_work() {
    let Some((db, mut c)) = TestDb::create(my_admin(), "my_abandon").await else { return };
    // ~10s of server work if it ran to completion
    let sql = "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 1000)
               SELECT a.n, REPEAT('x', 500), SLEEP(0.002) FROM s a, (SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3
               UNION ALL SELECT 4 UNION ALL SELECT 5) b";
    let (res, took) = abandon_after_first_batch(&mut c, sql).await;
    assert_eq!(res.unwrap_err().kind, ErrorKind::Cancelled);
    assert!(took < Duration::from_secs(4), "abandoning took {took:?}");
    assert_eq!(scalar(&mut c, "SELECT 8").await, Value::Int(8));
    db.drop(c).await;
}

#[tokio::test]
async fn my_tls_modes_negotiate_and_verify() {
    let admin = my_admin();
    let Some(mut c) = connect_or_skip(&admin, "MySQL").await else { return };
    let cipher = |rs: ResultSet| rs.rows.first().map(|r| r[1].display().into_owned()).unwrap_or_default();
    let negotiated = cipher(q(&mut c, "SHOW SESSION STATUS LIKE 'Ssl_cipher'").await);
    assert_eq!(c.info().tls, !negotiated.is_empty(), "info reports the negotiated state");
    let mut plain = Connection::connect(&ConnSpec { ssl_mode: SslMode::Disable, ..admin.clone() }).await.unwrap();
    assert!(!plain.info().tls);
    assert_eq!(cipher(q(&mut plain, "SHOW SESSION STATUS LIKE 'Ssl_cipher'").await), "");
    if !c.info().tls {
        return;
    }
    let local = matches!(admin.host.as_deref(), Some("127.0.0.1" | "localhost" | "::1"));
    if local {
        // a local test server's self-signed certificate must not pass full verification
        let e = Connection::connect(&ConnSpec { ssl_mode: SslMode::VerifyFull, ..admin.clone() }).await.err();
        assert_eq!(e.expect("self-signed cert rejected").kind, ErrorKind::Connection);
    }
    // QUARRY_TEST_MYSQL_CA: the server's CA file (e.g. `docker cp mysql:/var/lib/mysql/ca.pem .`)
    if let Ok(ca) = std::env::var("QUARRY_TEST_MYSQL_CA") {
        let spec = ConnSpec { ssl_mode: SslMode::VerifyCa, ssl_ca: Some(ca.into()), ..admin.clone() };
        match Connection::connect(&spec).await {
            Ok(verified) => assert!(verified.info().tls),
            // MySQL's auto-generated certificates have no subjectAltName (see mysql.rs open_verify_ca)
            Err(e) => assert!(e.message.contains("subjectAltName"), "{e:?}"),
        }
        if local {
            let e = Connection::connect(&ConnSpec { ssl_mode: SslMode::VerifyFull, ..spec }).await.err();
            assert!(e.is_some(), "verify-full also checks the host name, which auto-generated certs don't carry");
        }
    }
}

#[tokio::test]
async fn my_readonly_and_transaction_state() {
    let Some((db, mut c)) = TestDb::create(my_admin(), "my_readonly").await else { return };
    q(&mut c, "CREATE TABLE t (a int)").await;
    let mut ro = Connection::connect(&ConnSpec { readonly: true, ..db.spec.clone() }).await.unwrap();
    let e = ro.query("INSERT INTO t VALUES (1)").await.unwrap_err();
    assert_eq!(e.code.as_deref(), Some("1792"), "{e:?}");
    drop(ro);

    q(&mut c, "START TRANSACTION").await;
    assert!(c.in_transaction());
    q(&mut c, "INSERT INTO t VALUES (1)").await;
    assert!(c.in_transaction());
    q(&mut c, "ROLLBACK").await;
    assert!(!c.in_transaction());

    q(&mut c, "USE mysql").await;
    assert_eq!(c.info().database.as_deref(), Some("mysql"), "USE through execute updates the session db");
    c.change_database(&db.name).await.unwrap();
    assert_eq!(scalar(&mut c, "SELECT DATABASE()").await, Value::Text(db.name.clone()));
    db.drop(c).await;
}

const MY_SCHEMA: &[&str] = &[
    "CREATE TABLE authors (id int AUTO_INCREMENT PRIMARY KEY, name varchar(50) NOT NULL COMMENT 'full name')
     COMMENT 'writers'",
    "CREATE TABLE books (id int AUTO_INCREMENT PRIMARY KEY, author_id int, title varchar(100),
        price decimal(8,2) DEFAULT 0, UNIQUE KEY uq_title (title), KEY idx_lower ((lower(title))),
        CONSTRAINT fk_author FOREIGN KEY (author_id) REFERENCES authors (id) ON DELETE CASCADE,
        CONSTRAINT price_positive CHECK (price >= 0))",
    "CREATE VIEW cheap_books AS SELECT * FROM books WHERE price < 10",
    "CREATE FUNCTION add_one(x int) RETURNS int DETERMINISTIC RETURN x + 1",
    "CREATE PROCEDURE noop(IN a int, OUT b int) BEGIN SET b = a; END",
    "CREATE TRIGGER books_touch BEFORE INSERT ON books FOR EACH ROW SET NEW.title = TRIM(NEW.title)",
    "INSERT INTO authors (name) VALUES ('ann'), ('bob')",
    "INSERT INTO books (author_id, title, price) VALUES (1, 'one', 5), (2, 'two', 20)",
];

#[tokio::test]
async fn my_catalog_loads_current_database_eagerly() {
    let Some((db, mut c)) = TestDb::create(my_admin(), "my_catalog").await else { return };
    for sql in MY_SCHEMA {
        q(&mut c, sql).await;
    }
    let started = Instant::now();
    let cat = c.load_catalog().await.unwrap();
    eprintln!("mysql load_catalog: {:?}", started.elapsed());
    assert_eq!(cat.current_database.as_deref(), Some(db.name.as_str()));
    assert_eq!(cat.search_path, std::slice::from_ref(&db.name));
    assert!(cat.schema("mysql").is_some_and(|s| s.relations.is_empty()), "other databases are loaded lazily");
    let authors = cat.find_relation(None, "authors").unwrap();
    assert_eq!(authors.comment.as_deref(), Some("writers"));
    let id = &authors.columns[0];
    assert!(id.primary_key && id.auto && !id.nullable);
    assert_eq!(authors.columns[1].comment.as_deref(), Some("full name"));
    assert_eq!(cat.find_relation(None, "cheap_books").unwrap().kind, RelKind::View);
    let s = cat.schema(&db.name).unwrap();
    let add_one = s.functions.iter().find(|f| f.name == "add_one").unwrap();
    assert_eq!((add_one.args.as_str(), add_one.return_type.as_str()), ("x int", "int"));
    let noop = s.functions.iter().find(|f| f.name == "noop").unwrap();
    assert_eq!((noop.kind, noop.args.as_str()), (FunctionKind::Procedure, "IN a int, OUT b int"));
    let fk = &cat.foreign_keys[0];
    assert_eq!((fk.name.as_str(), fk.table.as_str(), fk.ref_table.as_str()), ("fk_author", "books", "authors"));
    assert_eq!(fk.on_delete.as_deref(), Some("CASCADE"));
    assert!(cat.users.iter().any(|u| u.starts_with("'root'@")), "{:?}", cat.users);
    let rels = c.list_relations("mysql").await.unwrap();
    assert!(rels.iter().any(|r| r.name == "user"));
    // Other databases load lazily; their columns are what completion offers after `db.table.`.
    let user = rels.iter().find(|r| r.name == "user").unwrap();
    assert!(user.columns.iter().any(|col| col.name == "Host"), "{:?}", user.columns);
    assert_eq!(rels.iter().filter(|r| r.name == "user").count(), 1, "one relation per table, not per column");
    db.drop(c).await;
}

#[tokio::test]
async fn my_table_details_ddl_and_explain() {
    let Some((db, mut c)) = TestDb::create(my_admin(), "my_details").await else { return };
    for sql in MY_SCHEMA {
        q(&mut c, sql).await;
    }
    let d = c.table_details(None, "books").await.unwrap();
    assert_eq!(d.primary_key(), ["id"]);
    let idx: Vec<(&str, bool, bool)> = d.indexes.iter().map(|i| (i.name.as_str(), i.unique, i.primary)).collect();
    assert_eq!(
        idx,
        [("PRIMARY", true, true), ("fk_author", false, false), ("idx_lower", false, false), ("uq_title", true, false)]
    );
    assert_eq!(d.indexes[2].columns, ["(lower(`title`))"]);
    assert_eq!(d.foreign_keys[0].ref_columns, ["id"]);
    let check = d.constraints.iter().find(|c| c.kind == "CHECK").unwrap();
    assert!(check.definition.starts_with("CHECK (") && check.definition.contains("price"), "{check:?}");
    assert_eq!(d.triggers[0].event, "BEFORE INSERT");
    let a = c.table_details(Some(&db.name), "authors").await.unwrap();
    assert_eq!(a.referenced_by[0].table, "books");
    let v = c.table_details(None, "cheap_books").await.unwrap();
    assert_eq!(v.kind, RelKind::View);
    assert!(v.view_definition.is_some());

    let ddl = c.object_ddl(None, "books", "table").await.unwrap();
    assert!(ddl.starts_with("CREATE TABLE `books`") && ddl.contains("fk_author"), "{ddl}");
    let ddl = c.object_ddl(None, "noop", "procedure").await.unwrap();
    assert!(ddl.contains("PROCEDURE `noop`"), "{ddl}");
    let ddl = c.object_ddl(None, "books_touch", "trigger").await.unwrap();
    assert!(ddl.contains("TRIGGER `books_touch`"), "{ddl}");

    let plan = c.explain("SELECT * FROM books b JOIN authors a ON a.id = b.author_id", false).await.unwrap();
    assert!(plan.label.starts_with("Query block"), "{plan:?}");
    assert!(plan.total_cost.is_some());
    fn has_table(n: &PlanNode, t: &str) -> bool {
        n.label.contains(&format!(" on {t}")) || n.children.iter().any(|c| has_table(c, t))
    }
    assert!(has_table(&plan, "b") && has_table(&plan, "a"), "{plan:#?}");

    let plan = c.explain("SELECT * FROM books WHERE price > 1", true).await.unwrap();
    assert!(plan.actual_rows.is_some() && plan.actual_time_ms.is_some(), "{plan:#?}");
    if let Ok(plan) = c.explain("DELETE b FROM books b JOIN authors a ON a.id = b.author_id", true).await {
        assert!(plan.actual_time_ms.is_some() || !plan.children.is_empty(), "{plan:#?}");
    }
    assert_eq!(scalar(&mut c, "SELECT COUNT(*) FROM books").await, Value::Int(2), "ANALYZE must be rolled back");
    assert!(!c.in_transaction());
    db.drop(c).await;
}

#[tokio::test]
async fn my_activity_lists_and_kills_other_sessions() {
    let Some((db, mut c)) = TestDb::create(my_admin(), "my_activity").await else { return };
    let mut victim = db.connect().await;
    let id = victim.info().session_id.clone().unwrap();
    let rs = c.activity().await.unwrap();
    let ids: Vec<String> = rs.rows.iter().map(|r| r[0].display().into_owned()).collect();
    assert!(ids.contains(&id), "{ids:?}");
    assert!(!ids.contains(c.info().session_id.as_ref().unwrap()), "own session is excluded");
    c.kill_session(&id).await.unwrap();
    let e = victim.query("SELECT 1").await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Connection, "{e:?}");
    drop(victim);
    db.drop(c).await;
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("quarry_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn lite(path: impl Into<PathBuf>) -> Connection {
    Connection::connect(&ConnSpec::sqlite(path)).await.expect("open sqlite")
}

#[tokio::test]
async fn lite_values_use_declared_type_or_storage_class() {
    let mut c = lite(":memory:").await;
    q(&mut c, "CREATE TABLE t (i INTEGER, r REAL, s VARCHAR(10), b BLOB, d DECIMAL(6,2), n)").await;
    // NUMERIC affinity stores '12.50' as REAL: values reflect SQLite's storage, kinds the declaration
    q(&mut c, "INSERT INTO t VALUES (1, 1.5, 'x', x'00ff', '12.50', NULL)").await;
    let rs = q(&mut c, "SELECT *, i + 1 AS expr FROM t").await;
    assert_eq!(
        rs.rows[0],
        vec![
            Value::Int(1),
            Value::Float(1.5),
            Value::Text("x".into()),
            Value::Bytes(vec![0, 255]),
            Value::Float(12.5),
            Value::Null,
            Value::Int(2),
        ]
    );
    use TypeKind::*;
    assert_eq!(kinds(&rs), [Integer, Float, Text, Binary, Decimal, Other, Integer]);
    assert_eq!(rs.columns[6].type_name, "INTEGER", "untyped expressions take the first row's storage class");
    assert_eq!(rs.summary.status.as_deref(), Some("SELECT 1"));
    let rs = q(&mut c, "SELECT * FROM t WHERE 0").await;
    assert_eq!((rs.columns.len(), rs.rows.len()), (6, 0), "empty results still describe their columns");
}

#[tokio::test]
async fn lite_streams_batches_and_reports_changes() {
    let mut c = lite(":memory:").await;
    let sql = "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 1000) SELECT n FROM s";
    let (res, evs) = events(&mut c, sql).await;
    res.unwrap();
    let (batches, rows) = assert_stream_shape(&evs);
    assert_eq!((rows, batches >= 4), (1000, true));
    q(&mut c, "CREATE TABLE t (a)").await;
    let rs = q(&mut c, "INSERT INTO t VALUES (1), (2), (3)").await;
    assert_eq!((rs.summary.status.as_deref(), rs.summary.rows_affected), (Some("INSERT 3"), Some(3)));
    let rs = q(&mut c, "DELETE FROM t WHERE a > 1 RETURNING a").await;
    assert_eq!((rs.rows.len(), rs.summary.status.as_deref()), (2, Some("DELETE 2")));
    let rs = q(&mut c, "CREATE INDEX ta ON t (a)").await;
    assert_eq!((rs.summary.status.as_deref(), rs.summary.rows_affected), (Some("OK"), None));
    let rs = q(&mut c, "-- only a comment").await;
    assert_eq!(rs.summary.status.as_deref(), Some("OK"));
}

#[tokio::test]
async fn lite_cancel_interrupts_long_statement() {
    let mut c = lite(":memory:").await;
    let endless = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";
    let (res, took) = cancel_after(&mut c, endless, Duration::from_millis(200)).await;
    assert_eq!(res.unwrap_err().kind, ErrorKind::Cancelled);
    assert!(took < Duration::from_secs(1), "interrupt took {took:?}");
    assert_eq!(scalar(&mut c, "SELECT 1").await, Value::Int(1));

    let rows = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c";
    let (res, took) = abandon_after_first_batch(&mut c, rows).await;
    assert_eq!(res.unwrap_err().kind, ErrorKind::Cancelled);
    assert!(took < Duration::from_secs(1), "abandoning took {took:?}");
}

#[tokio::test]
async fn lite_readonly_never_writes_or_creates_files() {
    let dir = TempDir::new("lite_ro");
    let path = dir.file("data.db");
    let mut rw = lite(&path).await;
    q(&mut rw, "CREATE TABLE t (a)").await;
    drop(rw);
    let mut ro = Connection::connect(&ConnSpec { readonly: true, ..ConnSpec::sqlite(&path) }).await.unwrap();
    let e = ro.query("INSERT INTO t VALUES (1)").await.unwrap_err();
    assert_eq!((e.kind, e.code.as_deref()), (ErrorKind::Query, Some("8")), "SQLITE_READONLY: {e:?}");
    let missing = dir.file("missing.db");
    let e = Connection::connect(&ConnSpec { readonly: true, ..ConnSpec::sqlite(&missing) }).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Connection);
    assert!(!missing.exists());
}

#[tokio::test]
async fn lite_errors_transactions_and_unsupported_features() {
    let dir = TempDir::new("lite_misc");
    let mut c = lite(dir.file("a.db")).await;
    let e = c.query("SELECT * FROM nope").await.unwrap_err();
    assert_eq!((e.kind, e.code.as_deref()), (ErrorKind::Query, Some("1")));
    let e = c.query("SELECT 1; x").await.unwrap_err();
    assert_eq!(e.position, Some(11), "{e:?}");
    q(&mut c, "BEGIN").await;
    assert!(c.in_transaction());
    q(&mut c, "COMMIT").await;
    assert!(!c.in_transaction());
    assert_eq!(c.activity().await.unwrap_err().kind, ErrorKind::Other);
    assert_eq!(c.kill_session("1").await.unwrap_err().kind, ErrorKind::Other);
    assert!(c.load_extension(dir.file("nope.so").to_str().unwrap()).await.is_err());

    q(&mut c, "CREATE TABLE only_in_a (x)").await;
    let other = dir.file("b.db");
    c.change_database(other.to_str().unwrap()).await.unwrap();
    assert_eq!(c.info().database.as_deref(), other.to_str());
    assert!(c.query("SELECT * FROM only_in_a").await.is_err());
    let (res, _) = cancel_after(
        &mut c,
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c",
        Duration::from_millis(100),
    )
    .await;
    assert_eq!(res.unwrap_err().kind, ErrorKind::Cancelled, "cancel follows the new database handle");
}

const LITE_SCHEMA: &[&str] = &[
    "CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE)",
    "CREATE TABLE books (id INTEGER PRIMARY KEY, author_id INTEGER REFERENCES authors ON DELETE CASCADE,
        title TEXT DEFAULT 'untitled', price NUMERIC)",
    "CREATE INDEX books_title ON books (title, lower(title))",
    "CREATE VIEW cheap AS SELECT * FROM books WHERE price < 10",
    "CREATE TRIGGER books_touch AFTER UPDATE ON books BEGIN SELECT 1; END",
    "INSERT INTO authors (name) VALUES ('ann'), ('bob')",
    "INSERT INTO books (author_id, title, price) VALUES (1, 'one', 5), (2, 'two', 20)",
];

#[tokio::test]
async fn lite_catalog_details_ddl_and_explain() {
    let dir = TempDir::new("lite_catalog");
    let mut c = lite(dir.file("main.db")).await;
    for sql in LITE_SCHEMA {
        q(&mut c, sql).await;
    }
    let aux = dir.file("aux.db");
    q(&mut c, &format!("ATTACH DATABASE '{}' AS aux", aux.display())).await;
    q(&mut c, "CREATE TABLE aux.extra (k TEXT PRIMARY KEY)").await;

    let cat = c.load_catalog().await.unwrap();
    let names: Vec<&str> = cat.schemas.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["main", "aux"], "temp is omitted while empty");
    assert_eq!(cat.search_path, ["main", "aux"]);
    let books = cat.find_relation(None, "books").unwrap();
    let cols: Vec<&str> = books.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cols, ["id", "author_id", "title", "price"]);
    assert!(books.columns[0].primary_key && books.columns[0].auto, "INTEGER PRIMARY KEY aliases rowid");
    assert_eq!(books.columns[2].default.as_deref(), Some("'untitled'"));
    assert_eq!(cat.find_relation(None, "cheap").unwrap().kind, RelKind::View);
    assert!(cat.find_relation(Some("aux"), "extra").is_some());
    let fk = cat.foreign_keys.iter().find(|f| f.table == "books").unwrap();
    assert_eq!(fk.ref_columns, ["id"], "implicit REFERENCES target resolves to the primary key");
    assert_eq!(fk.on_delete.as_deref(), Some("CASCADE"));
    assert!(cat.functions().any(|f| f.name == "abs"));

    q(&mut c, "CREATE TEMP TABLE scratch (x)").await;
    let cat = c.load_catalog().await.unwrap();
    assert_eq!(cat.search_path, ["temp", "main", "aux"], "SQLite resolves temp first");

    let d = c.table_details(None, "books").await.unwrap();
    assert_eq!(d.schema, "main");
    let ix = d.indexes.iter().find(|i| i.name == "books_title").unwrap();
    assert_eq!(ix.columns, ["title", "<expr>"]);
    assert!(ix.definition.as_deref().unwrap().starts_with("CREATE INDEX books_title"));
    assert_eq!(d.foreign_keys.len(), 1);
    assert_eq!(d.triggers[0].event, "AFTER UPDATE");
    let a = c.table_details(None, "authors").await.unwrap();
    assert_eq!(a.referenced_by[0].table, "books");
    assert!(a.indexes.iter().any(|i| i.unique && i.columns == ["name"]));
    assert!(a.constraints.iter().any(|c| c.definition == "UNIQUE (name)"), "{:?}", a.constraints);
    let v = c.table_details(None, "cheap").await.unwrap();
    assert_eq!(v.view_definition.as_deref(), Some("SELECT * FROM books WHERE price < 10"));

    let ddl = c.object_ddl(None, "books", "table").await.unwrap();
    assert!(ddl.starts_with("CREATE TABLE books ("), "{ddl}");
    assert!(ddl.contains("CREATE INDEX books_title") && ddl.contains("CREATE TRIGGER books_touch"), "{ddl}");

    let plan = c.explain("SELECT * FROM books b JOIN authors a ON a.id = b.author_id WHERE b.title = 'x'", true).await;
    let plan = plan.unwrap();
    assert_eq!(plan.label, "QUERY PLAN");
    let labels: Vec<&str> = plan.children.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.starts_with("SEARCH b USING INDEX books_title")), "{labels:?}");
    assert!(labels.iter().any(|l| l.starts_with("SEARCH a USING INTEGER PRIMARY KEY")), "{labels:?}");
}

#[tokio::test]
#[ignore = "slow: builds 3000 tables; run with --ignored to check catalog latency"]
async fn pg_catalog_stays_fast_with_thousands_of_tables() {
    let Some((db, mut c)) = TestDb::create(pg_admin(), "pg_perf").await else { return };
    q(
        &mut c,
        "DO $$ BEGIN FOR i IN 1..3000 LOOP
            EXECUTE format('CREATE TABLE t%s (id serial PRIMARY KEY, a int, b text, c numeric, d date,
                            e jsonb, f bool, g int REFERENCES t1 (id), h text DEFAULT %L, i timestamptz)',
                           i, 'x');
            IF i % 500 = 0 THEN COMMIT; END IF;
         END LOOP; END $$",
    )
    .await;
    q(&mut c, "ANALYZE").await;
    let started = Instant::now();
    let cat = c.load_catalog().await.unwrap();
    let took = started.elapsed();
    eprintln!("pg load_catalog with 3000 tables: {took:?}");
    assert!(cat.relations().filter(|r| r.schema == "public").count() >= 3000);
    assert!(took < Duration::from_millis(200), "catalog load took {took:?}");
    db.drop(c).await;
}
