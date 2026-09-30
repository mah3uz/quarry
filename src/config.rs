use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// `~/.config/quarry/config.toml`. Every field has a default so partial files work.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub main: MainConfig,
    /// Saved connections, used as `quarry <name>` and shown in the TUI connection manager.
    pub connections: BTreeMap<String, SavedConnection>,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MainConfig {
    pub theme: String,
    pub table_format: String,
    /// on | off | auto
    pub expanded: String,
    pub null_string: String,
    pub max_field_width: Option<usize>,
    pub row_limit: usize,
    pub timing: bool,
    pub multi_line: bool,
    pub vi: bool,
    pub smart_completion: bool,
    /// upper | lower | auto
    pub keyword_casing: String,
    pub auto_suggest: bool,
    pub join_suggestions: bool,
    pub show_toolbar: bool,
    pub enable_pager: bool,
    /// Defaults to $PAGER, then `less -SRXF`.
    pub pager: Option<String>,
    pub less_chatty: bool,
    /// Rules for confirmation, e.g. drop, truncate, shutdown, alter, unconditional_update, unconditional_delete.
    pub destructive_warning: Vec<String>,
    /// Prompt format: \u user, \h host, \p port, \d database, \n backend name, \t tx marker, \T time, \x readonly.
    pub prompt: String,
    pub prompt_continuation: String,
    pub history_size: usize,
    pub log_queries: bool,
    pub mouse: bool,
    pub auto_refresh_catalog: bool,
}

impl Default for MainConfig {
    fn default() -> Self {
        MainConfig {
            theme: "tokyo-night".into(),
            table_format: "rounded".into(),
            expanded: "auto".into(),
            null_string: "NULL".into(),
            max_field_width: Some(500),
            row_limit: 1000,
            timing: true,
            multi_line: true,
            vi: false,
            smart_completion: true,
            keyword_casing: "auto".into(),
            auto_suggest: true,
            join_suggestions: true,
            show_toolbar: true,
            enable_pager: true,
            pager: None,
            less_chatty: false,
            destructive_warning: ["drop", "truncate", "shutdown", "unconditional_update", "unconditional_delete"]
                .into_iter()
                .map(String::from)
                .collect(),
            prompt: "\\n \\u@\\h:\\d\\t❯ ".into(),
            prompt_continuation: "… ".into(),
            history_size: 10_000,
            log_queries: false,
            mouse: true,
            auto_refresh_catalog: true,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedConnection {
    /// Any URL accepted by `ConnSpec::parse`. A password here works but prompts a permission warning.
    pub url: String,
    /// Shell command whose stdout is the password (e.g. `pass show db/prod`).
    pub password_command: Option<String>,
    pub ssh: Option<String>,
    pub readonly: bool,
    /// Tag color in the TUI (e.g. `red` for production).
    pub color: Option<String>,
    pub init_commands: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { main: MainConfig::default(), connections: BTreeMap::new(), path: None }
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("QUARRY_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("quarry")
}

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("QUARRY_DATA_DIR") {
        return PathBuf::from(d);
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("quarry")
}

impl Config {
    /// Loads (creating a commented default file on first run). `path` overrides the default location.
    pub fn load(_path: Option<PathBuf>) -> anyhow::Result<Config> {
        todo!()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        todo!()
    }
}
