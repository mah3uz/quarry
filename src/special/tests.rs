use super::*;
use crate::db::Backend::{MySql, Postgres, Sqlite};

fn ok(input: &str, b: Backend) -> Special {
    match parse(input, b) {
        Some(Ok(s)) => s,
        other => panic!("{input:?} on {b:?}: expected a special command, got {other:?}"),
    }
}

fn err(input: &str, b: Backend) -> String {
    match parse(input, b) {
        Some(Err(e)) => e,
        other => panic!("{input:?}: expected an error, got {other:?}"),
    }
}

fn sql(input: &str, b: Backend) {
    assert_eq!(parse(input, b), None, "{input:?} on {b:?} must pass through as SQL");
    assert!(!submits_immediately(input, b));
}

#[test]
fn describe_family_with_verbose_and_patterns() {
    assert_eq!(ok("\\dt", Postgres), Special::ListRelations { kind: RelFilter::Tables, pattern: None, verbose: false });
    assert_eq!(
        ok("\\dt+ public.user*", Postgres),
        Special::ListRelations { kind: RelFilter::Tables, pattern: Some("public.user*".into()), verbose: true }
    );
    assert_eq!(ok("\\dv", Postgres), Special::ListRelations { kind: RelFilter::Views, pattern: None, verbose: false });
    assert_eq!(
        ok("\\dm+", Postgres),
        Special::ListRelations { kind: RelFilter::MaterializedViews, pattern: None, verbose: true }
    );
    assert_eq!(ok("\\dE", Postgres), Special::ListRelations { kind: RelFilter::ForeignTables, pattern: None, verbose: false });
    assert_eq!(ok("\\d", Postgres), Special::Describe { pattern: None, verbose: false });
    assert_eq!(ok("\\d+ users;", Postgres), Special::Describe { pattern: Some("users".into()), verbose: true });
    assert_eq!(
        ok("\\d \"My Table\" extra", Postgres),
        Special::Describe { pattern: Some("\"My Table\"".into()), verbose: false },
        "double quotes are kept so the pattern stays case-sensitive"
    );
    assert_eq!(ok("\\dtS+", Postgres), Special::ListRelations { kind: RelFilter::Tables, pattern: None, verbose: true });
    assert_eq!(ok("\\l+", Postgres), Special::ListDatabases { pattern: None, verbose: true });
    assert_eq!(ok("\\di idx*", Postgres), Special::ListIndexes { pattern: Some("idx*".into()), verbose: false });
    assert_eq!(ok("\\ds", Postgres), Special::ListSequences { pattern: None });
    assert_eq!(ok("\\df+ f?", Postgres), Special::ListFunctions { pattern: Some("f?".into()), verbose: true });
    assert_eq!(ok("\\dn", Postgres), Special::ListSchemas { pattern: None });
    assert_eq!(ok("\\du", Postgres), Special::ListRoles { pattern: None });
    assert_eq!(ok("\\dg adm*", Postgres), Special::ListRoles { pattern: Some("adm*".into()) });
    assert_eq!(ok("\\dT", Postgres), Special::ListTypes { pattern: None });
    assert_eq!(ok("\\dx", Postgres), Special::ListExtensions { pattern: None });
    assert_eq!(ok("\\z", Postgres), Special::ListPrivileges { pattern: None });
    assert_eq!(ok("\\sf+ my_func", Postgres), Special::ShowSource { name: "my_func".into(), kind: "function".into() });
    assert_eq!(ok("\\sv v1", Postgres), Special::ShowSource { name: "v1".into(), kind: "view".into() });
    assert!(err("\\sf", Postgres).contains("function name"));
    assert_eq!(ok("\\s", MySql), Special::Status);
    assert_eq!(ok("\\conninfo", Postgres), Special::ConnInfo);
}

#[test]
fn describe_word_only_for_bare_table_names() {
    assert_eq!(ok("describe users", MySql), Special::Describe { pattern: Some("users".into()), verbose: false });
    assert_eq!(ok("DESC shop.`order`;", MySql), Special::Describe { pattern: Some("shop.`order`".into()), verbose: false });
    assert_eq!(ok("desc users", Postgres), Special::Describe { pattern: Some("users".into()), verbose: false });
    sql("DESCRIBE SELECT * FROM t", MySql);
    sql("describe select 1", MySql);
    sql("desc users id", MySql);
    sql("EXPLAIN SELECT 1", MySql);
    sql("show tables", MySql);
    sql("SHOW TABLES;", MySql);
    sql("select 1", Postgres);
    sql("descending_table_select", MySql);
}

#[test]
fn bare_words() {
    assert_eq!(ok("help", MySql), Special::Help(None));
    assert_eq!(ok("\\? \\dt", Postgres), Special::Help(Some("\\dt".into())));
    sql("HELP 'contents'", MySql);
    assert_eq!(ok("quit", Postgres), Special::Quit);
    assert_eq!(ok("exit;", MySql), Special::Quit);
    assert_eq!(ok("\\q", Sqlite), Special::Quit);
    assert_eq!(ok("use shop", MySql), Special::Connect { target: Some("shop".into()) });
    assert_eq!(ok("USE `my db`;", MySql), Special::Connect { target: Some("my db".into()) });
    assert_eq!(ok("use other", Postgres), Special::Connect { target: Some("other".into()) });
    sql("use x", Sqlite);
    assert!(err("use", MySql).contains("database"));
    assert_eq!(ok("status", MySql), Special::Status);
    assert_eq!(ok("rehash", MySql), Special::Refresh);
    assert_eq!(ok("notee", MySql), Special::NoTee);
    assert_eq!(ok("nopager", MySql), Special::NoPager);
    assert_eq!(ok("pager less -S", MySql), Special::Pager(Some("less -S".into())));
    assert_eq!(ok("delimiter //", MySql), Special::Delimiter("//".into()));
    assert!(err("delimiter", MySql).contains("delimiter"));
    assert_eq!(ok("system ls -la", MySql), Special::System { command: "ls -la".into() });
    assert_eq!(ok("prompt \\u@\\h> ", MySql), Special::Prompt(Some("\\u@\\h>".into())));
}

#[test]
fn session_toggles_validate_arguments() {
    assert_eq!(ok("\\x", Postgres), Special::Expanded(None));
    assert_eq!(ok("\\x auto", Postgres), Special::Expanded(Some(Expanded::Auto)));
    assert_eq!(ok("\\x on", Postgres), Special::Expanded(Some(Expanded::On)));
    assert!(err("\\x sideways", Postgres).contains("on, off or auto"));
    assert_eq!(ok("\\timing off", Postgres), Special::Timing(Some(false)));
    assert!(err("\\timing maybe", Postgres).contains("on or off"));
    assert_eq!(ok("\\T psql", Postgres), Special::TableFormat(Some(TableFormat::Psql)));
    assert_eq!(ok("\\T github", Postgres), Special::TableFormat(Some(TableFormat::Markdown)));
    let e = err("\\T fancy_table", Postgres);
    assert!(e.contains("unknown table format") && e.contains("rounded"), "{e}");
    assert_eq!(ok("\\readonly on", Postgres), Special::ReadOnly(Some(true)));
    assert_eq!(ok("\\theme dracula", Postgres), Special::Theme(Some("dracula".into())));
    assert_eq!(ok("\\tui", Postgres), Special::Tui);
    assert_eq!(ok("\\history 20", Postgres), Special::History(Some(20)));
    assert!(err("\\history lots", Postgres).contains("number"));
    assert_eq!(ok("\\R \\d> ", Postgres), Special::Prompt(Some("\\d>".into())));
    assert_eq!(ok("\\R '\\d> '", Postgres), Special::Prompt(Some("\\d> ".into())));
    assert_eq!(ok("\\c", Postgres), Special::Connect { target: None });
    assert_eq!(ok("\\c postgres://u@h/db", Postgres), Special::Connect { target: Some("postgres://u@h/db".into()) });
}

#[test]
fn output_redirection() {
    assert_eq!(ok("tee out.txt", MySql), Special::Tee { path: "out.txt".into(), overwrite: false });
    assert_eq!(ok("tee -o 'my file.txt'", MySql), Special::Tee { path: "my file.txt".into(), overwrite: true });
    assert!(err("tee", MySql).contains("missing file"));
    assert!(err("tee a b", MySql).contains("one file"));
    assert_eq!(ok("\\o -o res.csv", Postgres), Special::Once { path: "res.csv".into(), overwrite: true });
    assert_eq!(ok("\\once r.txt", Postgres), Special::Once { path: "r.txt".into(), overwrite: false });
    assert_eq!(ok("\\| grep foo | wc -l", Postgres), Special::PipeOnce { command: "grep foo | wc -l".into() });
    assert!(err("\\|", Postgres).contains("missing command"));
    assert_eq!(ok("\\! echo a;", Postgres), Special::System { command: "echo a;".into() }, "shell commands keep `;`");
    assert_eq!(ok("\\echo hello  world", Postgres), Special::Echo("hello  world".into()));
    let home = dirs::home_dir().unwrap();
    assert_eq!(ok("\\i ~/q.sql", Postgres), Special::Source { path: home.join("q.sql").to_string_lossy().into() });
    assert_eq!(ok("source x.sql", MySql), Special::Source { path: "x.sql".into() });
    assert!(err("\\i", Postgres).contains("missing file"));
    assert_eq!(
        ok("\\export csv \"out file.csv\" select * from t;", Postgres),
        Special::Export { format: TableFormat::Csv, path: "out file.csv".into(), query: "select * from t".into() }
    );
    assert!(err("\\export yaml f.yml select 1", Postgres).contains("unknown table format"));
    assert!(err("\\export csv f.csv", Postgres).contains("missing query"));
}

#[test]
fn query_commands_and_suffix_forms() {
    assert_eq!(ok("select 1 \\e", Postgres), Special::Edit { file: None, query: Some("select 1".into()) });
    assert_eq!(ok("\\e", Postgres), Special::Edit { file: None, query: None });
    assert_eq!(ok("\\e q.sql", Postgres), Special::Edit { file: Some("q.sql".into()), query: None });
    assert_eq!(ok("select 1 \\clip", MySql), Special::Clip { query: Some("select 1".into()) });
    assert_eq!(ok("\\clip", MySql), Special::Clip { query: None });
    assert_eq!(
        ok("select now() \\watch 5", Postgres),
        Special::Watch { seconds: 5.0, clear: false, query: Some("select now()".into()) }
    );
    assert_eq!(
        ok("select now()\n\\watch", Postgres),
        Special::Watch { seconds: 2.0, clear: false, query: Some("select now()".into()) }
    );
    assert!(err("select 1 \\watch -1", Postgres).contains("positive"));
    assert_eq!(
        ok("\\watch 0.5 -c select count(*) from t;", Postgres),
        Special::Watch { seconds: 0.5, clear: true, query: Some("select count(*) from t".into()) }
    );
    assert_eq!(ok("\\watch i=3", Postgres), Special::Watch { seconds: 3.0, clear: false, query: None });
    assert_eq!(
        ok("watch 10 select 1", MySql),
        Special::Watch { seconds: 10.0, clear: false, query: Some("select 1".into()) }
    );
    assert!(err("\\watch 0", Postgres).contains("positive"));
    sql("select '\\e'", Postgres);
    sql("select 1 \\G", MySql);
    sql("select 1 \\x", Postgres);
    assert_eq!(ok("\\explain analyze select 1", Postgres), Special::Explain { analyze: true, query: "select 1".into() });
    assert_eq!(ok("\\explain select 1;", Postgres), Special::Explain { analyze: false, query: "select 1".into() });
    assert!(err("\\explain", Postgres).contains("missing query"));
    assert!(err("\\explain analyze", Postgres).contains("missing query"));
    assert_eq!(ok("\\format select 1", Postgres), Special::Format { query: Some("select 1".into()) });
}

#[test]
fn favorites_commands() {
    assert_eq!(ok("\\f", Postgres), Special::Favorite { name: None, args: vec![] });
    assert_eq!(
        ok("\\f top 10 'a b' --user=bob", Postgres),
        Special::Favorite { name: Some("top".into()), args: vec!["10".into(), "a b".into(), "--user=bob".into()] }
    );
    assert_eq!(ok("\\n q", Postgres), Special::Favorite { name: Some("q".into()), args: vec![] });
    assert_eq!(
        ok("\\fs top select * from t limit $1;", Postgres),
        Special::FavoriteSave { name: "top".into(), query: "select * from t limit $1".into() }
    );
    assert!(err("\\fs top", Postgres).contains("missing query"));
    assert!(err("\\fs", Postgres).contains("missing name"));
    assert_eq!(ok("\\fd top", Postgres), Special::FavoriteDelete { name: "top".into() });
    assert_eq!(ok("\\nd top", Postgres), Special::FavoriteDelete { name: "top".into() });
    assert!(err("\\fd", Postgres).contains("missing name"));
    assert!(err("\\f 'unterminated", Postgres).contains("cannot parse"));
}

#[test]
fn sqlite_dot_commands_only_on_sqlite() {
    assert_eq!(ok(".tables", Sqlite), Special::ListRelations { kind: RelFilter::All, pattern: None, verbose: false });
    assert_eq!(ok(".tables us%", Sqlite), Special::ListRelations { kind: RelFilter::All, pattern: Some("us%".into()), verbose: false });
    assert_eq!(ok(".views", Sqlite), Special::ListRelations { kind: RelFilter::Views, pattern: None, verbose: false });
    assert_eq!(ok(".schema t", Sqlite), Special::Schema { pattern: Some("t".into()) });
    assert_eq!(ok(".indexes t", Sqlite), Special::ListIndexes { pattern: Some("t".into()), verbose: false });
    assert_eq!(ok(".databases", Sqlite), Special::ListDatabases { pattern: None, verbose: false });
    assert_eq!(ok(".open other.db", Sqlite), Special::Connect { target: Some("other.db".into()) });
    assert_eq!(ok(".mode line", Sqlite), Special::TableFormat(Some(TableFormat::Vertical)));
    assert_eq!(ok(".mode csv", Sqlite), Special::TableFormat(Some(TableFormat::Csv)));
    assert_eq!(ok(".read a.sql", Sqlite), Special::Source { path: "a.sql".into() });
    assert_eq!(ok(".output o.txt", Sqlite), Special::Tee { path: "o.txt".into(), overwrite: true });
    assert_eq!(ok(".output stdout", Sqlite), Special::NoTee);
    assert_eq!(ok(".once o.txt", Sqlite), Special::Once { path: "o.txt".into(), overwrite: true });
    assert_eq!(ok(".load ./ext.so", Sqlite), Special::LoadExtension { path: "./ext.so".into() });
    assert_eq!(ok("\\load ./ext.so", Sqlite), Special::LoadExtension { path: "./ext.so".into() });
    assert_eq!(ok(".status", Sqlite), Special::Status);
    assert_eq!(ok(".exit", Sqlite), Special::Quit);
    assert!(err(".bogus", Sqlite).contains("unknown command .bogus"));
    assert!(err(".open", Sqlite).contains("missing file"));
    sql(".tables", Postgres);
    sql(".5 + 1", MySql);
}

#[test]
fn unknown_backslash_commands_are_errors_not_sql() {
    assert!(err("\\bogus", Postgres).contains("unknown command \\bogus"));
    assert!(err("\\load x", Postgres).contains("unknown command"));
    assert_eq!(parse("\\G", MySql), None, "\\G alone is a terminator for the splitter");
    assert_eq!(parse("   ", MySql), None);
}

#[test]
fn submits_immediately_for_specials_only() {
    assert!(submits_immediately("\\dt", Postgres));
    assert!(submits_immediately("use shop", MySql));
    assert!(submits_immediately("select 1 \\e", Postgres));
    assert!(submits_immediately("\\x bogus", Postgres), "errors are shown right away too");
    assert!(!submits_immediately("select 1", Postgres));
    assert!(!submits_immediately("select *\nfrom t", Postgres));
}

#[test]
fn registry_covers_every_parsed_command_name() {
    let names: Vec<&str> = registry().iter().flat_map(|c| c.names.iter().copied()).collect();
    let mut seen = std::collections::HashSet::new();
    for n in &names {
        assert!(seen.insert(*n), "duplicate command name {n}");
    }
    for c in registry() {
        let b = c.backends.first().copied().unwrap_or(Postgres);
        let first = c.names[0];
        if first.starts_with('\\') || first.starts_with('.') {
            let parsed = parse(first, if first.starts_with('.') { Sqlite } else { b });
            assert!(parsed.is_some(), "registry name {first} is not recognised by parse()");
            if let Some(Err(e)) = &parsed {
                assert!(!e.contains("unknown command"), "{first}: {e}");
            }
        }
        assert!(!c.description.is_empty() && !c.syntax.is_empty());
    }
}

#[test]
fn help_lists_backend_commands_and_topics() {
    let pg = help_text(None, Postgres);
    assert!(pg.contains("\\dT [pattern]") && pg.contains("Informational"));
    assert!(!pg.contains(".load"), "sqlite-only commands are hidden on postgres");
    assert!(help_text(None, Sqlite).contains(".load path"));
    let t = help_text(Some("dt+"), Postgres);
    assert!(t.starts_with("\\dt[+] [pattern]\n") && t.contains(".tables"), "{t}");
    assert!(help_text(Some("\\nope"), Postgres).contains("No help"));
    assert_eq!(lookup("desc").map(|c| c.names[0]), Some("\\d"));
}
