use std::path::{Path, PathBuf};
use std::time::Duration;

use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};

use crate::db::Backend;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    pub fn parse(s: &str) -> Option<SslMode> {
        Some(match s.to_ascii_lowercase().replace('_', "-").as_str() {
            "disable" | "disabled" | "off" | "false" => SslMode::Disable,
            "allow" | "prefer" | "preferred" => SslMode::Prefer,
            "require" | "required" | "on" | "true" => SslMode::Require,
            "verify-ca" => SslMode::VerifyCa,
            "verify-full" | "verify-identity" => SslMode::VerifyFull,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshSpec {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    pub identity: Option<PathBuf>,
}

impl SshSpec {
    /// `[user@]host[:port]`
    pub fn parse(s: &str) -> Option<SshSpec> {
        let (user, rest) = match s.rsplit_once('@') {
            Some((u, r)) => (Some(u.to_string()), r),
            None => (None, s),
        };
        let (host, port) = match rest.rsplit_once(':') {
            Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse().ok()),
            _ => (rest, None),
        };
        (!host.is_empty()).then(|| SshSpec { user, host: host.to_string(), port, identity: None })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConnSpec {
    pub backend: Backend,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub database: Option<String>,
    /// Unix socket path (pg: directory or full socket path; mysql: socket file).
    pub socket: Option<PathBuf>,
    /// SQLite database file (or `:memory:`).
    pub path: Option<PathBuf>,
    pub ssl_mode: SslMode,
    pub ssl_ca: Option<PathBuf>,
    pub ssl_cert: Option<PathBuf>,
    pub ssl_key: Option<PathBuf>,
    /// Extra driver parameters (application_name, options, charset, …).
    pub params: Vec<(String, String)>,
    pub readonly: bool,
    pub connect_timeout: Option<Duration>,
    pub ssh: Option<SshSpec>,
    pub init_commands: Vec<String>,
}

impl ConnSpec {
    pub fn new(backend: Backend) -> Self {
        ConnSpec {
            backend,
            host: None,
            port: None,
            user: None,
            password: None,
            database: None,
            socket: None,
            path: None,
            ssl_mode: SslMode::default(),
            ssl_ca: None,
            ssl_cert: None,
            ssl_key: None,
            params: Vec::new(),
            readonly: false,
            connect_timeout: Some(Duration::from_secs(15)),
            ssh: None,
            init_commands: Vec::new(),
        }
    }

    pub fn sqlite(path: impl Into<PathBuf>) -> Self {
        ConnSpec { path: Some(path.into()), ..ConnSpec::new(Backend::Sqlite) }
    }

    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    pub fn host_or_default(&self) -> &str {
        self.host.as_deref().unwrap_or("localhost")
    }

    pub fn port_or_default(&self) -> u16 {
        self.port.or(self.backend.default_port()).unwrap_or(0)
    }

    /// Default user when none given: the OS user (pg / mysql convention).
    pub fn user_or_default(&self) -> String {
        self.user.clone().unwrap_or_else(|| {
            std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "root".into())
        })
    }

    /// Parses a URL (`postgres://`, `postgresql://`, `mysql://`, `mariadb://`, `sqlite:`),
    /// or a bare SQLite file path.
    pub fn parse(input: &str) -> Result<ConnSpec, String> {
        let input = input.trim();
        let lower = input.to_ascii_lowercase();
        if lower.starts_with("sqlite:") || lower.starts_with("file:") {
            return Ok(Self::parse_sqlite_url(input));
        }
        let Some((scheme, _)) = input.split_once("://") else {
            if looks_like_sqlite_path(input) {
                return Ok(ConnSpec::sqlite(expand_tilde(input)));
            }
            return Err(format!(
                "cannot understand '{input}': expected postgres://, mysql://, sqlite: URL or a SQLite file path"
            ));
        };
        let backend = match scheme.to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" | "pg" | "pgsql" => Backend::Postgres,
            "mysql" | "mariadb" | "mysqlx" => Backend::MySql,
            other => return Err(format!("unsupported scheme '{other}'")),
        };
        let url = url::Url::parse(input).map_err(|e| format!("invalid URL: {e}"))?;
        let mut spec = ConnSpec::new(backend);
        let dec = |s: &str| percent_decode_str(s).decode_utf8_lossy().into_owned();
        if !url.username().is_empty() {
            spec.user = Some(dec(url.username()));
        }
        spec.password = url.password().map(dec);
        if let Some(h) = url.host_str() {
            let h = dec(h);
            if h.starts_with('/') {
                spec.socket = Some(PathBuf::from(h));
            } else if !h.is_empty() {
                spec.host = Some(h.trim_start_matches('[').trim_end_matches(']').to_string());
            }
        }
        spec.port = url.port();
        let path = url.path().trim_start_matches('/');
        if !path.is_empty() {
            spec.database = Some(dec(path));
        }
        for (k, v) in url.query_pairs() {
            let (k, v) = (k.into_owned(), v.into_owned());
            match k.to_ascii_lowercase().replace('-', "_").as_str() {
                "sslmode" | "ssl_mode" | "ssl" => {
                    spec.ssl_mode = SslMode::parse(&v).ok_or_else(|| format!("unknown ssl mode '{v}'"))?
                }
                "sslrootcert" | "ssl_ca" | "sslca" => spec.ssl_ca = Some(expand_tilde(&v)),
                "sslcert" | "ssl_cert" => spec.ssl_cert = Some(expand_tilde(&v)),
                "sslkey" | "ssl_key" => spec.ssl_key = Some(expand_tilde(&v)),
                "host" if v.starts_with('/') => spec.socket = Some(PathBuf::from(v)),
                "host" => spec.host = Some(v),
                "socket" | "unix_socket" => spec.socket = Some(expand_tilde(&v)),
                "port" => spec.port = v.parse().ok(),
                "user" => spec.user = Some(v),
                "password" => spec.password = Some(v),
                "dbname" | "database" => spec.database = Some(v),
                "connect_timeout" => spec.connect_timeout = v.parse().ok().map(Duration::from_secs),
                "readonly" | "read_only" => spec.readonly = matches!(v.as_str(), "1" | "true" | "yes" | "on"),
                "ssh" => spec.ssh = SshSpec::parse(&v),
                _ => spec.params.push((k, v)),
            }
        }
        Ok(spec)
    }

    fn parse_sqlite_url(input: &str) -> ConnSpec {
        let rest = input.split_once(':').map(|x| x.1).unwrap_or("");
        let (path_part, query) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (rest, None),
        };
        // sqlite:///abs → /abs ; sqlite://rel → rel ; sqlite:rel → rel
        let path = if let Some(p) = path_part.strip_prefix("//") { p } else { path_part };
        let path = percent_decode_str(path).decode_utf8_lossy().into_owned();
        let path = if path.is_empty() { ":memory:".to_string() } else { path };
        let mut spec = ConnSpec::sqlite(expand_tilde(&path));
        if let Some(q) = query {
            for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
                match k.as_ref() {
                    "mode" if v == "ro" => spec.readonly = true,
                    "readonly" | "read_only" => spec.readonly = matches!(v.as_ref(), "1" | "true" | "yes" | "on"),
                    _ => spec.params.push((k.into_owned(), v.into_owned())),
                }
            }
        }
        spec
    }

    /// URL for display: the password is never included.
    pub fn display_url(&self) -> String {
        match self.backend {
            Backend::Sqlite => format!(
                "sqlite:{}",
                self.path.as_deref().map(|p| p.display().to_string()).unwrap_or_else(|| ":memory:".into())
            ),
            b => {
                let scheme = if b == Backend::Postgres { "postgres" } else { "mysql" };
                let user = self.user.as_deref().map(|u| format!("{u}@")).unwrap_or_default();
                let host = match (&self.socket, &self.host) {
                    (Some(s), _) => s.display().to_string(),
                    (None, Some(h)) => h.clone(),
                    (None, None) => "localhost".into(),
                };
                let port = self.port.map(|p| format!(":{p}")).unwrap_or_default();
                let db = self.database.as_deref().unwrap_or("");
                format!("{scheme}://{user}{host}{port}/{db}")
            }
        }
    }

    /// Short label for prompts, tabs and the status bar: `user@host/db` or the sqlite file name.
    pub fn label(&self) -> String {
        match self.backend {
            Backend::Sqlite => self
                .path
                .as_deref()
                .and_then(Path::file_name)
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| ":memory:".into()),
            _ => format!(
                "{}@{}/{}",
                self.user_or_default(),
                self.host.as_deref().unwrap_or("localhost"),
                self.database.as_deref().unwrap_or("")
            ),
        }
    }
}

pub fn looks_like_sqlite_path(s: &str) -> bool {
    if s == ":memory:" {
        return true;
    }
    let lower = s.to_ascii_lowercase();
    [".db", ".sqlite", ".sqlite3", ".db3", ".s3db", ".sl3"].iter().any(|e| lower.ends_with(e))
        || (Path::new(&*expand_tilde(s)).is_file() && is_sqlite_file(&expand_tilde(s)))
}

fn is_sqlite_file(p: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 16];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut buf)).is_ok() && &buf == b"SQLite format 3\0"
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_url() {
        let s = ConnSpec::parse("postgresql://bob:p%40ss@db.example:6543/app?sslmode=verify-full&application_name=x")
            .unwrap();
        assert_eq!(s.backend, Backend::Postgres);
        assert_eq!(s.user.as_deref(), Some("bob"));
        assert_eq!(s.password.as_deref(), Some("p@ss"));
        assert_eq!(s.host.as_deref(), Some("db.example"));
        assert_eq!(s.port, Some(6543));
        assert_eq!(s.database.as_deref(), Some("app"));
        assert_eq!(s.ssl_mode, SslMode::VerifyFull);
        assert_eq!(s.param("application_name"), Some("x"));
    }

    #[test]
    fn display_url_never_leaks_password() {
        let s = ConnSpec::parse("mysql://root:secret@127.0.0.1/shop").unwrap();
        assert!(!s.display_url().contains("secret"));
        assert_eq!(s.display_url(), "mysql://root@127.0.0.1/shop");
    }

    #[test]
    fn socket_forms() {
        let s = ConnSpec::parse("postgres://u@%2Fvar%2Frun%2Fpostgresql/db").unwrap();
        assert_eq!(s.socket, Some(PathBuf::from("/var/run/postgresql")));
        let s = ConnSpec::parse("mysql://u@localhost/db?socket=/tmp/mysql.sock").unwrap();
        assert_eq!(s.socket, Some(PathBuf::from("/tmp/mysql.sock")));
    }

    #[test]
    fn sqlite_forms() {
        assert_eq!(ConnSpec::parse("sqlite:///tmp/a.db").unwrap().path, Some(PathBuf::from("/tmp/a.db")));
        assert_eq!(ConnSpec::parse("sqlite:rel.db").unwrap().path, Some(PathBuf::from("rel.db")));
        assert_eq!(ConnSpec::parse("sqlite://rel.db").unwrap().path, Some(PathBuf::from("rel.db")));
        assert_eq!(ConnSpec::parse("data/x.sqlite3").unwrap().backend, Backend::Sqlite);
        assert_eq!(ConnSpec::parse(":memory:").unwrap().path, Some(PathBuf::from(":memory:")));
        assert!(ConnSpec::parse("sqlite:x.db?mode=ro").unwrap().readonly);
    }

    #[test]
    fn rejects_garbage() {
        assert!(ConnSpec::parse("oracle://x").is_err());
        assert!(ConnSpec::parse("not-a-db").is_err());
    }

    #[test]
    fn ssh_spec() {
        let s = SshSpec::parse("deploy@bastion:2222").unwrap();
        assert_eq!((s.user.as_deref(), s.host.as_str(), s.port), (Some("deploy"), "bastion", Some(2222)));
    }
}
