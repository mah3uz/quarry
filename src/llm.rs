use std::time::Duration;

use serde_json::{Value as Json, json};

use crate::db::{Backend, Catalog};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const API_VERSION: &str = "2023-06-01";
/// Server-side refusal fallback (`fallbacks: "default"`) is gated by this beta.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const OAUTH_BETA: &str = "oauth-2025-04-20";
/// Keeps the schema part of the prompt bounded for very large databases.
const SCHEMA_BUDGET: usize = 120_000;

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
    pub model: &'a str,
}

enum Auth {
    ApiKey(String),
    Bearer(String),
}

fn auth() -> Result<Auth, String> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    if let Some(k) = get("ANTHROPIC_API_KEY") {
        return Ok(Auth::ApiKey(k));
    }
    if let Some(t) = get("ANTHROPIC_AUTH_TOKEN") {
        return Ok(Auth::Bearer(t));
    }
    Err("set ANTHROPIC_API_KEY (or ANTHROPIC_AUTH_TOKEN) to use \\llm".into())
}

pub fn credentials_available() -> Result<(), String> {
    auth().map(|_| ())
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

pub fn build_body(req: &Request) -> Json {
    json!({
        "model": req.model,
        "max_tokens": 16000,
        "output_config": {"effort": "medium"},
        "fallbacks": "default",
        "system": system_prompt(req),
        "messages": [{"role": "user", "content": req.question}],
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
    if text.trim().is_empty() {
        return Err("Claude returned no text".into());
    }
    let (sql, explanation) = parse_reply(&text);
    let model = resp.get("model").and_then(Json::as_str).unwrap_or_default().to_string();
    Ok(Answer { sql, explanation, model })
}

/// Blocking call to the Messages API. Callers run it off the UI thread.
pub fn ask(req: &Request) -> Result<Answer, String> {
    let auth = auth()?;
    let base = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".into());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(180)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut betas = vec![FALLBACK_BETA];
    let mut request = agent
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
    let mut resp = request.send_json(build_body(req)).map_err(|e| format!("could not reach the Claude API: {e}"))?;
    let status = resp.status();
    let body: Json = resp.body_mut().read_json().map_err(|e| format!("unexpected response ({status}): {e}"))?;
    interpret(&body)
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
        let req = Request { question: "q", backend: Backend::Postgres, server_version: "PostgreSQL 18", catalog: Some(&cat), model: DEFAULT_MODEL };
        let body = build_body(&req);
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert!(body["system"].as_str().unwrap().contains("table public.users(id int PK NOT NULL)"));
    }
}
