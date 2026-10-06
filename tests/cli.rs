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

/// Like `quarry`, but with its own config and data dirs so tests can write config without colliding.
fn quarry_in(dir: &std::path::Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_quarry"))
        .args(args)
        .env("QUARRY_CONFIG_DIR", dir.join("config"))
        .env("QUARRY_DATA_DIR", dir.join("data"))
        .env("NO_COLOR", "1")
        .env_remove("PAGER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn quarry");
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("quarry-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config")).unwrap();
    dir
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

/// A throwaway sshd on a loopback port, and an `ssh` on PATH that trusts only its host key.
#[cfg(unix)]
struct TestSshd {
    child: std::process::Child,
    dir: std::path::PathBuf,
    port: u16,
}

#[cfg(unix)]
impl TestSshd {
    fn start(name: &str) -> Option<TestSshd> {
        let tool = |t: &str| which::which(t).map_err(|_| eprintln!("SKIP: {t} not installed")).ok();
        let (sshd, ssh, keygen) = (tool("sshd")?, tool("ssh")?, tool("ssh-keygen")?);
        let dir = test_dir(name);
        let path = |f: &str| dir.join(f).display().to_string();
        for key in ["host_key", "client_key"] {
            let made = Command::new(&keygen).args(["-q", "-t", "ed25519", "-N", "", "-f", &path(key)]).status().unwrap();
            assert!(made.success());
        }
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let host_key = std::fs::read_to_string(dir.join("host_key.pub")).unwrap();
        let host_key: Vec<&str> = host_key.split_whitespace().take(2).collect();
        std::fs::write(dir.join("known_hosts"), format!("[127.0.0.1]:{port} {}\n", host_key.join(" "))).unwrap();
        std::fs::write(
            dir.join("sshd_config"),
            format!(
                "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\nPidFile none\n\
                 StrictModes no\nUsePAM no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\n",
                path("host_key"),
                path("client_key.pub")
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("ssh_config"),
            format!(
                "Host *\n  UserKnownHostsFile {}\n  GlobalKnownHostsFile /dev/null\n  StrictHostKeyChecking yes\n  \
                 IdentitiesOnly yes\n  IdentityAgent none\n",
                path("known_hosts")
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let shim = format!("#!/bin/sh\nexec {} -F {} \"$@\"\n", ssh.display(), path("ssh_config"));
        std::fs::write(dir.join("bin/ssh"), shim).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("bin/ssh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let child = Command::new(sshd)
            .args(["-D", "-e", "-f", &path("sshd_config")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut server = TestSshd { child, dir, port };
        for _ in 0..50 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(server);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        server.stop();
        eprintln!("SKIP: sshd did not start");
        None
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn quarry(&self, url: &str, sql: &str) -> Output {
        let user = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout).unwrap();
        let path = format!("{}:{}", self.dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        Command::new(env!("CARGO_BIN_EXE_quarry"))
            .args([url, "--ssh", &format!("{}@127.0.0.1:{}", user.trim(), self.port)])
            .arg("--ssh-key")
            .arg(self.dir.join("client_key"))
            .args(["-e", sql])
            .env("PATH", path)
            .env("QUARRY_CONFIG_DIR", self.dir.join("config"))
            .env("QUARRY_DATA_DIR", self.dir.join("data"))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

#[cfg(unix)]
impl Drop for TestSshd {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(unix)]
#[test]
fn queries_travel_through_an_ssh_tunnel() {
    let servers = [
        server("QUARRY_TEST_PG", "postgres://postgres@127.0.0.1:5432/postgres"),
        server("QUARRY_TEST_MYSQL", "mysql://root@127.0.0.1:3306"),
    ];
    if servers.iter().all(Option::is_none) {
        return;
    }
    let Some(mut sshd) = TestSshd::start("ssh-tunnel") else { return };
    for url in servers.iter().flatten() {
        let o = sshd.quarry(url, "select 42 as answer");
        assert!(o.status.success(), "{url}: {}", String::from_utf8_lossy(&o.stderr));
        assert_eq!(stdout(&o), "answer\n42\n");
    }
    sshd.stop();
    for url in servers.iter().flatten() {
        let o = sshd.quarry(url, "select 42 as answer");
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        assert!(!o.status.success() && err.contains("SSH tunnel"), "without the SSH server there is no other way in: {err}");
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

#[test]
fn backslash_c_opens_saved_connections_by_name() {
    let dir = test_dir("saved-c");
    let other = dir.join("other.db");
    quarry(&[other.to_str().unwrap(), "-e", "create table t (name text)", "-e", "insert into t values ('from other')"], None);
    std::fs::write(dir.join("config/config.toml"), format!("[connections.other]\nurl = \"{}\"\n", other.display())).unwrap();

    let o = quarry_in(&dir, &[dir.join("main.db").to_str().unwrap()], "\\c other\nselect name from t;\n");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(stdout(&o).contains("from other"), "\\c should switch to the saved connection: {}", stdout(&o));
}

#[test]
fn include_runs_special_commands_like_a_script_file() {
    let dir = test_dir("include");
    let script = dir.join("inc.sql");
    std::fs::write(&script, "create table included (id integer);\n\\dt\n").unwrap();
    let include = format!("\\i {}", script.display());
    let o = quarry_in(&dir, &[dir.join("a.db").to_str().unwrap(), "-e", &include], "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(stdout(&o).contains("included"), "\\dt inside the file should list the new table: {}", stdout(&o));
}

#[test]
fn a_failing_command_fails_the_run_like_failing_sql_does() {
    let dir = test_dir("cmd-exit");
    let o = quarry_in(&dir, &[":memory:", "-e", "\\d nosuchtable"], "");
    assert!(!o.status.success(), "a script must be able to tell that the table is missing");
    let o = quarry_in(&dir, &[":memory:", "-e", "\\nosuchcommand"], "");
    assert!(!o.status.success());
    let o = quarry_in(&dir, &[":memory:", "-e", "\\echo fine"], "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    let script = "\\d nosuchtable\nselect 'later' as v;\n";
    let o = quarry_in(&dir, &[":memory:"], script);
    assert!(!o.status.success());
    assert!(!stdout(&o).contains("later"), "a failing command stops the script: {}", stdout(&o));
    let o = quarry_in(&dir, &[":memory:", "--continue-on-error"], script);
    assert!(!o.status.success(), "a failure is still reported in the exit status");
    assert!(stdout(&o).contains("later"), "{}", stdout(&o));
}

#[test]
fn a_large_result_is_written_whole_as_it_arrives() {
    let dir = test_dir("stream");
    let sql = "with recursive n(i) as (select 1 union all select i + 1 from n where i < 50000) select i, 'r' || i as label from n";
    for (format, lines) in [("tsv", 50001), ("csv", 50001), ("jsonl", 50000)] {
        let o = quarry_in(&dir, &[":memory:", "-F", format, "-e", sql], "");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let out = stdout(&o);
        assert_eq!(out.lines().count(), lines, "{format}: one header at most, and no row lost between batches");
        assert!(out.lines().last().unwrap().contains("r50000"), "{format}: {:?}", out.lines().last());
    }
}

#[test]
fn quit_stops_a_script() {
    let dir = test_dir("quit");
    let o = quarry_in(&dir, &[":memory:"], "select 'before' as v;\n\\q\nselect 'after' as v;\n");
    assert!(o.status.success());
    assert!(stdout(&o).contains("before") && !stdout(&o).contains("after"), "{}", stdout(&o));
}

#[test]
fn a_failure_stops_the_rest_of_the_script_unless_asked_to_continue() {
    let dir = test_dir("stop");
    let script = "select * from missing;\n\\echo still running\nselect 'later' as v;\n";
    let o = quarry_in(&dir, &[":memory:"], script);
    assert!(!o.status.success());
    assert!(!stdout(&o).contains("still running") && !stdout(&o).contains("later"), "{}", stdout(&o));

    let o = quarry_in(&dir, &[":memory:", "--continue-on-error"], script);
    assert!(!o.status.success(), "a failure is still reported in the exit status");
    assert!(stdout(&o).contains("still running") && stdout(&o).contains("later"), "{}", stdout(&o));
}

#[cfg(unix)]
#[test]
fn config_warnings_are_shown_not_swallowed() {
    use std::os::unix::fs::PermissionsExt;
    let dir = test_dir("warn");
    let config = dir.join("config/config.toml");
    std::fs::write(&config, "[connections.leaky]\nurl = \"postgres://me:secret@db/app\"\n").unwrap();
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644)).unwrap();
    let o = quarry_in(&dir, &[":memory:", "-e", "select 1"], "");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("readable by other users") && err.contains("leaky"), "{err}");
}

#[test]
fn save_never_writes_a_password_to_the_config() {
    let dir = test_dir("save");
    let o = quarry_in(&dir, &["postgres://me:s3cret@127.0.0.1:1/app?sslmode=disable", "--save", "leaky", "-w", "-e", "select 1"], "");
    let config = std::fs::read_to_string(dir.join("config/config.toml")).unwrap();
    assert!(config.contains("[connections.leaky]") && config.contains("sslmode=disable"), "{config}");
    assert!(!config.contains("s3cret"), "the password must not be saved: {config}");
    assert!(String::from_utf8_lossy(&o.stderr).contains("password was not saved"));
}

#[test]
fn tab_completion_offers_saved_connections_without_passwords_or_side_effects() {
    let dir = test_dir("complete");
    let config = dir.join("config/config.toml");
    std::fs::write(&config, "[connections.prod]\nurl = \"postgres://deploy:s3cret@db.internal/app\"\nreadonly = true\n").unwrap();
    let complete = |words: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_quarry"))
            .args(["--", "quarry"])
            .args(words)
            .env("COMPLETE", "fish")
            .env("QUARRY_CONFIG_DIR", dir.join("config"))
            .env("QUARRY_DATA_DIR", dir.join("data"))
            .current_dir(&dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    assert_eq!(complete(&["pr"]), "prod\tpostgres://deploy@db.internal/app (read-only)\n");
    let urls = complete(&["postgres://"]);
    assert!(urls.contains("postgres://deploy@db.internal/app") && !urls.contains("s3cret"), "{urls}");
    assert!(complete(&["--format", "mark"]).starts_with("markdown\t"));
    assert!(!dir.join("data").exists(), "completing must not create the data directory");
    assert_eq!(std::fs::read_dir(dir.join("config")).unwrap().count(), 1, "nor write anything next to the config");
}

/// `--default-config` is meant to be saved as the config file as-is, so it must parse to exactly
/// the built-in defaults and must not depend on (or read) an existing config.
#[test]
fn default_config_prints_a_file_that_parses_to_the_defaults() {
    let out = quarry(&["--default-config", "--config", "/nonexistent/quarry.toml"], None);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("[main]") && text.contains("# [keys]"));
    assert_eq!(quarry::config::Config::parse(&text).unwrap(), quarry::config::Config::default());
}
