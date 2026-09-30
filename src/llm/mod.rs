pub mod setup;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::config::LlmConfig;
use crate::db::{Backend, Catalog};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const API_VERSION: &str = "2023-06-01";
/// Server-side refusal fallback (`fallbacks: "default"`) is gated by this beta.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const OAUTH_BETA: &str = "oauth-2025-04-20";
/// Keeps the schema part of the prompt bounded for very large databases.
const SCHEMA_BUDGET: usize = 120_000;
const TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    #[default]
    Anthropic,
    Openai,
    ClaudeCode,
    Codex,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic API",
            Provider::Openai => "OpenAI-compatible API",
            Provider::ClaudeCode => "Claude Code",
            Provider::Codex => "Codex",
        }
    }

    pub fn binary(self) -> Option<&'static str> {
        match self {
            Provider::ClaudeCode => Some("claude"),
            Provider::Codex => Some("codex"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// The generated SQL (empty when the model only replied with prose).
    pub sql: String,
    pub explanation: String,
    pub model: String,
}

pub struct Request<'a> {
    pub question: &'a str,
    pub backend: Backend,
    pub server_version: &'a str,
    pub catalog: Option<&'a Catalog>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Auth {
    ApiKey(String),
    Bearer(String),
}

/// API keys saved by `--setup-llm`. They live in the data dir, not next to config.toml, because
/// people back up and share their config directory.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Credentials {
    #[serde(default)]
    api_keys: BTreeMap<String, String>,
}

pub fn credentials_path() -> PathBuf {
    crate::config::data_dir().join("credentials.toml")
}

/// Keys are stored per endpoint so a key saved for one OpenAI-compatible server is never sent to another.
fn key_slot(cfg: &LlmConfig) -> Option<String> {
    match cfg.provider {
        Provider::Anthropic => Some("anthropic".into()),
        Provider::Openai => cfg.base_url.as_deref().map(|u| u.trim_end_matches('/').to_string()),
        Provider::ClaudeCode | Provider::Codex => None,
    }
}

fn read_credentials() -> Result<Credentials, String> {
    let path = credentials_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Credentials::default()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

pub fn saved_key(cfg: &LlmConfig) -> Result<Option<String>, String> {
    let Some(slot) = key_slot(cfg) else { return Ok(None) };
    Ok(read_credentials()?.api_keys.remove(&slot))
}

pub fn save_key(cfg: &LlmConfig, key: &str) -> anyhow::Result<()> {
    let slot = key_slot(cfg).ok_or_else(|| anyhow::anyhow!("{} does not use an API key", cfg.provider.label()))?;
    let mut creds = read_credentials().map_err(anyhow::Error::msg)?;
    creds.api_keys.insert(slot, key.to_string());
    crate::config::write_private(&credentials_path(), &toml::to_string(&creds)?)
}

/// The environment wins over a saved key for Anthropic. OPENAI_API_KEY is deliberately not read:
/// the endpoint is configurable, so it could send an OpenAI key to some other server.
pub fn resolve_auth(cfg: &LlmConfig, env: impl Fn(&str) -> Option<String>, saved: Option<String>) -> Option<Auth> {
    let env = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    match cfg.provider {
        Provider::Anthropic => env("ANTHROPIC_API_KEY")
            .map(Auth::ApiKey)
            .or_else(|| env("ANTHROPIC_AUTH_TOKEN").map(Auth::Bearer))
            .or_else(|| saved.map(Auth::ApiKey)),
        Provider::Openai => saved.map(Auth::Bearer),
        Provider::ClaudeCode | Provider::Codex => None,
    }
}

fn auth(cfg: &LlmConfig) -> Result<Option<Auth>, String> {
    Ok(resolve_auth(cfg, |k| std::env::var(k).ok(), saved_key(cfg)?))
}

/// What the user sees while waiting, e.g. `claude-opus-5-5` or `Claude Code (sonnet)`.
pub fn describe(cfg: &LlmConfig) -> String {
    match (cfg.provider, cfg.model.is_empty()) {
        (Provider::Anthropic | Provider::Openai, _) => cfg.model.clone(),
        (p, true) => p.label().to_string(),
        (p, false) => format!("{} ({})", p.label(), cfg.model),
    }
}

/// Cheap check before asking, so a missing setup is reported without a network round trip.
pub fn check(cfg: &LlmConfig) -> Result<(), String> {
    const SETUP: &str = "run `quarry --setup-llm`";
    match cfg.provider {
        Provider::Anthropic if auth(cfg)?.is_none() => Err(format!("no Anthropic API key: {SETUP} or set ANTHROPIC_API_KEY")),
        Provider::Openai if cfg.base_url.is_none() || cfg.model.is_empty() => {
            Err(format!("the OpenAI-compatible provider needs base_url and model: {SETUP}"))
        }
        p => match p.binary() {
            Some(bin) if which::which(bin).is_err() => Err(format!("`{bin}` is not on PATH: install it or {SETUP}")),
            _ => Ok(()),
        },
    }
}

pub fn schema_context(cat: &Catalog) -> String {
    let mut out = String::new();
    let mut relations: Vec<_> = cat
        .relations()
        .filter(|r| !crate::tui::sidebar::is_system_schema(&r.schema, cat.backend))
        .collect();
    relations.sort_by_key(|r| (!cat.is_on_search_path(&r.schema), r.schema.clone(), r.name.clone()));
    for r in relations {
        let cols: Vec<String> = r
            .columns
            .iter()
            .map(|c| {
                let mut s = format!("{} {}", c.name, c.data_type);
                if c.primary_key {
                    s.push_str(" PK");
                }
                if !c.nullable {
                    s.push_str(" NOT NULL");
                }
                s
            })
            .collect();
        let line = format!("{} {}.{}({})\n", r.kind.label(), r.schema, r.name, cols.join(", "));
        if out.len() + line.len() > SCHEMA_BUDGET {
            out.push_str("… (more tables omitted)\n");
            break;
        }
        out.push_str(&line);
    }
    for fk in &cat.foreign_keys {
        let line = format!(
            "fk {}.{}({}) -> {}.{}({})\n",
            fk.schema,
            fk.table,
            fk.columns.join(", "),
            fk.ref_schema,
            fk.ref_table,
            fk.ref_columns.join(", ")
        );
        if out.len() + line.len() > SCHEMA_BUDGET {
            break;
        }
        out.push_str(&line);
    }
    out
}

fn system_prompt(req: &Request) -> String {
    let mut s = format!(
        "You write SQL for {} ({}). Answer with exactly one fenced ```sql block containing the query, \
         followed by one short sentence explaining it. Use only tables and columns from the schema. \
         Prefer read-only queries; if the user asks to change data, write the statement but never add DROP \
         or TRUNCATE unless explicitly asked.",
        req.backend.name(),
        req.server_version
    );
    if let Some(cat) = req.catalog {
        if let Some(db) = &cat.current_database {
            s.push_str(&format!("\nCurrent database: {db}."));
        }
        if !cat.search_path.is_empty() {
            s.push_str(&format!("\nSearch path: {}.", cat.search_path.join(", ")));
        }
        s.push_str("\n\nSchema:\n");
        s.push_str(&schema_context(cat));
    }
    s
}

pub fn build_body(req: &Request, model: &str) -> Json {
    json!({
        "model": model,
        "max_tokens": 16000,
        "output_config": {"effort": "medium"},
        "fallbacks": "default",
        "system": system_prompt(req),
        "messages": [{"role": "user", "content": req.question}],
    })
}

pub fn build_openai_body(req: &Request, model: &str) -> Json {
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt(req)},
            {"role": "user", "content": req.question},
        ],
    })
}

/// Splits the reply into the SQL inside the first fenced block and the surrounding prose.
pub fn parse_reply(text: &str) -> (String, String) {
    let Some(open) = text.find("```") else {
        return (String::new(), text.trim().to_string());
    };
    let after_fence = &text[open + 3..];
    let body_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
    let body = &after_fence[body_start..];
    let (sql, rest) = match body.find("```") {
        Some(close) => (&body[..close], &body[close + 3..]),
        None => (body, ""),
    };
    let prose = format!("{} {}", text[..open].trim(), rest.trim()).trim().to_string();
    (sql.trim().to_string(), prose)
}

fn answer(text: &str, model: String) -> Result<Answer, String> {
    if text.trim().is_empty() {
        return Err("the model returned no text".into());
    }
    let (sql, explanation) = parse_reply(text);
    Ok(Answer { sql, explanation, model })
}

pub fn interpret(resp: &Json) -> Result<Answer, String> {
    if resp.get("type").and_then(Json::as_str) == Some("error") {
        let msg = resp.pointer("/error/message").and_then(Json::as_str).unwrap_or("unknown error");
        return Err(format!("Claude API error: {msg}"));
    }
    if resp.get("stop_reason").and_then(Json::as_str) == Some("refusal") {
        let why = resp.pointer("/stop_details/explanation").and_then(Json::as_str).unwrap_or("");
        return Err(format!("Claude declined this request. {why}").trim().to_string());
    }
    let text: String = resp
        .get("content")
        .and_then(Json::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Json::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Json::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let model = resp.get("model").and_then(Json::as_str).unwrap_or_default().to_string();
    answer(&text, model)
}

pub fn interpret_openai(resp: &Json) -> Result<Answer, String> {
    if let Some(err) = resp.get("error") {
        // Ollama and some proxies send `"error": "text"` instead of an object.
        let msg = err.get("message").and_then(Json::as_str).or(err.as_str()).unwrap_or("unknown error");
        return Err(format!("API error: {msg}"));
    }
    let text = resp.pointer("/choices/0/message/content").and_then(Json::as_str).unwrap_or_default();
    let model = resp.get("model").and_then(Json::as_str).unwrap_or_default().to_string();
    answer(text, model)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build().into()
}

fn read_json(mut resp: ureq::http::Response<ureq::Body>) -> Result<Json, String> {
    let status = resp.status();
    resp.body_mut().read_json().map_err(|e| format!("unexpected response ({status}): {e}"))
}

/// Blocking; callers run it off the UI thread.
pub fn ask(cfg: &LlmConfig, req: &Request) -> Result<Answer, String> {
    ask_with(cfg, req, auth(cfg)?)
}

pub fn ask_with(cfg: &LlmConfig, req: &Request, auth: Option<Auth>) -> Result<Answer, String> {
    match cfg.provider {
        Provider::Anthropic => ask_anthropic(cfg, req, auth),
        Provider::Openai => ask_openai(cfg, req, auth),
        Provider::ClaudeCode | Provider::Codex => {
            let text = run_cli(&cli_argv(cfg), &format!("{}\n\nRequest: {}", system_prompt(req), req.question))?;
            answer(&text, describe(cfg))
        }
    }
}

fn ask_anthropic(cfg: &LlmConfig, req: &Request, auth: Option<Auth>) -> Result<Answer, String> {
    let auth = auth.ok_or("no Anthropic API key: run `quarry --setup-llm` or set ANTHROPIC_API_KEY")?;
    let base = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".into());
    let mut betas = vec![FALLBACK_BETA];
    let mut request = agent()
        .post(format!("{}/v1/messages", base.trim_end_matches('/')))
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json");
    request = match &auth {
        Auth::ApiKey(k) => request.header("x-api-key", k),
        Auth::Bearer(t) => {
            betas.push(OAUTH_BETA);
            request.header("authorization", format!("Bearer {t}"))
        }
    };
    request = request.header("anthropic-beta", betas.join(","));
    let resp = request.send_json(build_body(req, &cfg.model)).map_err(|e| format!("could not reach the Claude API: {e}"))?;
    interpret(&read_json(resp)?)
}

fn ask_openai(cfg: &LlmConfig, req: &Request, auth: Option<Auth>) -> Result<Answer, String> {
    let base = cfg.base_url.as_deref().ok_or("no base_url configured: run `quarry --setup-llm`")?;
    let mut request = agent()
        .post(format!("{}/chat/completions", base.trim_end_matches('/')))
        .header("content-type", "application/json");
    if let Some(Auth::Bearer(k) | Auth::ApiKey(k)) = &auth {
        request = request.header("authorization", format!("Bearer {k}"));
    }
    let resp = request.send_json(build_openai_body(req, &cfg.model)).map_err(|e| format!("could not reach {base}: {e}"))?;
    interpret_openai(&read_json(resp)?)
}

/// Model ids an OpenAI-compatible server offers (`GET /models`), for the setup wizard.
pub fn list_openai_models(base_url: &str, key: Option<&str>) -> Result<Vec<String>, String> {
    let mut request = agent().get(format!("{}/models", base_url.trim_end_matches('/')));
    if let Some(k) = key {
        request = request.header("authorization", format!("Bearer {k}"));
    }
    let resp = request.call().map_err(|e| format!("could not reach {base_url}: {e}"))?;
    let status = resp.status();
    let body = read_json(resp)?;
    if !status.is_success() {
        return Err(interpret_openai(&body).err().unwrap_or_else(|| format!("HTTP {status}")));
    }
    let mut ids: Vec<String> = body
        .get("data")
        .and_then(Json::as_array)
        .map(|models| models.iter().filter_map(|m| m.get("id").and_then(Json::as_str)).map(String::from).collect())
        .unwrap_or_default();
    ids.sort();
    Ok(ids)
}

/// The CLIs get no tools and a read-only sandbox: they only need to write text, never touch files.
pub fn cli_argv(cfg: &LlmConfig) -> Vec<String> {
    let mut argv: Vec<&str> = match cfg.provider {
        Provider::ClaudeCode => vec!["claude", "-p", "--tools", "", "--no-session-persistence", "--output-format", "text"],
        Provider::Codex => vec!["codex", "exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--color", "never"],
        Provider::Anthropic | Provider::Openai => unreachable!("not a CLI provider"),
    };
    if !cfg.model.is_empty() {
        argv.extend([if cfg.provider == Provider::Codex { "-m" } else { "--model" }, cfg.model.as_str()]);
    }
    if cfg.provider == Provider::Codex {
        argv.push("-");
    }
    argv.into_iter().map(String::from).collect()
}

/// Runs a CLI with the prompt on stdin (a large schema would exceed argv limits). It runs in the
/// temp dir so the CLI does not pick up instructions (CLAUDE.md, AGENTS.md) from the user's project.
fn run_cli(argv: &[String], input: &str) -> Result<String, String> {
    let bin = &argv[0];
    let mut child = Command::new(bin)
        .args(&argv[1..])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run `{bin}`: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let input = input.to_string();
    std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let drain = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let stdout = drain(Box::new(child.stdout.take().expect("piped stdout")));
    let stderr = drain(Box::new(child.stderr.take().expect("piped stderr")));
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait().map_err(|e| format!("`{bin}`: {e}"))? {
            Some(status) => break status,
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("`{bin}` did not answer within {}s", TIMEOUT.as_secs()));
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let out = stdout.join().unwrap_or_default();
    if !status.success() {
        let err = stderr.join().unwrap_or_default();
        let tail: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).rev().take(3).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!("`{bin}` failed ({status}): {}", tail.join(" / ")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ColumnInfo, RelKind, Relation, SchemaInfo};

    #[test]
    fn reply_is_split_into_sql_and_explanation() {
        let (sql, prose) = parse_reply("Here you go:\n```sql\nSELECT 1;\n```\nCounts nothing.");
        assert_eq!(sql, "SELECT 1;");
        assert_eq!(prose, "Here you go: Counts nothing.");
        assert_eq!(parse_reply("no code").0, "");
    }

    #[test]
    fn refusals_and_errors_never_yield_sql() {
        let refusal = json!({"type": "message", "stop_reason": "refusal", "stop_details": {"explanation": "policy"}, "content": []});
        assert!(interpret(&refusal).unwrap_err().contains("declined"));
        let err = json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}});
        assert!(interpret(&err).unwrap_err().contains("invalid x-api-key"));
    }

    #[test]
    fn answer_takes_text_blocks_only() {
        let ok = json!({
            "type": "message", "model": "claude-opus-5-5", "stop_reason": "end_turn",
            "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": "```sql\nselect * from users\n```\nAll users."}]
        });
        let a = interpret(&ok).unwrap();
        assert_eq!(a.sql, "select * from users");
        assert_eq!(a.explanation, "All users.");
    }

    #[test]
    fn body_opts_into_fallbacks_and_carries_schema() {
        let mut cat = Catalog::empty(Backend::Postgres);
        cat.search_path = vec!["public".into()];
        cat.schemas = vec![SchemaInfo {
            name: "public".into(),
            relations: vec![Relation {
                schema: "public".into(),
                name: "users".into(),
                kind: RelKind::Table,
                columns: vec![ColumnInfo {
                    name: "id".into(),
                    data_type: "int".into(),
                    nullable: false,
                    default: None,
                    primary_key: true,
                    auto: true,
                    comment: None,
                }],
                comment: None,
                row_estimate: None,
            }],
            functions: vec![],
            types: vec![],
        }];
        let req = Request { question: "q", backend: Backend::Postgres, server_version: "PostgreSQL 18", catalog: Some(&cat) };
        let body = build_body(&req, DEFAULT_MODEL);
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert!(body["system"].as_str().unwrap().contains("table public.users(id int PK NOT NULL)"));
    }

    fn cfg(provider: Provider, model: &str, base_url: Option<&str>) -> LlmConfig {
        LlmConfig { provider, model: model.into(), base_url: base_url.map(String::from) }
    }

    #[test]
    fn environment_key_beats_saved_key_for_anthropic() {
        let c = cfg(Provider::Anthropic, DEFAULT_MODEL, None);
        let env = |k: &str| (k == "ANTHROPIC_API_KEY").then(|| "env-key".to_string());
        assert_eq!(resolve_auth(&c, env, Some("saved".into())), Some(Auth::ApiKey("env-key".into())));
        assert_eq!(resolve_auth(&c, |_| None, Some("saved".into())), Some(Auth::ApiKey("saved".into())));
        assert_eq!(resolve_auth(&c, |_| Some("  ".into()), None), None, "a blank env var is not a key");
    }

    #[test]
    fn openai_compatible_endpoints_never_receive_openai_api_key_from_env() {
        let c = cfg(Provider::Openai, "m", Some("https://openrouter.ai/api/v1"));
        let env = |_: &str| Some("sk-openai-secret".to_string());
        assert_eq!(resolve_auth(&c, env, None), None);
        assert_eq!(resolve_auth(&c, env, Some("or-key".into())), Some(Auth::Bearer("or-key".into())));
    }

    #[test]
    fn saved_keys_are_scoped_to_their_endpoint() {
        let a = cfg(Provider::Openai, "m", Some("https://openrouter.ai/api/v1/"));
        let b = cfg(Provider::Openai, "m", Some("http://localhost:11434/v1"));
        assert_eq!(key_slot(&a).as_deref(), Some("https://openrouter.ai/api/v1"));
        assert_ne!(key_slot(&a), key_slot(&b), "a key typed for one server must not be sent to another");
        assert_eq!(key_slot(&cfg(Provider::ClaudeCode, "", None)), None);
    }

    #[test]
    fn cli_providers_run_without_tools_or_write_access() {
        let claude = cli_argv(&cfg(Provider::ClaudeCode, "sonnet", None));
        let tools = claude.iter().position(|a| a == "--tools").expect("--tools");
        assert_eq!(claude[tools + 1], "", "claude must get an empty tool list");
        assert!(claude.windows(2).any(|w| w == ["--model", "sonnet"]));
        let codex = cli_argv(&cfg(Provider::Codex, "", None));
        assert!(codex.windows(2).any(|w| w == ["--sandbox", "read-only"]));
        assert!(!codex.iter().any(|a| a == "-m"), "empty model leaves Codex on its default");
        assert_eq!(codex.last().map(String::as_str), Some("-"), "prompt comes from stdin");
    }

    #[test]
    fn openai_replies_and_errors() {
        let ok = json!({"model": "llama3", "choices": [{"message": {"role": "assistant", "content": "```sql\nselect 1\n```\nOne."}}]});
        let a = interpret_openai(&ok).unwrap();
        assert_eq!((a.sql.as_str(), a.model.as_str()), ("select 1", "llama3"));
        let err = json!({"error": {"message": "invalid api key", "type": "invalid_request_error"}});
        assert!(interpret_openai(&err).unwrap_err().contains("invalid api key"));
        let ollama = json!({"error": "model 'x' not found"});
        assert!(interpret_openai(&ollama).unwrap_err().contains("not found"));
    }

    #[test]
    fn openai_body_carries_schema_as_system_message() {
        let req = Request { question: "q", backend: Backend::Sqlite, server_version: "3.46", catalog: None };
        let body = build_openai_body(&req, "gpt");
        assert_eq!(body["messages"][0]["role"], "system");
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("SQLite"));
        assert_eq!(body["messages"][1]["content"], "q");
    }
}
