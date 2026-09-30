//! Tab-completion candidates, computed when the shell asks (`COMPLETE=<shell> quarry -- …`).

use std::ffi::OsStr;

use clap_complete::engine::{CompletionCandidate, PathCompleter, ValueCompleter};

use crate::config::{Config, config_dir, config_path};
use crate::conn::ConnSpec;
use crate::output::TableFormat;

/// The user's config, read without creating one: completion must never write files.
fn config() -> Config {
    std::fs::read_to_string(config_path()).ok().and_then(|t| Config::parse(&t).ok()).unwrap_or_default()
}

fn candidate(value: impl Into<std::ffi::OsString>, help: impl Into<String>) -> CompletionCandidate {
    CompletionCandidate::new(value).help(Some(help.into().into()))
}

fn saved_specs() -> Vec<(String, ConnSpec)> {
    config().connections.into_iter().filter_map(|(name, c)| ConnSpec::parse(&c.url).ok().map(|s| (name, s))).collect()
}

/// TARGET: saved connection names, URL schemes, SQLite files and folders; after `://`, saved URLs.
pub fn target(current: &OsStr) -> Vec<CompletionCandidate> {
    let cur = current.to_string_lossy();
    let config = config();
    if cur.contains("://") {
        return config
            .connections
            .values()
            .map(|c| crate::conn::url::strip_password(&c.url).0)
            .filter(|url| url.starts_with(cur.as_ref()))
            .map(|url| candidate(url, "saved connection"))
            .collect();
    }
    let mut out: Vec<CompletionCandidate> = config
        .connections
        .iter()
        .filter(|(name, _)| name.starts_with(cur.as_ref()))
        .map(|(name, c)| {
            let url = ConnSpec::parse(&c.url).map(|s| s.display_url()).unwrap_or_else(|_| c.url.clone());
            let ro = if c.readonly { " (read-only)" } else { "" };
            candidate(name, format!("{url}{ro}"))
        })
        .collect();
    let schemes = [
        ("postgres://", "PostgreSQL URL"),
        ("mysql://", "MySQL / MariaDB URL"),
        ("sqlite:", "SQLite URL"),
        (":memory:", "in-memory SQLite database"),
    ];
    out.extend(schemes.iter().filter(|(s, _)| s.starts_with(cur.as_ref())).map(|(s, help)| candidate(*s, *help)));
    let sqlite_or_dir =
        PathCompleter::any().filter(|p| p.is_dir() || crate::conn::url::looks_like_sqlite_path(&p.to_string_lossy()));
    out.extend(sqlite_or_dir.complete(current).into_iter().filter(|c| c.get_value() != "."));
    out
}

pub fn saved_names() -> Vec<CompletionCandidate> {
    config().connections.keys().map(|name| candidate(name, "replace this saved connection")).collect()
}

pub fn themes() -> Vec<CompletionCandidate> {
    let builtin = crate::theme::builtin_names();
    crate::theme::list_all(&config_dir().join("themes"))
        .into_iter()
        .map(|name| {
            let help = if builtin.contains(&name.as_str()) { "built-in theme" } else { "your theme" };
            candidate(name, help)
        })
        .collect()
}

pub fn formats() -> Vec<CompletionCandidate> {
    TableFormat::ALL.iter().map(|(name, format)| candidate(*name, format_help(*format))).collect()
}

fn format_help(format: TableFormat) -> &'static str {
    match format {
        TableFormat::Rounded => "table with rounded corners (default)",
        TableFormat::Psql => "like psql",
        TableFormat::Ascii => "+---+ borders, like mysql",
        TableFormat::Unicode => "single-line box drawing",
        TableFormat::Double => "double-line box drawing",
        TableFormat::Minimal => "column gaps and a header rule",
        TableFormat::Plain => "columns separated by spaces",
        TableFormat::Simple => "a ---- rule under the header",
        TableFormat::Markdown => "GitHub Markdown table",
        TableFormat::Csv => "CSV with a header row",
        TableFormat::Tsv => "tab-separated, with a header row",
        TableFormat::Json => "array of JSON objects",
        TableFormat::JsonLines => "one JSON object per line",
        TableFormat::Html => "HTML <table>",
        TableFormat::Vertical => "one column per line",
        TableFormat::SqlInsert => "INSERT statements",
        TableFormat::SqlUpdate => "UPDATE statements",
    }
}

pub fn backends() -> Vec<CompletionCandidate> {
    vec![candidate("postgres", "PostgreSQL"), candidate("mysql", "MySQL / MariaDB"), candidate("sqlite", "SQLite")]
}

pub fn ssl_modes() -> Vec<CompletionCandidate> {
    vec![
        candidate("disable", "no TLS"),
        candidate("prefer", "TLS if the server supports it (default)"),
        candidate("require", "always TLS, certificate not checked"),
        candidate("verify-ca", "TLS with a certificate signed by a trusted CA"),
        candidate("verify-full", "TLS, trusted CA and matching host name"),
    ]
}

pub fn shells() -> Vec<CompletionCandidate> {
    ["bash", "zsh", "fish", "elvish", "powershell"].into_iter().map(|s| candidate(s, "shell")).collect()
}

/// Values used by saved connections for one field, e.g. every host, with the connections using it.
fn from_saved(field: impl Fn(&ConnSpec) -> Option<String>) -> Vec<CompletionCandidate> {
    let mut seen: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (name, spec) in saved_specs() {
        if let Some(v) = field(&spec).filter(|v| !v.is_empty()) {
            seen.entry(v).or_default().push(name);
        }
    }
    seen.into_iter().map(|(v, names)| candidate(v, format!("from {}", names.join(", ")))).collect()
}

pub fn hosts() -> Vec<CompletionCandidate> {
    let mut out = from_saved(|s| s.host.clone());
    for (host, help) in [("localhost", "this machine"), ("127.0.0.1", "this machine over TCP")] {
        if !out.iter().any(|c| c.get_value() == host) {
            out.push(candidate(host, help));
        }
    }
    out
}

pub fn users() -> Vec<CompletionCandidate> {
    from_saved(|s| s.user.clone())
}

pub fn databases() -> Vec<CompletionCandidate> {
    from_saved(|s| s.database.clone())
}

pub fn ports() -> Vec<CompletionCandidate> {
    vec![candidate("5432", "PostgreSQL"), candidate("3306", "MySQL / MariaDB")]
}

/// `--ssh`: host aliases from ~/.ssh/config and tunnels used by saved connections.
pub fn ssh_hosts() -> Vec<CompletionCandidate> {
    let mut out: Vec<CompletionCandidate> = config()
        .connections
        .iter()
        .filter_map(|(name, c)| c.ssh.as_ref().map(|ssh| candidate(ssh, format!("tunnel of {name}"))))
        .collect();
    let ssh_config = dirs::home_dir().and_then(|h| std::fs::read_to_string(h.join(".ssh/config")).ok());
    out.extend(ssh_config_hosts(&ssh_config.unwrap_or_default()).into_iter().map(|h| candidate(h, "~/.ssh/config")));
    out
}

/// `Host` aliases in an OpenSSH config, skipping patterns (`*`, `?`, `!`).
fn ssh_config_hosts(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (key, rest) = l.split_once(|c: char| c.is_whitespace() || c == '=')?;
            key.eq_ignore_ascii_case("host").then_some(rest)
        })
        .flat_map(|rest| rest.split_whitespace().map(String::from).collect::<Vec<_>>())
        .filter(|h| !h.contains(['*', '?', '!']))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_config_aliases_skip_patterns() {
        let text = "Host bastion prod-db\n  HostName 10.0.0.1\nHost *.internal !skip\nhost=staging\n# Host commented\n";
        assert_eq!(ssh_config_hosts(text), vec!["bastion", "prod-db", "staging"]);
    }
}
