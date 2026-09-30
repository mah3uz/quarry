use std::io::{ErrorKind, IsTerminal};

use anyhow::{Result, bail};
use cliclack::{confirm, input, intro, log, outro, outro_cancel, password, select, spinner};
use nu_ansi_term::{Color, Style};

use super::{Auth, Provider, Request};
use crate::config::{Config, LlmConfig};
use crate::db::Backend;

const OTHER: &str = "\u{0}other";

const ANTHROPIC_MODELS: &[(&str, &str, &str)] = &[
    ("claude-opus-5-5", "Claude Opus 5.5", "recommended"),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5", "faster, cheaper"),
    ("claude-haiku-4-5", "Claude Haiku 4.5", "fastest, cheapest"),
];

const CLAUDE_CODE_MODELS: &[(&str, &str, &str)] = &[
    ("", "Claude Code's default", "whatever `claude` uses"),
    ("opus", "Opus", "latest Opus"),
    ("sonnet", "Sonnet", "latest Sonnet"),
    ("haiku", "Haiku", "latest Haiku"),
];

const ENDPOINTS: &[(&str, &str, &str)] = &[
    ("https://api.openai.com/v1", "OpenAI", "api.openai.com"),
    ("https://openrouter.ai/api/v1", "OpenRouter", "openrouter.ai"),
    ("http://localhost:11434/v1", "Ollama", "local, no key needed"),
    ("http://localhost:1234/v1", "LM Studio", "local, no key needed"),
];

/// `quarry --setup-llm`: asks how `\llm` should reach a model, tests it, and saves the answers.
pub fn run(config: &mut Config) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        bail!("--setup-llm is interactive; run it in a terminal (or edit the [llm] section of the config file)");
    }
    match wizard(config) {
        // cliclack has already printed "Operation cancelled."
        Err(e) if e.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == ErrorKind::Interrupted) => Ok(()),
        other => other,
    }
}

fn wizard(config: &mut Config) -> Result<()> {
    intro(Style::new().on(Color::Cyan).fg(Color::Black).paint(" quarry · LLM setup "))?;
    let current = &config.llm;

    let provider = select("How should \\llm reach a model?")
        .initial_value(current.provider)
        .item(Provider::Anthropic, "Anthropic API", "Claude with your API key")
        .item(Provider::ClaudeCode, "Claude Code", cli_hint("claude"))
        .item(Provider::Openai, "OpenAI-compatible API", "OpenAI, OpenRouter, Ollama, LM Studio, …")
        .item(Provider::Codex, "Codex", cli_hint("codex"))
        .interact()?;
    let same = provider == current.provider;
    let mut llm = LlmConfig { provider, model: String::new(), base_url: None };

    let mut new_key = None;
    match provider {
        Provider::Anthropic => {
            new_key = ask_key(&llm, "Anthropic API key", false)?;
            llm.model = pick(
                "Which model?",
                ANTHROPIC_MODELS,
                if same { &current.model } else { super::DEFAULT_MODEL },
                "Model id",
            )?;
        }
        Provider::Openai => {
            llm.base_url = Some(pick_endpoint(current.base_url.as_deref().filter(|_| same))?);
            new_key = ask_key(&llm, "API key", true)?;
            llm.model = pick_openai_model(&llm, new_key.as_deref(), if same { &current.model } else { "" })?;
        }
        Provider::ClaudeCode | Provider::Codex => {
            let bin = provider.binary().unwrap_or_default();
            if which::which(bin).is_err() {
                log::warning(format!("`{bin}` is not on your PATH. Install it and log in before using \\llm."))?;
            } else {
                let limits = if provider == Provider::Codex { "in a read-only sandbox" } else { "with no tools" };
                log::info(format!("quarry runs `{bin}` {limits}, so it cannot change your files. It uses {bin}'s own login."))?;
            }
            llm.model = if provider == Provider::ClaudeCode {
                pick("Which model?", CLAUDE_CODE_MODELS, if same { &current.model } else { "" }, "Model name or alias")?
            } else {
                input("Model")
                    .placeholder("leave empty for Codex's default")
                    .default_input(if same { &current.model } else { "" })
                    .required(false)
                    .interact()?
            };
        }
    }

    if !test(&llm, new_key.as_deref())? && !confirm("Save these settings anyway?").initial_value(false).interact()? {
        outro_cancel("Nothing was changed.")?;
        return Ok(());
    }

    if let Some(key) = &new_key {
        super::save_key(&llm, key)?;
        log::success(format!("API key saved to {}", super::credentials_path().display()))?;
    }
    config.llm = llm;
    config.save()?;
    let path = config.path.clone().unwrap_or_else(crate::config::config_path);
    outro(format!("Saved to {}. Try `\\llm top 10 customers by revenue` in quarry.", path.display()))?;
    Ok(())
}

fn cli_hint(bin: &str) -> String {
    match which::which(bin) {
        Ok(_) => format!("runs your `{bin}` CLI with its own login"),
        Err(_) => format!("`{bin}` not found on PATH"),
    }
}

/// Returns a newly typed key, or None to keep using the environment / the saved key.
fn ask_key(llm: &LlmConfig, prompt: &str, optional: bool) -> Result<Option<String>> {
    let saved = super::saved_key(llm).map_err(anyhow::Error::msg)?;
    let from_env = llm.provider == Provider::Anthropic
        && std::env::var("ANTHROPIC_API_KEY").is_ok_and(|v| !v.trim().is_empty());
    let keep = if from_env {
        log::info("ANTHROPIC_API_KEY is set in your environment; it is used before any saved key.")?;
        Some("press Enter to keep using ANTHROPIC_API_KEY")
    } else if saved.is_some() {
        Some("press Enter to keep the saved key")
    } else if optional {
        Some("press Enter if the server needs no key")
    } else {
        None
    };
    let prompt = match keep {
        Some(hint) => format!("{prompt} ({hint})"),
        None => prompt.to_string(),
    };
    let mut p = password(prompt).mask('▪');
    if keep.is_some() {
        p = p.allow_empty();
    }
    let key = p.interact()?;
    Ok(Some(key.trim().to_string()).filter(|k| !k.is_empty()))
}

fn pick(prompt: &str, options: &[(&str, &str, &str)], current: &str, other_prompt: &str) -> Result<String> {
    let known = options.iter().any(|(v, ..)| *v == current);
    let mut s = select(prompt).initial_value(if known { current } else { OTHER });
    for (value, label, hint) in options {
        s = s.item(*value, *label, *hint);
    }
    let custom_hint = if known { String::new() } else { current.to_string() };
    let choice = s.item(OTHER, "Other…", custom_hint).interact()?;
    if choice != OTHER {
        return Ok(choice.to_string());
    }
    Ok(input(other_prompt).default_input(if known { "" } else { current }).interact()?)
}

fn pick_endpoint(current: Option<&str>) -> Result<String> {
    let current = current.unwrap_or(ENDPOINTS[0].0);
    let url = pick("Which server?", ENDPOINTS, current, "Base URL (ends before /chat/completions)")?;
    if url.starts_with("http://") || url.starts_with("https://") {
        return Ok(url.trim_end_matches('/').to_string());
    }
    log::warning("A base URL starts with http:// or https://.")?;
    pick_endpoint(None)
}

fn pick_openai_model(llm: &LlmConfig, new_key: Option<&str>, current: &str) -> Result<String> {
    let base = llm.base_url.as_deref().unwrap_or_default();
    let saved = super::saved_key(llm).map_err(anyhow::Error::msg)?;
    let key = new_key.map(String::from).or(saved);
    let sp = spinner();
    sp.start(format!("Fetching models from {base}"));
    match super::list_openai_models(base, key.as_deref()) {
        Ok(ids) if !ids.is_empty() => {
            sp.stop(format!("{} models available", ids.len()));
            let initial = if ids.iter().any(|m| m == current) { current } else { ids[0].as_str() };
            let mut s = select("Which model? (type to filter)").filter_mode().max_rows(12).initial_value(initial);
            for id in &ids {
                s = s.item(id.as_str(), id, "");
            }
            Ok(s.interact()?.to_string())
        }
        Ok(_) => {
            sp.stop("The server listed no models");
            Ok(input("Model").default_input(current).interact()?)
        }
        Err(e) => {
            sp.error(format!("Could not list models: {e}"));
            Ok(input("Model").default_input(current).interact()?)
        }
    }
}

/// Sends a tiny request so a bad key, model or login shows up now rather than on first use.
fn test(llm: &LlmConfig, new_key: Option<&str>) -> Result<bool> {
    let auth = match new_key {
        Some(k) if llm.provider == Provider::Anthropic => Some(Auth::ApiKey(k.to_string())),
        Some(k) => Some(Auth::Bearer(k.to_string())),
        None => super::auth(llm).map_err(anyhow::Error::msg)?,
    };
    let req = Request {
        question: "Return the current date.",
        backend: Backend::Postgres,
        server_version: "PostgreSQL",
        catalog: None,
    };
    let sp = spinner();
    sp.start(format!("Testing {}", super::describe(llm)));
    match super::ask_with(llm, &req, auth) {
        Ok(a) if !a.sql.is_empty() => {
            sp.stop(format!("It works: {}", a.sql.lines().next().unwrap_or_default()));
            Ok(true)
        }
        Ok(_) => {
            sp.error("It answered, but without SQL. A different model may work better.");
            Ok(false)
        }
        Err(e) => {
            sp.error(format!("Test failed: {e}"));
            Ok(false)
        }
    }
}
