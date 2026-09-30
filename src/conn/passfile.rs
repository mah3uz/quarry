use std::path::{Path, PathBuf};

use super::ConnSpec;
use crate::config::group_or_world_accessible;
use crate::db::Backend;

/// Password from `$PGPASSFILE` or `~/.pgpass`, like libpq: first matching line wins, `*` matches
/// anything, and the file is ignored when group/world accessible.
pub fn pgpass_lookup(host: &str, port: u16, database: &str, user: &str) -> Option<String> {
    let path = std::env::var_os("PGPASSFILE")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".pgpass")))?;
    pgpass_lookup_in(&path, host, port, database, user)
}

pub fn pgpass_lookup_in(path: &Path, host: &str, port: u16, database: &str, user: &str) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    if group_or_world_accessible(path) {
        eprintln!(
            "WARNING: password file \"{}\" has group or world access; permissions should be u=rw (0600) or less",
            path.display()
        );
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    pgpass_find(&text, host, port, database, user)
}

/// Splits a `.pgpass` line on unescaped `:`; the fifth field (password) takes the rest of the line.
fn pgpass_fields(line: &str) -> Option<[String; 5]> {
    let mut fields: [String; 5] = Default::default();
    let mut idx = 0;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => fields[idx].push(chars.next().unwrap_or('\\')),
            ':' if idx < 4 => idx += 1,
            c => fields[idx].push(c),
        }
    }
    (idx == 4).then_some(fields)
}

pub fn pgpass_find(text: &str, host: &str, port: u16, database: &str, user: &str) -> Option<String> {
    let port = port.to_string();
    let wanted = [host, port.as_str(), database, user];
    text.lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(pgpass_fields)
        .find(|f| f[..4].iter().zip(wanted).all(|(pat, val)| pat == "*" || pat == val))
        .map(|f| f[4].clone())
}

/// `[client]` / `[mysql]` options from MySQL option files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MyCnf {
    pub user: Option<String>,
    pub password: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub socket: Option<String>,
    pub database: Option<String>,
}

/// Reads `/etc/my.cnf`, `/etc/mysql/my.cnf` and `~/.my.cnf` in that order (later files win).
/// `~/.mylogin.cnf` is encrypted and skipped.
pub fn mycnf_client() -> MyCnf {
    let mut paths = vec![PathBuf::from("/etc/my.cnf"), PathBuf::from("/etc/mysql/my.cnf")];
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".my.cnf"));
    }
    let mut cnf = MyCnf::default();
    for p in paths {
        if let Ok(text) = std::fs::read_to_string(&p) {
            parse_mycnf(&text, &mut cnf);
        }
    }
    cnf
}

fn mycnf_value(raw: &str) -> String {
    let raw = raw.trim();
    let quote = raw.chars().next().filter(|c| *c == '"' || *c == '\'');
    let mut out = String::new();
    let mut chars = raw.chars().skip(quote.is_some() as usize);
    while let Some(c) = chars.next() {
        match c {
            c if Some(c) == quote => break,
            '#' if quote.is_none() => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('b') => out.push('\u{8}'),
                Some('s') => out.push(' '),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            c => out.push(c),
        }
    }
    if quote.is_none() { out.trim_end().to_string() } else { out }
}

pub fn parse_mycnf(text: &str, cnf: &mut MyCnf) {
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('!') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.split(']').next()) {
            in_section = matches!(name.trim().to_ascii_lowercase().as_str(), "client" | "mysql");
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = mycnf_value(value);
        match key.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "user" => cnf.user = Some(value),
            "password" => cnf.password = Some(value),
            "host" => cnf.host = Some(value),
            "port" => cnf.port = value.parse().ok().or(cnf.port),
            "socket" => cnf.socket = Some(value),
            "database" => cnf.database = Some(value),
            _ => {}
        }
    }
}

/// Fills missing connection fields from the environment and password files.
/// Precedence: explicit value > environment (`PG*`, `MYSQL_PWD`, `MYSQL_HOST`…) > `.pgpass` / `my.cnf`.
pub fn apply_defaults(spec: &mut ConnSpec) {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    match spec.backend {
        Backend::Postgres => apply_pg(spec, &env, &pgpass_lookup),
        Backend::MySql => apply_my(spec, &env, &mycnf_client()),
        Backend::Sqlite => {}
    }
}

type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

fn apply_pg(spec: &mut ConnSpec, env: Env, pgpass: &dyn Fn(&str, u16, &str, &str) -> Option<String>) {
    if spec.user.is_none() {
        spec.user = env("PGUSER");
    }
    if spec.host.is_none() && spec.socket.is_none() {
        match env("PGHOST") {
            Some(h) if h.starts_with('/') => spec.socket = Some(PathBuf::from(h)),
            h => spec.host = h,
        }
    }
    if spec.port.is_none() {
        spec.port = env("PGPORT").and_then(|p| p.parse().ok());
    }
    if spec.database.is_none() {
        spec.database = env("PGDATABASE");
    }
    if spec.password.is_none() {
        spec.password = env("PGPASSWORD").or_else(|| {
            // libpq matches socket connections against "localhost".
            let host = if spec.socket.is_some() { "localhost".to_string() } else { spec.host_or_default().to_string() };
            let user = spec.user_or_default();
            let db = spec.database.clone().unwrap_or_else(|| user.clone());
            pgpass(&host, spec.port_or_default(), &db, &user)
        });
    }
}

fn apply_my(spec: &mut ConnSpec, env: Env, cnf: &MyCnf) {
    if spec.user.is_none() {
        spec.user = cnf.user.clone();
    }
    if spec.host.is_none() && spec.socket.is_none() {
        spec.host = env("MYSQL_HOST").or_else(|| cnf.host.clone());
        if spec.host.is_none() {
            spec.socket = env("MYSQL_UNIX_PORT").or_else(|| cnf.socket.clone()).map(PathBuf::from);
        }
    }
    if spec.port.is_none() {
        spec.port = env("MYSQL_TCP_PORT").and_then(|p| p.parse().ok()).or(cnf.port);
    }
    if spec.database.is_none() {
        spec.database = cnf.database.clone();
    }
    if spec.password.is_none() {
        spec.password = env("MYSQL_PWD").or_else(|| cnf.password.clone());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const PGPASS: &str = "\
# comment
db.example:5432:app:alice:first
db.example:*:*:bob:bob\\:pw\\\\x
*:*:*:alice:fallback
localhost:5432:*:*:local:with:colons
";

    #[test]
    fn pgpass_first_match_wins_with_wildcards_and_escapes() {
        assert_eq!(pgpass_find(PGPASS, "db.example", 5432, "app", "alice").as_deref(), Some("first"));
        assert_eq!(pgpass_find(PGPASS, "db.example", 6000, "other", "alice").as_deref(), Some("fallback"));
        assert_eq!(pgpass_find(PGPASS, "db.example", 1, "x", "bob").as_deref(), Some("bob:pw\\x"));
        assert_eq!(pgpass_find(PGPASS, "localhost", 5432, "x", "carol").as_deref(), Some("local:with:colons"));
        assert_eq!(pgpass_find(PGPASS, "elsewhere", 5432, "x", "carol"), None);
        assert_eq!(pgpass_find("h:5432:db\n", "h", 5432, "db", "u"), None, "short lines never match");
    }

    #[cfg(unix)]
    #[test]
    fn pgpass_ignored_when_group_or_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("quarry-pgpass-{}", std::process::id()));
        std::fs::write(&path, "*:*:*:*:secret\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(pgpass_lookup_in(&path, "h", 1, "d", "u").as_deref(), Some("secret"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(pgpass_lookup_in(&path, "h", 1, "d", "u"), None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn mycnf_client_and_mysql_sections_last_wins() {
        let mut cnf = MyCnf::default();
        parse_mycnf(
            "!include /etc/other.cnf\n[mysqld]\nuser = server\n[client]\nuser = alice\npassword = \"p#ss word\"\n\
             host=db.local # trailing comment\nport = 3307\n[mysql]\ndatabase = shop\nuser = bob\n",
            &mut cnf,
        );
        assert_eq!(
            cnf,
            MyCnf {
                user: Some("bob".into()),
                password: Some("p#ss word".into()),
                host: Some("db.local".into()),
                port: Some(3307),
                socket: None,
                database: Some("shop".into()),
            }
        );
        parse_mycnf("[CLIENT]\npassword='x\\ty'\nsocket = /tmp/my.sock\n", &mut cnf);
        assert_eq!(cnf.password.as_deref(), Some("x\ty"));
        assert_eq!(cnf.socket.as_deref(), Some("/tmp/my.sock"));
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| m.get(k).cloned()
    }

    #[test]
    fn pg_defaults_env_then_pgpass_but_never_override_explicit() {
        let env = env_of(&[("PGUSER", "envuser"), ("PGHOST", "/run/pg"), ("PGPORT", "6543"), ("PGDATABASE", "envdb")]);
        let seen = std::cell::RefCell::new(None);
        let pgpass = |h: &str, p: u16, d: &str, u: &str| {
            *seen.borrow_mut() = Some((h.to_string(), p, d.to_string(), u.to_string()));
            Some("frompass".to_string())
        };
        let mut s = ConnSpec::new(Backend::Postgres);
        apply_pg(&mut s, &env, &pgpass);
        assert_eq!(s.user.as_deref(), Some("envuser"));
        assert_eq!(s.socket, Some(PathBuf::from("/run/pg")));
        assert_eq!((s.port, s.database.as_deref(), s.password.as_deref()), (Some(6543), Some("envdb"), Some("frompass")));
        assert_eq!(*seen.borrow(), Some(("localhost".into(), 6543, "envdb".into(), "envuser".into())));

        let mut explicit = ConnSpec::parse("postgres://me:given@h:1/db").unwrap();
        apply_pg(&mut explicit, &env_of(&[("PGPASSWORD", "env")]), &|_, _, _, _| Some("file".into()));
        assert_eq!((explicit.user.as_deref(), explicit.password.as_deref()), (Some("me"), Some("given")));

        let mut s = ConnSpec::parse("postgres://me@h/db").unwrap();
        apply_pg(&mut s, &env_of(&[("PGPASSWORD", "env")]), &|_, _, _, _| Some("file".into()));
        assert_eq!(s.password.as_deref(), Some("env"), "PGPASSWORD beats .pgpass like libpq");
    }

    #[test]
    fn mysql_defaults_from_env_and_mycnf() {
        let cnf = MyCnf {
            user: Some("cnfuser".into()),
            password: Some("cnfpw".into()),
            host: None,
            port: Some(3307),
            socket: Some("/tmp/mysql.sock".into()),
            database: Some("cnfdb".into()),
        };
        let mut s = ConnSpec::new(Backend::MySql);
        apply_my(&mut s, &env_of(&[("MYSQL_PWD", "envpw")]), &cnf);
        assert_eq!(s.user.as_deref(), Some("cnfuser"));
        assert_eq!(s.password.as_deref(), Some("envpw"));
        assert_eq!(s.socket, Some(PathBuf::from("/tmp/mysql.sock")));
        assert_eq!((s.port, s.database.as_deref()), (Some(3307), Some("cnfdb")));

        let mut s = ConnSpec::parse("mysql://root@db:3306/x").unwrap();
        apply_my(&mut s, &env_of(&[("MYSQL_HOST", "other")]), &cnf);
        assert_eq!((s.host.as_deref(), s.socket.as_ref(), s.password.as_deref()), (Some("db"), None, Some("cnfpw")));
    }
}
