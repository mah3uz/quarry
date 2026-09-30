use std::io::Write;
use std::process::{Command, Output, Stdio};

fn quarry(args: &[&str], stdin: Option<&str>) -> Output {
    let dir = std::env::temp_dir().join(format!("quarry-cli-test-{}", std::process::id()));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_quarry"));
    cmd.args(args)
        .env("QUARRY_CONFIG_DIR", dir.join("config"))
        .env("QUARRY_DATA_DIR", dir.join("data"))
        .env("NO_COLOR", "1")
        .env_remove("PAGER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn quarry");
    if let Some(input) = stdin {
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn temp_db(name: &str) -> String {
    let p = std::env::temp_dir().join(format!("quarry-cli-{}-{name}.db", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p.display().to_string()
}

#[test]
fn batch_execute_prints_tsv_with_header() {
    let o = quarry(&[":memory:", "-e", "select 1 as a, 'x' as b"], None);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(stdout(&o), "a\tb\n1\tx\n");
}

#[test]
fn format_flag_switches_output() {
    let o = quarry(&[":memory:", "-F", "csv", "-e", "select 'a,b' as v"], None);
    assert_eq!(stdout(&o), "v\n\"a,b\"\n");
    let o = quarry(&[":memory:", "-F", "json", "-e", "select 1 as n, null as z"], None);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v, serde_json::json!([{"n": 1, "z": null}]), "numbers stay numbers and NULL stays null");
}

#[test]
fn scripts_from_stdin_run_every_statement_and_persist() {
    let db = temp_db("script");
    let script = "create table t (id integer primary key, name text);\ninsert into t (name) values ('a'), ('b');\nselect count(*) as n from t;\n";
    let o = quarry(&[&db], Some(script));
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(stdout(&o).ends_with("n\n2\n"), "{}", stdout(&o));
    let o = quarry(&[&db, "-e", "select name from t order by id"], None);
    assert_eq!(stdout(&o), "name\na\nb\n");
}

#[test]
fn errors_fail_the_process_and_stop_the_script() {
    let db = temp_db("err");
    let o = quarry(&[&db], Some("create table x (a int);\nselect * from missing;\ninsert into x values (1);\n"));
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("missing"));
    let o = quarry(&[&db, "-e", "select count(*) from x"], None);
    assert!(stdout(&o).ends_with("0\n"), "statement after the error must not run: {}", stdout(&o));
}

#[test]
fn readonly_refuses_writes_but_allows_reads() {
    let db = temp_db("ro");
    quarry(&[&db, "-e", "create table t (a int)"], None);
    let o = quarry(&[&db, "--readonly", "-e", "insert into t values (1)"], None);
    assert!(!o.status.success());
    let o = quarry(&[&db, "--readonly", "-e", "select count(*) from t"], None);
    assert!(o.status.success());
}

#[test]
fn special_commands_work_in_scripts() {
    let db = temp_db("special");
    let o = quarry(&[&db], Some("create table users (id integer primary key);\n.tables\n"));
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(stdout(&o).contains("users"), "{}", stdout(&o));
}

#[test]
fn bad_target_is_a_clear_error() {
    let o = quarry(&["oracle://nope", "-e", "select 1"], None);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("unsupported scheme"));
}

// macOS included: the README documents `~/.config/quarry` and `~/.local/share/quarry`, not
// `~/Library/Application Support`.
#[cfg(unix)]
#[test]
fn config_and_data_follow_xdg_on_every_unix() {
    let home = std::env::temp_dir().join(format!("quarry-cli-xdg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let run = |xdg: Option<(&std::path::Path, &std::path::Path)>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_quarry"));
        cmd.args([":memory:", "-e", "select 1"])
            .env("HOME", &home)
            .env_remove("QUARRY_CONFIG_DIR")
            .env_remove("QUARRY_DATA_DIR")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME");
        if let Some((config, data)) = xdg {
            cmd.env("XDG_CONFIG_HOME", config).env("XDG_DATA_HOME", data);
        }
        assert!(cmd.output().unwrap().status.success());
    };
    let enable_query_log = |config: &std::path::Path| {
        let text = std::fs::read_to_string(config).unwrap();
        std::fs::write(config, text.replace("log_queries = false", "log_queries = true")).unwrap();
    };

    run(None);
    enable_query_log(&home.join(".config/quarry/config.toml"));
    run(None);
    assert!(home.join(".local/share/quarry/quarry.log").exists());

    let (config, data) = (home.join("xdg-config"), home.join("xdg-data"));
    run(Some((&config, &data)));
    enable_query_log(&config.join("quarry/config.toml"));
    run(Some((&config, &data)));
    assert!(data.join("quarry/quarry.log").exists());
    let _ = std::fs::remove_dir_all(&home);
}

fn server(env: &str, default: &str) -> Option<String> {
    let url = std::env::var(env).unwrap_or_else(|_| default.to_string());
    let o = quarry(&[&url, "-e", "select 1"], None);
    if o.status.success() {
        Some(url)
    } else {
        eprintln!("SKIP: {url} unreachable: {}", String::from_utf8_lossy(&o.stderr));
        None
    }
}

#[test]
fn postgres_batch_roundtrip() {
    let Some(url) = server("QUARRY_TEST_PG", "postgres://postgres@127.0.0.1:5432/postgres") else { return };
    let o = quarry(&[&url, "-e", "select 42::int as answer, 'x'::text as t, null::int as n"], None);
    assert_eq!(stdout(&o), "answer\tt\tn\n42\tx\tNULL\n", "TSV prints NULL like mysql -B");
}

#[test]
fn mysql_batch_roundtrip() {
    let Some(url) = server("QUARRY_TEST_MYSQL", "mysql://root@127.0.0.1:3306") else { return };
    let o = quarry(&[&url, "-F", "json", "-e", "select 42 as answer, 'x' as t"], None);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v, serde_json::json!([{"answer": 42, "t": "x"}]));
}
