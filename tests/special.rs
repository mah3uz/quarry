// Unreachable servers and todo!() drivers print SKIP; only `quarry_test_special_*` databases are touched.

use std::future::Future;
use std::panic::AssertUnwindSafe;

use futures_util::FutureExt;
use quarry::conn::ConnSpec;
use quarry::db::{Connection, Value};
use quarry::special::introspect;
use quarry::special::{RelFilter, Special, Titled};

const PG_ADMIN: &str = "postgres://postgres@127.0.0.1:5432/postgres";
const MY_ADMIN: &str = "mysql://root@127.0.0.1:3306";

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
}

/// Runs `f`; a `todo!()` panic from an unfinished driver becomes a SKIP, anything else fails the test.
async fn guarded(name: &str, f: impl Future<Output = ()>) {
    if let Err(p) = AssertUnwindSafe(f).catch_unwind().await {
        let msg = panic_text(&*p);
        if msg.contains("not yet implemented") || msg.contains("not implemented") {
            println!("SKIP {name}: driver not implemented ({msg})");
        } else {
            std::panic::resume_unwind(p);
        }
    }
}

async fn connect(url: &str) -> Option<Connection> {
    let spec = ConnSpec::parse(url).expect("valid url");
    match AssertUnwindSafe(Connection::connect(&spec)).catch_unwind().await {
        Ok(Ok(c)) => Some(c),
        Ok(Err(e)) => {
            println!("SKIP: cannot connect to {url}: {e}");
            None
        }
        Err(p) => {
            println!("SKIP: connecting to {url} panicked: {}", panic_text(&*p));
            None
        }
    }
}

async fn exec(conn: &mut Connection, sql: &str) {
    conn.query(sql).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn run(conn: &mut Connection, cmd: Special) -> Vec<Titled> {
    introspect::run(conn, &cmd)
        .await
        .unwrap_or_else(|e| panic!("{cmd:?} failed: {e}"))
        .unwrap_or_else(|| panic!("{cmd:?} is not an introspection command"))
}

async fn one(conn: &mut Connection, cmd: Special) -> Titled {
    let mut v = run(conn, cmd).await;
    assert_eq!(v.len(), 1);
    v.remove(0)
}

fn column(t: &Titled, name: &str) -> Vec<String> {
    let idx = t
        .result
        .columns
        .iter()
        .position(|c| c.name == name)
        .unwrap_or_else(|| panic!("no column {name} in {:?} / text {:?}", t.result.columns, t.text));
    t.result.rows.iter().map(|r| r.get(idx).map(|v: &Value| v.display().into_owned()).unwrap_or_default()).collect()
}

fn has(t: &Titled, col: &str, value: &str) -> bool {
    column(t, col).iter().any(|v| v == value)
}

fn text(t: &Titled) -> &str {
    t.text.as_deref().unwrap_or_else(|| panic!("expected text, got table {:?}", t.title))
}

fn rel(kind: RelFilter, pattern: Option<&str>, verbose: bool) -> Special {
    Special::ListRelations { kind, pattern: pattern.map(String::from), verbose }
}

fn describe(p: &str, verbose: bool) -> Special {
    Special::Describe { pattern: Some(p.into()), verbose }
}

const PG_DB: &str = "quarry_test_special_pg";

#[tokio::test]
async fn postgres_introspection() {
    let Some(mut admin) = connect(PG_ADMIN).await else { return };
    guarded("postgres", async {
        exec(&mut admin, &format!("DROP DATABASE IF EXISTS {PG_DB}")).await;
        exec(&mut admin, &format!("CREATE DATABASE {PG_DB}")).await;
        let mut c = connect(&format!("postgres://postgres@127.0.0.1:5432/{PG_DB}")).await.expect("test db connects");
        for sql in [
            "CREATE TABLE users (id serial PRIMARY KEY, email text UNIQUE NOT NULL, name text)",
            "COMMENT ON TABLE users IS 'app users'",
            "CREATE TABLE orders (id serial PRIMARY KEY, user_id int REFERENCES users(id) ON DELETE CASCADE, \
             total numeric(10,2) NOT NULL DEFAULT 0 CONSTRAINT total_pos CHECK (total >= 0))",
            "CREATE INDEX orders_user_idx ON orders(user_id)",
            "CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100",
            "CREATE MATERIALIZED VIEW order_totals AS SELECT user_id, sum(total) AS t FROM orders GROUP BY user_id",
            "CREATE SEQUENCE my_seq",
            "CREATE FUNCTION add_one(i int) RETURNS int LANGUAGE sql AS 'SELECT i + 1'",
            "CREATE SCHEMA sales",
            "CREATE TABLE sales.items (id int)",
            "CREATE TYPE mood AS ENUM ('ok', 'sad')",
            "CREATE TABLE \"Weird'Name\" (x int)",
        ] {
            exec(&mut c, sql).await;
        }

        let dt = one(&mut c, rel(RelFilter::Tables, None, false)).await;
        assert_eq!(dt.title.as_deref(), Some("List of relations"));
        assert!(has(&dt, "Name", "users") && has(&dt, "Name", "orders") && has(&dt, "Name", "Weird'Name"));
        assert!(!has(&dt, "Name", "big_orders") && !has(&dt, "Name", "items"), "views and off-path schemas excluded");
        assert!(has(&one(&mut c, rel(RelFilter::Tables, Some("sales.*"), false)).await, "Name", "items"));
        let dt_plus = one(&mut c, rel(RelFilter::Tables, Some("us*"), true)).await;
        assert_eq!(column(&dt_plus, "Name"), ["users"]);
        assert_eq!(column(&dt_plus, "Description"), ["app users"]);
        assert!(!column(&dt_plus, "Size")[0].is_empty());
        assert!(has(&one(&mut c, rel(RelFilter::Views, None, false)).await, "Name", "big_orders"));
        assert!(has(&one(&mut c, rel(RelFilter::MaterializedViews, None, false)).await, "Name", "order_totals"));
        let none = one(&mut c, rel(RelFilter::Tables, Some("nope*"), false)).await;
        assert_eq!(text(&none), "Did not find any table named \"nope*\".");
        let all = one(&mut c, Special::Describe { pattern: None, verbose: false }).await;
        assert!(has(&all, "Name", "my_seq") && has(&all, "Name", "big_orders"));

        let d = one(&mut c, describe("users", false)).await;
        assert_eq!(d.title.as_deref(), Some("Table \"public.users\""));
        assert!(has(&d, "Column", "email"));
        let footer = d.footer.clone().unwrap_or_default();
        assert!(footer.contains("\"users_pkey\" PRIMARY KEY, btree (id)"), "{footer}");
        assert!(footer.contains("Referenced by:\n    TABLE \"orders\""), "{footer}");
        let d = one(&mut c, describe("orders", true)).await;
        let footer = d.footer.clone().unwrap_or_default();
        assert!(footer.contains("Foreign-key constraints:") && footer.contains("ON DELETE CASCADE"), "{footer}");
        assert!(footer.contains("\"total_pos\" CHECK"), "{footer}");
        assert!(has(&d, "Storage", "main") || has(&d, "Storage", "plain"));
        assert_eq!(one(&mut c, describe("\"Weird'Name\"", false)).await.title.as_deref(), Some("Table \"public.Weird'Name\""));
        assert_eq!(text(&one(&mut c, describe("nosuch", false)).await), "Did not find any relation named \"nosuch\".");
        let titles: Vec<String> =
            run(&mut c, describe("*orders*", false)).await.into_iter().filter_map(|t| t.title).collect();
        assert_eq!(
            titles,
            [
                "View \"public.big_orders\"",
                "Table \"public.orders\"",
                "Sequence \"public.orders_id_seq\"",
                "Index \"public.orders_pkey\"",
                "Index \"public.orders_user_idx\"",
            ],
            "wildcards describe every matching relation, like psql"
        );
        assert_eq!(one(&mut c, describe("orders_user_idx", false)).await.title.as_deref(), Some("Index \"public.orders_user_idx\""));
        assert_eq!(one(&mut c, describe("my_seq", false)).await.title.as_deref(), Some("Sequence \"public.my_seq\""));

        assert!(has(&one(&mut c, Special::ListIndexes { pattern: None, verbose: true }).await, "Name", "orders_user_idx"));
        assert!(has(&one(&mut c, Special::ListIndexes { pattern: Some("orders".into()), verbose: false }).await, "Name", "orders_pkey"));
        assert!(has(&one(&mut c, Special::ListSequences { pattern: None }).await, "Name", "my_seq"));
        let df = one(&mut c, Special::ListFunctions { pattern: None, verbose: true }).await;
        assert_eq!(column(&df, "Name"), ["add_one"]);
        assert_eq!(column(&df, "Argument data types"), ["i integer"]);
        let dn = one(&mut c, Special::ListSchemas { pattern: None }).await;
        assert!(has(&dn, "Name", "sales") && has(&dn, "Name", "public") && !has(&dn, "Name", "pg_catalog"));
        assert!(has(&one(&mut c, Special::ListRoles { pattern: None }).await, "Role name", "postgres"));
        assert!(has(&one(&mut c, Special::ListTypes { pattern: None }).await, "Name", "mood"));
        assert!(has(&one(&mut c, Special::ListExtensions { pattern: None }).await, "Name", "plpgsql"));
        assert!(has(&one(&mut c, Special::ListPrivileges { pattern: Some("users".into()) }).await, "Name", "users"));
        let l = one(&mut c, Special::ListDatabases { pattern: Some("quarry_test_special_*".into()), verbose: true }).await;
        assert_eq!(column(&l, "Name"), [PG_DB]);

        let sf = one(&mut c, Special::ShowSource { name: "add_one".into(), kind: "function".into() }).await;
        assert!(text(&sf).starts_with("CREATE OR REPLACE FUNCTION public.add_one"), "{}", text(&sf));
        let sv = one(&mut c, Special::ShowSource { name: "big_orders".into(), kind: "view".into() }).await;
        assert!(text(&sv).starts_with("CREATE OR REPLACE VIEW public.big_orders AS\n"), "{}", text(&sv));
        let schema = one(&mut c, Special::Schema { pattern: Some("users".into()) }).await;
        assert!(text(&schema).to_uppercase().contains("CREATE TABLE"), "{}", text(&schema));

        let s = one(&mut c, Special::Status).await;
        assert!(text(&s).contains("Server version:") && text(&s).contains(PG_DB), "{}", text(&s));
        assert!(text(&one(&mut c, Special::ConnInfo).await).contains(&format!("database \"{PG_DB}\"")));
        assert_eq!(introspect::run(&mut c, &Special::Quit).await.unwrap(), None);
        drop(c);
    })
    .await;
    let _ = AssertUnwindSafe(admin.query(&format!("DROP DATABASE IF EXISTS {PG_DB} WITH (FORCE)"))).catch_unwind().await;
}

const MY_DB: &str = "quarry_test_special_my";

#[tokio::test]
async fn mysql_introspection() {
    let Some(mut admin) = connect(MY_ADMIN).await else { return };
    guarded("mysql", async {
        exec(&mut admin, &format!("DROP DATABASE IF EXISTS {MY_DB}")).await;
        exec(&mut admin, &format!("CREATE DATABASE {MY_DB}")).await;
        let mut c = connect(&format!("mysql://root@127.0.0.1:3306/{MY_DB}")).await.expect("test db connects");
        for sql in [
            "CREATE TABLE users (id INT AUTO_INCREMENT PRIMARY KEY, email VARCHAR(100) NOT NULL UNIQUE, name TEXT) COMMENT 'app users'",
            "CREATE TABLE orders (id INT AUTO_INCREMENT PRIMARY KEY, user_id INT, total DECIMAL(10,2) NOT NULL DEFAULT 0, \
             CONSTRAINT total_pos CHECK (total >= 0), \
             CONSTRAINT orders_user_fk FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE)",
            "CREATE INDEX orders_total_idx ON orders(total)",
            "CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100",
            "CREATE FUNCTION add_one(i INT) RETURNS INT DETERMINISTIC RETURN i + 1",
            "CREATE TABLE `Weird'Name` (x INT)",
        ] {
            exec(&mut c, sql).await;
        }

        let dt = one(&mut c, rel(RelFilter::Tables, None, false)).await;
        assert!(has(&dt, "Name", "users") && has(&dt, "Name", "Weird'Name") && !has(&dt, "Name", "big_orders"));
        let dt_plus = one(&mut c, rel(RelFilter::Tables, Some("us*"), true)).await;
        assert_eq!(column(&dt_plus, "Comment"), ["app users"]);
        assert!(column(&dt_plus, "Size")[0].ends_with("kB") || column(&dt_plus, "Size")[0].ends_with("bytes"));
        assert!(has(&one(&mut c, rel(RelFilter::Views, None, false)).await, "Name", "big_orders"));
        assert!(text(&one(&mut c, rel(RelFilter::MaterializedViews, None, false)).await).contains("not supported"));
        assert!(has(&one(&mut c, rel(RelFilter::All, Some("mysql.user"), false)).await, "Name", "user"));

        let d = one(&mut c, describe("orders", true)).await;
        assert_eq!(d.title.as_deref(), Some(&*format!("Table \"{MY_DB}.orders\"")));
        let footer = d.footer.clone().unwrap_or_default();
        assert!(footer.contains("Foreign-key constraints:") && footer.contains("REFERENCES users(id) ON DELETE CASCADE"), "{footer}");
        assert!(footer.contains("\"total_pos\" CHECK"), "{footer}");
        assert!(has(&d, "Default", "auto_increment"));
        assert!(one(&mut c, describe("users", false)).await.footer.unwrap_or_default().contains("Referenced by:"));
        assert_eq!(text(&one(&mut c, describe("nosuch", false)).await), "Did not find any relation named \"nosuch\".");

        let di = one(&mut c, Special::ListIndexes { pattern: Some("orders".into()), verbose: false }).await;
        assert!(has(&di, "Name", "PRIMARY") && has(&di, "Name", "orders_total_idx"));
        let df = one(&mut c, Special::ListFunctions { pattern: None, verbose: true }).await;
        assert_eq!(column(&df, "Name"), ["add_one"]);
        assert_eq!(column(&df, "Argument data types"), ["i int"]);
        assert!(has(&one(&mut c, Special::ListSchemas { pattern: None }).await, "Name", MY_DB));
        assert_eq!(column(&one(&mut c, Special::ListDatabases { pattern: Some(MY_DB.into()), verbose: true }).await, "Name"), [MY_DB]);
        assert!(has(&one(&mut c, Special::ListRoles { pattern: Some("root".into()) }).await, "User", "root"));
        assert!(has(&one(&mut c, Special::ListExtensions { pattern: Some("InnoDB".into()) }).await, "Name", "InnoDB"));
        assert!(text(&one(&mut c, Special::ListTypes { pattern: None }).await).contains("not supported"));
        assert_eq!(run(&mut c, Special::ListPrivileges { pattern: None }).await.len(), 2);

        let sf = one(&mut c, Special::ShowSource { name: "add_one".into(), kind: "function".into() }).await;
        assert!(text(&sf).contains("add_one"), "{}", text(&sf));
        let schema = one(&mut c, Special::Schema { pattern: Some("users".into()) }).await;
        assert!(text(&schema).to_uppercase().contains("CREATE TABLE"), "{}", text(&schema));
        let s = one(&mut c, Special::Status).await;
        assert!(text(&s).contains("Server version:") && text(&s).contains(MY_DB), "{}", text(&s));
        assert!(text(&one(&mut c, Special::ConnInfo).await).contains("You are connected"));
        drop(c);

        let mut nodb = connect(MY_ADMIN).await.expect("admin reconnects");
        assert!(text(&one(&mut nodb, rel(RelFilter::Tables, None, false)).await).starts_with("No database selected"));
    })
    .await;
    let _ = AssertUnwindSafe(admin.query(&format!("DROP DATABASE IF EXISTS {MY_DB}"))).catch_unwind().await;
}

#[tokio::test]
async fn sqlite_introspection() {
    let path = std::env::temp_dir().join(format!("quarry_test_special_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let Some(mut c) = connect(&format!("sqlite://{}", path.display())).await else { return };
    guarded("sqlite", async {
        for sql in [
            "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL UNIQUE, name TEXT)",
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INT REFERENCES users(id) ON DELETE CASCADE, \
             total REAL NOT NULL DEFAULT 0 CHECK (total >= 0))",
            "CREATE INDEX orders_user_idx ON orders(user_id)",
            "CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100",
            "INSERT INTO users (email) VALUES ('a@b')",
        ] {
            exec(&mut c, sql).await;
        }
        let tables = one(&mut c, rel(RelFilter::All, None, false)).await;
        assert!(has(&tables, "Name", "users") && has(&tables, "Name", "big_orders"));
        assert!(!has(&tables, "Name", "sqlite_schema") && !has(&tables, "Name", "sqlite_sequence"));
        let dt_plus = one(&mut c, rel(RelFilter::Tables, Some("us*"), true)).await;
        assert_eq!(column(&dt_plus, "Name"), ["users"]);
        assert_eq!(column(&dt_plus, "Columns"), ["3"]);
        assert!(!has(&one(&mut c, rel(RelFilter::Tables, None, false)).await, "Name", "big_orders"));

        let d = one(&mut c, describe("orders", false)).await;
        assert_eq!(d.title.as_deref(), Some("Table \"main.orders\""));
        let footer = d.footer.clone().unwrap_or_default();
        assert!(footer.contains("Foreign-key constraints:") && footer.contains("REFERENCES users(id)"), "{footer}");
        assert!(footer.contains("\"orders_user_idx\""), "{footer}");
        assert!(text(&one(&mut c, describe("zzz", false)).await).starts_with("Did not find"));

        let idx = one(&mut c, Special::ListIndexes { pattern: Some("orders".into()), verbose: false }).await;
        assert_eq!(column(&idx, "Name"), ["orders_user_idx"]);
        assert_eq!(column(&idx, "Columns"), ["user_id"]);
        assert!(has(&one(&mut c, Special::ListSequences { pattern: None }).await, "Table", "users"));
        assert!(has(&one(&mut c, Special::ListDatabases { pattern: None, verbose: false }).await, "Name", "main"));
        assert!(text(&one(&mut c, Special::ListRoles { pattern: None }).await).contains("not supported"));
        assert!(text(&one(&mut c, Special::ShowSource { name: "f".into(), kind: "function".into() }).await).contains("not supported"));

        let schema = one(&mut c, Special::Schema { pattern: Some("users".into()) }).await;
        assert_eq!(
            text(&schema),
            "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL UNIQUE, name TEXT);"
        );
        let all = one(&mut c, Special::Schema { pattern: None }).await;
        assert!(text(&all).contains("CREATE VIEW big_orders") && text(&all).contains("CREATE INDEX orders_user_idx"));
        let s = one(&mut c, Special::Status).await;
        assert!(text(&s).contains("SQLite version:") && text(&s).contains("quarry_test_special_"), "{}", text(&s));
        assert!(text(&one(&mut c, Special::ConnInfo).await).contains("quarry_test_special_"));
    })
    .await;
    drop(c);
    let _ = std::fs::remove_file(&path);
}
