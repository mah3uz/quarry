use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{ArgAction, Parser};

use crate::config::Config;
use crate::conn::ssh::Tunnel;
use crate::conn::{ConnSpec, SshSpec, SslMode, passfile};
use crate::db::{Backend, Connection, ErrorKind};

#[derive(Parser, Debug, Default)]
#[command(
    name = "quarry",
    version,
    about = "A fast, beautiful SQL client and TUI for PostgreSQL, MySQL/MariaDB and SQLite",
    disable_help_flag = true,
    after_help = "Examples:\n  quarry postgres://user@localhost/app\n  quarry mysql://root@127.0.0.1/shop -e 'select * from orders' --format csv\n  quarry data.db\n  quarry --tui prod          (saved connection 'prod')\n  quarry                     (TUI connection manager)"
)]
pub struct Args {
    /// URL (postgres://, mysql://, sqlite:), saved connection name, SQLite file, or database name
    pub target: Option<String>,
    /// Database name (when TARGET is a URL without one, or pgcli-style `quarry dbname user`)
    pub extra: Option<String>,

    #[arg(short = 'h', long)]
    pub host: Option<String>,
    #[arg(short = 'p', short_alias = 'P', long)]
    pub port: Option<u16>,
    #[arg(short = 'u', short_alias = 'U', long, alias = "username")]
    pub user: Option<String>,
    #[arg(short = 'd', short_alias = 'D', long, alias = "dbname")]
    pub database: Option<String>,
    /// Unix socket path
    #[arg(short = 'S', long)]
    pub socket: Option<PathBuf>,
    /// Backend when connecting with flags only: postgres, mysql or sqlite
    #[arg(long, value_parser = parse_backend)]
    pub backend: Option<Backend>,
    /// Always prompt for a password
    #[arg(short = 'W', long = "password")]
    pub force_password: bool,
    /// Never prompt for a password
    #[arg(short = 'w', long = "no-password")]
    pub no_password: bool,

    /// disable | prefer | require | verify-ca | verify-full
    #[arg(long, value_parser = parse_ssl)]
    pub ssl_mode: Option<SslMode>,
    #[arg(long)]
    pub ssl_ca: Option<PathBuf>,
    #[arg(long)]
    pub ssl_cert: Option<PathBuf>,
    #[arg(long)]
    pub ssl_key: Option<PathBuf>,
    /// Tunnel through SSH: [user@]host[:port]
    #[arg(long)]
    pub ssh: Option<String>,
    #[arg(long)]
    pub ssh_key: Option<PathBuf>,

    /// Execute SQL (repeatable) and exit
    #[arg(short = 'e', long = "execute", action = ArgAction::Append)]
    pub execute: Vec<String>,
    /// Execute SQL from a file and exit
    #[arg(short = 'f', long)]
    pub file: Option<PathBuf>,
    /// Output format: rounded, psql, ascii, csv, tsv, json, jsonl, markdown, html, vertical, sql-insert …
    #[arg(short = 'F', long)]
    pub format: Option<String>,
    /// Launch the full-screen TUI
    #[arg(short = 'T', long)]
    pub tui: bool,
    /// Refuse statements that modify data or schema
    #[arg(short = 'r', long)]
    pub readonly: bool,
    /// SQL to run right after connecting (repeatable)
    #[arg(long = "init-command", action = ArgAction::Append)]
    pub init_command: Vec<String>,
    #[arg(long)]
    pub prompt: Option<String>,
    #[arg(long)]
    pub theme: Option<String>,
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// List saved connections
    #[arg(short = 'l', long = "list")]
    pub list: bool,
    /// Save this connection under NAME in the config
    #[arg(long, value_name = "NAME")]
    pub save: Option<String>,
    #[arg(long)]
    pub no_color: bool,
    /// Skip the intro banner and goodbye message
    #[arg(long)]
    pub less_chatty: bool,
    /// Ask before displaying more than N rows (0 = never)
    #[arg(long)]
    pub row_limit: Option<usize>,
    /// Continue executing a script after an error
    #[arg(long)]
    pub continue_on_error: bool,
    /// Print help
    #[arg(long, action = ArgAction::Help)]
    pub help: Option<bool>,
}

fn parse_backend(s: &str) -> Result<Backend, String> {
    match s.to_ascii_lowercase().as_str() {
        "pg" | "postgres" | "postgresql" => Ok(Backend::Postgres),
        "mysql" | "mariadb" | "my" => Ok(Backend::MySql),
        "sqlite" | "sqlite3" | "lite" => Ok(Backend::Sqlite),
        _ => Err(format!("unknown backend '{s}' (postgres, mysql, sqlite)")),
    }
}

fn parse_ssl(s: &str) -> Result<SslMode, String> {
    SslMode::parse(s).ok_or_else(|| format!("unknown ssl mode '{s}'"))
}

/// Connection target resolved from args + config, plus how to obtain a password.
pub struct Resolved {
    pub spec: ConnSpec,
    pub password_command: Option<String>,
    /// Name of the saved connection used, if any.
    pub saved_name: Option<String>,
}

impl Args {
    fn has_conn_flags(&self) -> bool {
        self.host.is_some() || self.port.is_some() || self.user.is_some() || self.database.is_some()
            || self.socket.is_some() || self.backend.is_some()
    }
}

/// Returns None when nothing identifies a connection (→ TUI connection manager).
pub fn resolve(args: &Args, config: &Config) -> Result<Option<Resolved>> {
    let mut password_command = None;
    let mut saved_name = None;
    let mut spec = match args.target.as_deref() {
        Some(t) if config.connections.contains_key(t) => {
            let saved = &config.connections[t];
            let mut spec = ConnSpec::parse(&saved.url).map_err(|e| anyhow!("saved connection '{t}': {e}"))?;
            spec.readonly |= saved.readonly;
            if let Some(ssh) = &saved.ssh {
                spec.ssh = SshSpec::parse(ssh);
            }
            spec.init_commands.extend(saved.init_commands.iter().cloned());
            password_command = saved.password_command.clone();
            saved_name = Some(t.to_string());
            spec
        }
        Some(t) => match ConnSpec::parse(t) {
            Ok(spec) => spec,
            Err(_) if !t.contains("://") && !t.contains('/') => {
                // pgcli/mycli style: `quarry dbname [user]`
                let backend = args.backend.unwrap_or_else(|| guess_backend(args));
                let mut spec = ConnSpec::new(backend);
                spec.database = Some(t.to_string());
                if let Some(u) = &args.extra {
                    spec.user = Some(u.clone());
                }
                spec
            }
            Err(e) => bail!(e),
        },
        None if args.has_conn_flags() => ConnSpec::new(args.backend.unwrap_or_else(|| guess_backend(args))),
        None => return Ok(None),
    };

    if let Some(extra) = &args.extra
        && spec.database.is_none() && spec.backend != Backend::Sqlite {
            spec.database = Some(extra.clone());
        }
    if let Some(h) = &args.host {
        spec.host = Some(h.clone());
    }
    if let Some(p) = args.port {
        spec.port = Some(p);
    }
    if let Some(u) = &args.user {
        spec.user = Some(u.clone());
    }
    if let Some(d) = &args.database {
        spec.database = Some(d.clone());
    }
    if let Some(s) = &args.socket {
        spec.socket = Some(s.clone());
    }
    if let Some(m) = args.ssl_mode {
        spec.ssl_mode = m;
    }
    for (dst, src) in [
        (&mut spec.ssl_ca, &args.ssl_ca),
        (&mut spec.ssl_cert, &args.ssl_cert),
        (&mut spec.ssl_key, &args.ssl_key),
    ] {
        if src.is_some() {
            *dst = src.clone();
        }
    }
    if let Some(ssh) = &args.ssh {
        spec.ssh = Some(SshSpec::parse(ssh).ok_or_else(|| anyhow!("invalid --ssh '{ssh}'"))?);
    }
    if let (Some(ssh), Some(key)) = (spec.ssh.as_mut(), &args.ssh_key) {
        ssh.identity = Some(key.clone());
    }
    spec.readonly |= args.readonly;
    spec.init_commands.extend(args.init_command.iter().cloned());
    if spec.backend != Backend::Sqlite {
        passfile::apply_defaults(&mut spec);
    }
    Ok(Some(Resolved { spec, password_command, saved_name }))
}

fn guess_backend(args: &Args) -> Backend {
    let socket_is_mysql = args.socket.as_ref().is_some_and(|s| s.to_string_lossy().contains("mysql"));
    if args.port == Some(3306) || socket_is_mysql {
        Backend::MySql
    } else {
        Backend::Postgres
    }
}

pub fn run_password_command(cmd: &str) -> Result<String> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stderr(std::process::Stdio::inherit())
        .output()
        .with_context(|| format!("running password command `{cmd}`"))?;
    if !out.status.success() {
        bail!("password command exited with {}", out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches(['\n', '\r']).to_string())
}

/// An open connection plus the SSH tunnel it depends on (dropped together).
pub struct Opened {
    pub conn: Connection,
    /// The spec actually used (password filled, host/port rewritten for tunnels).
    pub spec: ConnSpec,
    pub tunnel: Option<Tunnel>,
}

/// Connects, prompting for a password on auth failure when interactive (like psql/pgcli).
pub async fn open(
    mut spec: ConnSpec,
    password_command: Option<&str>,
    force_prompt: bool,
    allow_prompt: bool,
) -> Result<Opened> {
    let tunnel = match spec.ssh.clone() {
        Some(ssh) if spec.backend != Backend::Sqlite => {
            let host = spec.host_or_default().to_string();
            let port = spec.port_or_default();
            let t = Tunnel::open(&ssh, &host, port).await.context("opening SSH tunnel")?;
            spec.host = Some("127.0.0.1".into());
            spec.port = Some(t.local_port);
            spec.socket = None;
            Some(t)
        }
        _ => None,
    };
    if spec.password.is_none()
        && let Some(cmd) = password_command {
            spec.password = Some(run_password_command(cmd)?);
        }
    let can_prompt = allow_prompt && std::io::stdin().is_terminal();
    if force_prompt && can_prompt && spec.backend != Backend::Sqlite {
        spec.password = Some(prompt_password(&spec)?);
    }
    let mut attempts = 0;
    loop {
        match Connection::connect(&spec).await {
            Ok(conn) => return Ok(Opened { conn, spec, tunnel }),
            Err(e) if e.kind == ErrorKind::Auth && can_prompt && attempts < 3 => {
                if spec.password.is_some() || attempts > 0 {
                    eprintln!("{}", e.message);
                }
                attempts += 1;
                spec.password = Some(prompt_password(&spec)?);
            }
            Err(e) => {
                let url = spec.display_url();
                return Err(anyhow::Error::new(e).context(format!("could not connect to {url}")));
            }
        }
    }
}

fn prompt_password(spec: &ConnSpec) -> Result<String> {
    let who = format!("{}@{}", spec.user_or_default(), spec.host.as_deref().unwrap_or("localhost"));
    rpassword::prompt_password(format!("Password for {who}: ")).context("reading password")
}

pub fn connect_timeout(spec: &ConnSpec) -> Duration {
    spec.connect_timeout.unwrap_or(Duration::from_secs(15))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SavedConnection;

    fn args(v: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("quarry").chain(v.iter().copied())).unwrap()
    }

    #[test]
    fn nothing_given_means_connection_manager() {
        assert!(resolve(&args(&[]), &Config::default()).unwrap().is_none());
    }

    #[test]
    fn saved_connection_by_name_carries_its_settings() {
        let mut cfg = Config::default();
        cfg.connections.insert(
            "local".into(),
            SavedConnection { url: "sqlite::memory:".into(), readonly: true, ..Default::default() },
        );
        let r = resolve(&args(&["local"]), &cfg).unwrap().unwrap();
        assert_eq!(r.saved_name.as_deref(), Some("local"));
        assert!(r.spec.readonly);
    }

    #[test]
    fn flags_override_url_parts() {
        let r = resolve(&args(&["sqlite:a.db", "--readonly"]), &Config::default()).unwrap().unwrap();
        assert!(r.spec.readonly);
    }

    #[test]
    fn mysql_style_short_flags_are_accepted() {
        let a = args(&["-u", "root", "-P", "3306", "-D", "shop"]);
        assert_eq!((a.user.as_deref(), a.port, a.database.as_deref()), (Some("root"), Some(3306), Some("shop")));
        assert_eq!(guess_backend(&a), Backend::MySql);
    }
}
