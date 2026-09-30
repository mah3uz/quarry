use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::db::catalog::{ColumnInfo, ForeignKey, FunctionInfo, FunctionKind, RelKind, Relation, SchemaInfo};

fn rel(schema: &str, name: &str, kind: RelKind, cols: &[(&str, &str)]) -> Relation {
    Relation {
        schema: schema.into(),
        name: name.into(),
        kind,
        columns: cols
            .iter()
            .map(|(n, t)| ColumnInfo {
                name: (*n).into(),
                data_type: (*t).into(),
                nullable: true,
                default: None,
                primary_key: *n == "id",
                auto: false,
                comment: None,
            })
            .collect(),
        comment: None,
        row_estimate: None,
    }
}

fn fk(table: &str, cols: &[&str], ref_table: &str, ref_cols: &[&str]) -> ForeignKey {
    ForeignKey {
        name: format!("{table}_fk"),
        schema: "public".into(),
        table: table.into(),
        columns: cols.iter().map(|s| s.to_string()).collect(),
        ref_schema: "public".into(),
        ref_table: ref_table.into(),
        ref_columns: ref_cols.iter().map(|s| s.to_string()).collect(),
        on_update: None,
        on_delete: None,
    }
}

fn func(schema: &str, name: &str, args: &str, ret: &str) -> FunctionInfo {
    FunctionInfo { schema: schema.into(), name: name.into(), args: args.into(), return_type: ret.into(), kind: FunctionKind::Function }
}

/// users ← orders ← order_items → products, plus a view, a mixed-case table and a second schema.
fn shop(backend: Backend) -> Catalog {
    let t = RelKind::Table;
    let public = SchemaInfo {
        name: "public".into(),
        relations: vec![
            rel("public", "users", t, &[("id", "integer"), ("name", "text"), ("email", "text")]),
            rel("public", "orders", t, &[("id", "integer"), ("user_id", "integer"), ("total", "numeric"), ("created_at", "timestamptz")]),
            rel("public", "order_items", t, &[("id", "integer"), ("order_id", "integer"), ("product_id", "integer"), ("qty", "integer")]),
            rel("public", "products", t, &[("id", "integer"), ("name", "text"), ("price", "numeric")]),
            rel("public", "active_users", RelKind::View, &[("id", "integer"), ("name", "text")]),
            rel("public", "MyTable", t, &[("CamelCol", "text"), ("plain", "text")]),
        ],
        functions: vec![func("public", "calc_total", "order_id integer", "numeric")],
        types: vec!["mood".into()],
    };
    let sales = SchemaInfo {
        name: "sales".into(),
        relations: vec![rel("sales", "invoices", t, &[("id", "integer"), ("amount", "numeric")])],
        functions: vec![func("sales", "report", "", "void")],
        types: vec![],
    };
    Catalog {
        backend,
        databases: vec!["app".into(), "analytics".into()],
        current_database: Some("app".into()),
        search_path: vec!["public".into()],
        schemas: vec![public, sales],
        foreign_keys: vec![
            fk("orders", &["user_id"], "users", &["id"]),
            fk("order_items", &["order_id"], "orders", &["id"]),
            fk("order_items", &["product_id"], "products", &["id"]),
        ],
        users: vec!["alice".into(), "bob".into()],
    }
}

fn extras() -> Extras {
    Extras {
        specials: vec![
            ("\\d".into(), "Describe".into()),
            ("\\dt".into(), "List tables".into()),
            ("\\df".into(), "List functions".into()),
            ("\\c".into(), "Connect".into()),
            ("\\x".into(), "Expanded output".into()),
            (".tables".into(), "List tables".into()),
            (".mode".into(), "Output mode".into()),
            ("source".into(), "Run a file".into()),
        ],
        favorites: vec!["daily_report".into(), "top_users".into()],
        table_formats: vec!["psql".into(), "csv".into(), "json".into()],
        themes: vec!["nord".into(), "dracula".into()],
    }
}

fn completer_with(backend: Backend, options: CompleteOptions) -> Completer {
    Completer::new(backend, Arc::new(shop(backend)), options, extras())
}

fn completer(backend: Backend) -> Completer {
    completer_with(backend, CompleteOptions::default())
}

/// `input` contains `‸` marking the cursor.
fn run(c: &Completer, input: &str) -> Completions {
    let cursor = input.find('‸').expect("cursor marker");
    let text = input.replace('‸', "");
    c.complete(&text, cursor)
}

fn texts(c: &Completions) -> Vec<&str> {
    c.items.iter().map(|s| s.text.as_str()).collect()
}

fn pg(input: &str) -> Completions {
    run(&completer(Backend::Postgres), input)
}

fn pos(c: &Completions, text: &str) -> usize {
    texts(c).iter().position(|t| *t == text).unwrap_or_else(|| panic!("{text:?} not in {:?}", texts(c)))
}

#[test]
fn columns_are_offered_before_from_is_typed_after_cursor() {
    let c = pg("SELECT ‸ FROM users");
    let t = texts(&c);
    assert_eq!(&t[..3], &["id", "name", "email"], "columns of the FROM table come first: {t:?}");
    assert!(!t.contains(&"total"), "columns of tables not in the statement are excluded");
    let rank = |k: SuggestionKind| match k {
        SuggestionKind::Column => 0,
        SuggestionKind::Alias => 1,
        SuggestionKind::Function => 2,
        _ => 3,
    };
    let kinds: Vec<_> = c.items.iter().map(|s| rank(s.kind)).collect();
    assert!(kinds.windows(2).all(|w| w[0] <= w[1]), "columns, aliases, functions, then keywords");
    assert!(t.contains(&"calc_total"));
    assert_eq!(c.items[0].detail.as_deref(), Some("integer"), "column detail is its data type");
    let c = pg("SELECT dis‸ FROM users");
    assert_eq!(c.items[0].text, "distinct", "expression keywords outrank generic ones like DISABLE");
}

#[test]
fn colliding_columns_are_qualified_with_alias() {
    let c = pg("SELECT * FROM users u JOIN orders o ON o.user_id = u.id WHERE ‸");
    let t = texts(&c);
    assert!(t.contains(&"u.id") && t.contains(&"o.id"), "{t:?}");
    assert!(t.contains(&"email") && t.contains(&"user_id"), "unique names stay bare: {t:?}");
    assert!(!t.contains(&"id"));
    assert!(t.contains(&"u") && t.contains(&"o"), "aliases are suggested");
}

#[test]
fn join_using_offers_bare_shared_columns_first() {
    let c = pg("SELECT * FROM users JOIN products USING (‸");
    let t = texts(&c);
    assert_eq!(&t[..2], &["id", "name"], "names in both tables first: {t:?}");
    assert!(t.iter().all(|s| !s.contains('.')), "USING takes bare names: {t:?}");
}

#[test]
fn alias_dot_limits_to_that_table_and_replaces_after_dot() {
    let input = "SELECT u.na‸ FROM users u JOIN orders o ON true";
    let c = pg(input);
    assert_eq!(texts(&c), vec!["name"]);
    assert_eq!(c.replace_start, input.find("na").unwrap());

    let c = pg("SELECT o.‸ FROM users u JOIN orders o ON true");
    assert_eq!(texts(&c), vec!["id", "total", "user_id", "created_at"]);
}

#[test]
fn schema_dot_lists_that_schemas_objects() {
    let c = pg("SELECT * FROM sales.‸");
    assert_eq!(texts(&c), vec!["invoices"]);

    let c = pg("SELECT sales.‸");
    let t = texts(&c);
    assert!(t.contains(&"invoices") && t.contains(&"report"), "{t:?}");
    assert!(!t.contains(&"users"));
}

#[test]
fn schema_qualified_table_resolves_columns() {
    let c = pg("SELECT i.‸ FROM sales.invoices i");
    assert_eq!(texts(&c), vec!["id", "amount"]);
}

#[test]
fn join_suggestions_follow_foreign_keys() {
    let c = pg("SELECT * FROM users u JOIN ‸");
    assert_eq!(c.items[0].kind, SuggestionKind::Join);
    assert_eq!(c.items[0].text, "orders ON orders.user_id = u.id");
    assert!(texts(&c).contains(&"products"), "plain tables are still offered");

    let c = pg("SELECT * FROM orders o JOIN ‸");
    let joins: Vec<_> = c.items.iter().filter(|s| s.kind == SuggestionKind::Join).map(|s| s.text.as_str()).collect();
    assert!(joins.contains(&"users ON users.id = o.user_id"), "child → parent: {joins:?}");
    assert!(joins.contains(&"order_items ON order_items.order_id = o.id"), "parent → child: {joins:?}");
    assert!(!joins.iter().any(|j| j.starts_with("products")), "no FK between orders and products");
}

#[test]
fn join_suggestions_generate_aliases_and_respect_option() {
    let opts = CompleteOptions { generate_aliases: true, ..CompleteOptions::default() };
    let c = run(&completer_with(Backend::Postgres, opts), "SELECT * FROM users u JOIN ‸");
    assert_eq!(c.items[0].text, "orders o ON o.user_id = u.id");

    let opts = CompleteOptions { join_suggestions: false, ..CompleteOptions::default() };
    let c = run(&completer_with(Backend::Postgres, opts), "SELECT * FROM users u JOIN ‸");
    assert!(c.items.iter().all(|s| s.kind != SuggestionKind::Join));
    assert!(texts(&c).contains(&"orders"));
}

#[test]
fn self_join_gets_a_distinct_alias() {
    let c = pg("SELECT * FROM orders JOIN users ON true JOIN ‸");
    let t = texts(&c);
    assert!(t.contains(&"orders o ON o.user_id = users.id"), "orders already referenced → aliased: {t:?}");
}

#[test]
fn join_conditions_come_first_after_on() {
    let c = pg("SELECT * FROM users u JOIN orders o ON ‸");
    assert_eq!(c.items[0].kind, SuggestionKind::JoinCondition);
    assert_eq!(c.items[0].text, "o.user_id = u.id");
    assert!(texts(&c).contains(&"total"), "columns of joined tables follow");

    let c = pg("SELECT * FROM orders o JOIN order_items oi ON ‸ WHERE true");
    assert_eq!(c.items[0].text, "oi.order_id = o.id");
}

#[test]
fn insert_column_list_offers_only_target_columns() {
    let c = pg("INSERT INTO orders (‸");
    assert_eq!(texts(&c), vec!["id", "total", "user_id", "created_at"]);
    let c = pg("INSERT INTO orders (id, ‸) VALUES (1, 2)");
    assert_eq!(texts(&c), vec!["id", "total", "user_id", "created_at"]);
}

#[test]
fn insert_into_offers_tables() {
    let c = pg("INSERT INTO ‸");
    assert_eq!(c.items[0].text, "users");
    assert!(c.items.iter().all(|s| matches!(s.kind, SuggestionKind::Table | SuggestionKind::View | SuggestionKind::Schema)));
}

#[test]
fn update_set_offers_target_columns() {
    let c = pg("UPDATE users SET ‸");
    assert_eq!(texts(&c), vec!["id", "name", "email"]);
    let c = pg("UPDATE users SET name = 'x', ‸ WHERE id = 1");
    assert_eq!(texts(&c), vec!["id", "name", "email"]);
}

#[test]
fn alter_table_column_actions_offer_its_columns() {
    let c = pg("ALTER TABLE products DROP COLUMN ‸");
    assert_eq!(texts(&c), vec!["id", "name", "price"]);
    let c = pg("ALTER TABLE products RENAME COLUMN pr‸");
    assert_eq!(texts(&c), vec!["price"]);
}

#[test]
fn datatypes_after_cast_operator_and_in_column_definitions() {
    for input in ["SELECT created_at::‸ FROM orders", "SELECT CAST(total AS ‸) FROM orders", "CREATE TABLE t (id ‸"] {
        let c = pg(input);
        let t = texts(&c);
        assert!(t.contains(&"TIMESTAMPTZ") && t.contains(&"INTEGER"), "{input}: {t:?}");
        assert!(t.contains(&"mood"), "user-defined types: {input}");
        assert!(c.items.iter().all(|s| s.kind == SuggestionKind::DataType), "{input}: only types");
    }
    let c = pg("CREATE TABLE t (id integer, created ‸");
    assert!(texts(&c).contains(&"TIMESTAMPTZ"));
}

#[test]
fn nothing_inside_strings_or_comments() {
    for input in [
        "SELECT 'us‸' FROM users",
        "SELECT 'unterminated us‸",
        "SELECT 1 -- us‸",
        "SELECT /* us‸ */ 1",
        "SELECT \"ok\" FROM t WHERE x = $$ us‸ $$",
    ] {
        assert!(pg(input).items.is_empty(), "{input}");
    }
    assert!(!pg("SELECT 'a' ‸").items.is_empty(), "right after a closed string is fine");
}

#[test]
fn databases_after_use_and_connect() {
    let my = completer(Backend::MySql);
    assert_eq!(texts(&run(&my, "USE ‸")), vec!["app", "analytics"]);
    assert_eq!(texts(&run(&my, "use an‸")), vec!["analytics"]);
    assert_eq!(texts(&pg("\\c ‸")), vec!["app", "analytics"]);
    assert_eq!(texts(&pg("DROP DATABASE ‸")), vec!["app", "analytics"]);
}

#[test]
fn special_commands_complete_by_name_and_argument() {
    let input = "\\d‸";
    let c = pg(input);
    assert_eq!(c.replace_start, 0);
    assert_eq!(texts(&c), vec!["\\d", "\\df", "\\dt"]);
    assert_eq!(c.items[0].kind, SuggestionKind::Special);
    assert_eq!(c.items[0].detail.as_deref(), Some("Describe"));

    let c = pg("\\dt us‸");
    assert_eq!(texts(&c)[0], "users");
    assert!(c.items.iter().all(|s| s.kind != SuggestionKind::Keyword));

    assert_eq!(texts(&pg("\\df calc‸")), vec!["calc_total"]);
    assert_eq!(texts(&pg("\\f ‸")), vec!["top_users", "daily_report"]);
    assert_eq!(texts(&pg("\\T ‸")), vec!["csv", "json", "psql"]);
    assert_eq!(texts(&pg("\\theme d‸"))[0], "dracula");
}

#[test]
fn sqlite_dot_commands() {
    let lite = completer(Backend::Sqlite);
    let c = run(&lite, ".ta‸");
    assert_eq!(texts(&c), vec![".tables"]);
    assert_eq!(c.replace_start, 0);
    assert_eq!(texts(&run(&lite, ".mode j‸")), vec!["json"]);
    assert_eq!(texts(&run(&lite, ".schema ord‸"))[0], "orders");
    assert!(run(&completer(Backend::Postgres), ".ta‸").items.iter().all(|s| s.kind != SuggestionKind::Special));
}

#[test]
fn statement_start_offers_starting_keywords() {
    let c = pg("‸");
    let t = texts(&c);
    for kw in ["SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "CREATE", "ALTER", "DROP", "EXPLAIN", "BEGIN"] {
        assert!(t.contains(&kw), "{kw} missing: {t:?}");
    }
    assert!(!t.contains(&"WHERE"), "non-starting keywords are not offered");
    assert!(!t.contains(&"users"));

    let my = completer(Backend::MySql);
    assert!(texts(&run(&my, "sou‸")).contains(&"source"));
    assert!(texts(&run(&my, "HE‸")).contains(&"HELP"));
    assert!(texts(&run(&my, "us‸")).contains(&"use"));
}

#[test]
fn keyword_casing_auto_matches_typed_case() {
    assert_eq!(texts(&pg("sel‸"))[0], "select");
    assert_eq!(texts(&pg("SEL‸"))[0], "SELECT");
    assert_eq!(texts(&pg("seL‸"))[0], "SELECT");
    assert!(texts(&pg("‸")).contains(&"SELECT"), "nothing typed → upper");
    let lower = CompleteOptions { keyword_casing: KeywordCasing::Lower, ..CompleteOptions::default() };
    assert_eq!(texts(&run(&completer_with(Backend::Postgres, lower), "SEL‸"))[0], "select");
}

#[test]
fn fuzzy_segment_prefix_ranks_user_id_first() {
    let c = pg("SELECT ui‸ FROM orders");
    assert_eq!(c.items[0].text, "user_id");
    let c = pg("SELECT * FROM oi‸");
    assert_eq!(c.items[0].text, "order_items");
}

#[test]
fn ranking_prefers_prefix_then_context_kind() {
    let c = pg("SELECT * FROM users WHERE na‸");
    assert_eq!(c.items[0].text, "name", "column beats keywords like NATURAL");
    let c = pg("SELECT * FROM us‸");
    assert_eq!(c.items[0].text, "users");
    assert!(pos(&c, "users") < pos(&c, "active_users"), "prefix beats substring");
    assert_eq!(texts(&pg("‸"))[..4], ["SELECT", "INSERT", "UPDATE", "DELETE"], "statement start in curated order");
}

#[test]
fn mixed_case_identifiers_are_quoted_for_postgres() {
    let c = pg("SELECT * FROM My‸");
    assert_eq!(c.items[0].text, "\"MyTable\"");

    let input = "SELECT * FROM \"MyT‸";
    let c = pg(input);
    assert_eq!(c.replace_start, input.find('"').unwrap());
    assert_eq!(c.items[0].text, "\"MyTable\"");

    assert!(pg("SELECT \"fa‸").items.is_empty(), "a quoted identifier never completes to keywords like FALSE");

    let c = pg("SELECT ‸ FROM \"MyTable\"");
    assert_eq!(texts(&c)[..2], ["plain", "\"CamelCol\""]);

    let my = completer(Backend::MySql);
    assert_eq!(run(&my, "SELECT * FROM My‸").items[0].text, "MyTable", "mysql needs no quotes");
}

#[test]
fn only_the_statement_under_the_cursor_counts() {
    let c = pg("SELECT * FROM orders; SELECT ‸ FROM users; SELECT * FROM products");
    let t = texts(&c);
    assert_eq!(&t[..3], &["id", "name", "email"]);
    assert!(!t.contains(&"total") && !t.contains(&"price"));

    let c = pg("SELECT * FROM users;‸");
    assert!(texts(&c).contains(&"SELECT"));
    assert!(!texts(&c).contains(&"email"));

    let c = pg("SELECT ‸ FROM users\\G SELECT * FROM orders");
    assert!(!texts(&c).contains(&"total"));
}

#[test]
fn cte_names_and_columns_are_in_scope() {
    let c = pg("WITH recent AS (SELECT id, total AS amount FROM orders) SELECT * FROM ‸");
    assert_eq!(c.items[0].text, "recent");
    assert_eq!(c.items[0].detail.as_deref(), Some("cte"));

    let c = pg("WITH recent AS (SELECT id, total AS amount FROM orders) SELECT ‸ FROM recent");
    assert_eq!(&texts(&c)[..2], &["id", "amount"]);

    let c = pg("WITH r (a, b) AS (SELECT 1, 2), s AS (SELECT * FROM users) SELECT ‸ FROM r, s");
    let t = texts(&c);
    assert!(t.contains(&"a") && t.contains(&"b") && t.contains(&"email"), "{t:?}");
}

#[test]
fn derived_table_alias_exposes_its_select_list() {
    let c = pg("SELECT s.‸ FROM (SELECT id, total, count(*) n FROM orders GROUP BY 1) s");
    assert_eq!(texts(&c), vec!["n", "id", "total"], "shorter first at equal quality");
}

#[test]
fn subquery_sees_its_own_and_outer_tables() {
    let c = pg("SELECT * FROM users WHERE id IN (SELECT ‸ FROM orders)");
    let t = texts(&c);
    assert!(t.contains(&"total") && t.contains(&"email"), "{t:?}");
    let c = pg("SELECT ‸ FROM users WHERE id IN (SELECT user_id FROM orders)");
    assert!(!texts(&c).contains(&"total"), "inner tables do not leak out");
}

#[test]
fn select_star_expands_to_column_list() {
    let input = "SELECT *‸ FROM users";
    let c = pg(input);
    assert_eq!(texts(&c), vec!["id, name, email"]);
    assert_eq!(c.replace_start, input.find('*').unwrap());

    let c = pg("SELECT *‸ FROM users u JOIN orders o ON o.user_id = u.id");
    assert_eq!(texts(&c), vec!["u.id, u.name, u.email, o.id, o.user_id, o.total, o.created_at"]);

    let input = "SELECT u.*‸ FROM users u JOIN orders o ON true";
    let c = pg(input);
    assert_eq!(texts(&c), vec!["u.id, u.name, u.email"]);
    assert_eq!(c.replace_start, input.find("u.*").unwrap());

    assert!(pg("SELECT 2 *‸ FROM users").items.is_empty(), "multiplication is not a star");
}

#[test]
fn users_after_grant_to() {
    assert_eq!(texts(&pg("GRANT SELECT ON users TO ‸")), vec!["bob", "alice"]);
    assert_eq!(texts(&pg("ALTER TABLE users OWNER TO b‸")), vec!["bob"]);
}

#[test]
fn functions_after_function_keywords() {
    let c = pg("DROP FUNCTION calc‸");
    assert_eq!(texts(&c), vec!["calc_total"]);
    let c = pg("SELECT calc‸ FROM orders");
    let f = &c.items[0];
    assert_eq!((f.text.as_str(), f.display.as_str()), ("calc_total", "calc_total(order_id integer)"));
    assert_eq!(f.detail.as_deref(), Some("numeric"));
    assert_eq!(f.kind, SuggestionKind::Function);
}

#[test]
fn views_have_view_kind_and_tables_rank_after_from() {
    let c = pg("SELECT * FROM ‸");
    let v = c.items.iter().find(|s| s.text == "active_users").unwrap();
    assert_eq!(v.kind, SuggestionKind::View);
    assert_eq!(v.detail.as_deref(), Some("view"));
    assert!(pos(&c, "users") < pos(&c, "sales"), "tables before schemas");
    assert!(!texts(&c).contains(&"invoices"), "off-search-path tables need a qualifier");
}

#[test]
fn file_paths_complete_with_trailing_slash_for_dirs() {
    let dir = std::env::temp_dir().join(format!("quarry-complete-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::write(dir.join("schema.sql"), "").unwrap();
    std::fs::write(dir.join("other.txt"), "").unwrap();
    let base = format!("{}/", dir.display());
    let input = format!("\\i {base}s‸");
    let c = pg(&input);
    let mut t = texts(&c);
    t.sort();
    assert_eq!(t, vec![format!("{base}schema.sql"), format!("{base}scripts/")]);
    assert_eq!(c.replace_start, 3);
    assert!(c.items.iter().all(|s| s.kind == SuggestionKind::File));
    let lite = completer(Backend::Sqlite);
    assert_eq!(run(&lite, &format!(".read {base}oth‸")).items[0].text, format!("{base}other.txt"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn plain_mode_skips_context_analysis() {
    let opts = CompleteOptions { smart: false, ..CompleteOptions::default() };
    let c = run(&completer_with(Backend::Postgres, opts), "SELECT * FROM users WHERE tot‸");
    assert!(texts(&c).contains(&"total"), "columns of unrelated tables are offered");
    let c = run(&completer_with(Backend::Postgres, CompleteOptions::default()), "SELECT * FROM users WHERE tot‸");
    assert!(!texts(&c).contains(&"total"), "smart mode limits to tables in scope");
}

#[test]
fn results_are_deduplicated_and_capped() {
    let c = pg("SELECT ‸ FROM users u JOIN products p ON true");
    let t = texts(&c);
    let mut uniq = t.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), t.len(), "no duplicates");
    let opts = CompleteOptions { max_items: 5, ..CompleteOptions::default() };
    assert_eq!(run(&completer_with(Backend::Postgres, opts), "SELECT ‸").items.len(), 5);
}

#[test]
fn clause_keywords_follow_expressions() {
    assert_eq!(texts(&pg("SELECT * FROM users ORDER ‸")), vec!["BY"]);
    let c = pg("SELECT * FROM users ORDER BY ‸");
    assert_eq!(&texts(&c)[..3], &["id", "name", "email"]);
    let c = pg("SELECT * FROM users u wh‸");
    assert_eq!(texts(&c)[0], "where", "WHERE beats the equally short WHEN after a table");
    assert_eq!(texts(&pg("SELECT * ‸"))[0], "FROM", "curated follow-ups rank in list order");
    assert_eq!(texts(&pg("INSERT INTO orders (id) ‸"))[0], "VALUES");
    assert_eq!(texts(&pg("ALTER TABLE users ‸"))[0], "ADD");
    assert_eq!(texts(&pg("SELECT * FROM users WHERE id = 1 ‸"))[0], "AND");
    assert_eq!(texts(&pg("SELECT * FROM users ORDER BY id ‸"))[0], "ASC");
}

#[test]
fn catalog_swap_rebuilds_the_index() {
    let mut c = completer(Backend::Postgres);
    assert!(texts(&run(&c, "SELECT * FROM us‸")).contains(&"users"));
    let mut cat = shop(Backend::Postgres);
    cat.schemas[0].relations.retain(|r| r.name != "users");
    c.catalog = Arc::new(cat);
    assert!(!texts(&run(&c, "SELECT * FROM us‸")).contains(&"users"));
}

fn big_catalog(tables: usize, cols: usize) -> Catalog {
    let colnames: Vec<(String, &str)> = (0..cols).map(|i| (format!("column_{i}"), "integer")).collect();
    let relations = (0..tables)
        .map(|i| {
            let cols: Vec<(&str, &str)> = colnames.iter().map(|(n, t)| (n.as_str(), *t)).collect();
            rel("public", &format!("table_{i:05}"), RelKind::Table, &cols)
        })
        .collect();
    let mut cat = Catalog::empty(Backend::Postgres);
    cat.search_path = vec!["public".into()];
    cat.schemas = vec![SchemaInfo { name: "public".into(), relations, functions: vec![], types: vec![] }];
    cat
}

#[test]
fn completion_is_fast_on_large_catalogs() {
    let c = Completer::new(Backend::Postgres, Arc::new(big_catalog(5_000, 10)), CompleteOptions::default(), Extras::default());
    run(&c, "SELECT * FROM ‸");
    let budget = Duration::from_millis(50);
    for input in [
        "SELECT * FROM ‸",
        "SELECT * FROM tab‸",
        "SELECT * FROM t9‸",
        "SELECT ‸ FROM table_00001 a JOIN table_00002 b ON true",
        "SELECT * FROM table_00001 WHERE col‸",
    ] {
        let t = Instant::now();
        let r = run(&c, input);
        let took = t.elapsed();
        assert!(!r.items.is_empty(), "{input}");
        assert!(took < budget, "{input} took {took:?}");
    }
}

#[test]
fn cursor_inside_a_word_matches_only_the_typed_part() {
    let input = "SELECT * FROM ord‸ers";
    let c = pg(input);
    assert_eq!(c.replace_start, input.find("ord").unwrap());
    assert_eq!(&texts(&c)[..2], &["orders", "order_items"]);
}

#[test]
fn table_name_works_as_qualifier_without_alias() {
    assert_eq!(texts(&pg("SELECT * FROM users WHERE users.em‸")), vec!["email"]);
    assert_eq!(texts(&pg("SELECT products.‸")), vec!["id", "name", "price"], "catalog table outside the statement");
}

#[test]
fn out_of_range_or_mid_char_cursor_is_clamped() {
    let c = completer(Backend::Postgres);
    let text = "SELECT naïve";
    assert!(c.complete(text, 1_000).replace_start <= text.len());
    let mid = text.find('ï').unwrap() + 1;
    let r = c.complete(text, mid);
    assert!(text.is_char_boundary(r.replace_start));
}
