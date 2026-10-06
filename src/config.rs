use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

/// `~/.config/quarry/config.toml`. Every field has a default so partial files work.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub main: MainConfig,
    /// How `\llm` reaches a model; `quarry --setup-llm` writes it.
    pub llm: LlmConfig,
    /// TUI key bindings by action name, replacing that action's default keys.
    pub keys: BTreeMap<String, KeyBinding>,
    /// Saved connections, used as `quarry <name>` and shown in the TUI connection manager.
    pub connections: BTreeMap<String, SavedConnection>,
    #[serde(skip)]
    pub path: Option<PathBuf>,
    /// Problems found while loading (insecure permissions, unwritable config dir) to show the user.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MainConfig {
    pub theme: String,
    pub table_format: String,
    /// A line between result rows in the boxed table formats.
    pub row_lines: bool,
    /// Glyphs in the prompt, the TUI and messages: auto, nerd (needs a Nerd Font), unicode or ascii.
    pub icons: crate::icons::IconSet,
    /// The TUI leaves the terminal's own background (e.g. a translucent one) instead of the theme's.
    pub transparent: bool,
    /// on | off | auto
    pub expanded: String,
    pub null_string: String,
    /// `0` in the file means no limit (a missing key means the default).
    #[serde(with = "zero_is_none")]
    pub max_field_width: Option<usize>,
    pub row_limit: usize,
    pub timing: bool,
    pub multi_line: bool,
    pub vi: bool,
    pub smart_completion: bool,
    /// upper | lower | auto
    pub keyword_casing: String,
    pub auto_suggest: bool,
    /// Open the completion menu while typing (Tab always opens it).
    pub complete_while_typing: bool,
    pub join_suggestions: bool,
    pub enable_pager: bool,
    /// Defaults to $PAGER, then `less -SRXF`.
    pub pager: Option<String>,
    pub less_chatty: bool,
    /// Rules for confirmation, e.g. drop, truncate, shutdown, alter, unconditional_update, unconditional_delete.
    pub destructive_warning: Vec<String>,
    /// `auto` (two-line themed prompt) or a format: \u user, \h host, \p port, \d database,
    /// \t product, \n newline, \T transaction marker, \x read-only marker, \D date-time, \R time.
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
            row_lines: true,
            icons: crate::icons::IconSet::Auto,
            transparent: false,
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
            complete_while_typing: true,
            join_suggestions: true,
            enable_pager: true,
            pager: None,
            less_chatty: false,
            destructive_warning: ["drop", "truncate", "shutdown", "unconditional_update", "unconditional_delete"]
                .into_iter()
                .map(String::from)
                .collect(),
            prompt: "auto".into(),
            prompt_continuation: "… ".into(),
            history_size: 10_000,
            log_queries: false,
            mouse: true,
            auto_refresh_catalog: true,
        }
    }
}

/// One key (`"ctrl+enter"`) or several (`["f5", "ctrl+shift+enter"]`); `[]` unbinds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KeyBinding {
    One(String),
    Many(Vec<String>),
}

impl KeyBinding {
    pub fn keys(&self) -> Vec<String> {
        match self {
            KeyBinding::One(k) => vec![k.clone()],
            KeyBinding::Many(ks) => ks.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub provider: crate::llm::Provider,
    /// Empty means the CLI's own default (claude-code, codex).
    pub model: String,
    /// Endpoint for the OpenAI-compatible provider, e.g. `http://localhost:11434/v1`.
    pub base_url: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        LlmConfig { provider: crate::llm::Provider::Anthropic, model: crate::llm::DEFAULT_MODEL.into(), base_url: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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


pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("QUARRY_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    xdg_dir("XDG_CONFIG_HOME", ".config", dirs::config_dir).unwrap_or_else(|| PathBuf::from(".")).join("quarry")
}

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("QUARRY_DATA_DIR") {
        return PathBuf::from(d);
    }
    xdg_dir("XDG_DATA_HOME", ".local/share", dirs::data_dir).unwrap_or_else(|| PathBuf::from(".")).join("quarry")
}

// `dirs` uses `~/Library/Application Support` on macOS; CLI users expect the XDG layout there too.
#[cfg(unix)]
fn xdg_dir(var: &str, default_under_home: &str, _native: fn() -> Option<PathBuf>) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(default_under_home)))
}

#[cfg(not(unix))]
fn xdg_dir(_var: &str, _default_under_home: &str, native: fn() -> Option<PathBuf>) -> Option<PathBuf> {
    native()
}

mod zero_is_none {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<usize>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(v.unwrap_or(0) as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<usize>, D::Error> {
        Ok(Some(usize::deserialize(d)?).filter(|n| *n > 0))
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Writes `contents` atomically (temp file + rename) with 0600 permissions, creating parent dirs.
pub(crate) fn write_private(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let result = (|| {
        let mut f = crate::output::sink::open_private(&tmp, true)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("cannot write {}", path.display()))
}

impl Config {
    /// Loads (creating a commented default file on first run). `path` overrides the default location.
    pub fn load(path: Option<PathBuf>) -> anyhow::Result<Config> {
        let path = path.unwrap_or_else(config_path);
        let mut warnings = Vec::new();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Err(e) = write_private(&path, DEFAULT_CONFIG) {
                    warnings.push(format!("could not create a default config file: {e:#}"));
                }
                DEFAULT_CONFIG.to_string()
            }
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        let mut cfg = Config::parse(&text).map_err(|e| anyhow!("{}: {e}", path.display()))?;
        warnings.extend(cfg.permission_warnings(&path));
        cfg.path = Some(path);
        cfg.warnings = warnings;
        Ok(cfg)
    }

    /// Parses config text; the error message carries line and column.
    pub fn parse(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| e.to_string().trim_end().to_string())
    }

    /// Rewrites the whole file from the in-memory config, so comments in a hand-edited file are
    /// lost; the previous file is kept next to it as `config.toml.bak`.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = self.path.clone().unwrap_or_else(config_path);
        let text = toml::to_string_pretty(self).context("cannot serialize config")?;
        if path.exists() {
            let backup = path.with_extension("toml.bak");
            std::fs::copy(&path, &backup).with_context(|| format!("cannot back up to {}", backup.display()))?;
        }
        write_private(&path, &text)
    }

    fn permission_warnings(&self, path: &Path) -> Vec<String> {
        let with_password: Vec<&str> = self
            .connections
            .iter()
            .filter(|(_, c)| crate::conn::ConnSpec::parse(&c.url).is_ok_and(|s| s.password.is_some()))
            .map(|(name, _)| name.as_str())
            .collect();
        if with_password.is_empty() || !group_or_world_accessible(path) {
            return Vec::new();
        }
        let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        vec![format!(
            "{file} is readable by other users and contains passwords (connections: {}); run `chmod 600 {}` \
             or use password_command instead",
            with_password.join(", "),
            path.display()
        )]
    }

    fn base_dir(&self) -> PathBuf {
        self.path.as_deref().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_else(config_dir)
    }

    pub fn history_path(&self) -> PathBuf {
        data_dir().join("history.txt")
    }

    pub fn favorites_path(&self) -> PathBuf {
        self.base_dir().join("favorites.toml")
    }

    /// Custom themes: `*.toml` palettes and base16/base24 `*.yaml` schemes.
    pub fn themes_dir(&self) -> PathBuf {
        self.base_dir().join("themes")
    }

    pub fn log_path(&self) -> PathBuf {
        data_dir().join("quarry.log")
    }
}

#[cfg(unix)]
pub(crate) fn group_or_world_accessible(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o077 != 0)
}

#[cfg(not(unix))]
pub(crate) fn group_or_world_accessible(_path: &Path) -> bool {
    false
}

pub const DEFAULT_CONFIG: &str = r##"# quarry configuration.
# Every option is optional: delete a line to get the built-in default back.

[main]
# Color theme. Built-in: tokyo-night, tokyo-night-storm, tokyo-night-day, catppuccin-mocha,
# catppuccin-macchiato, catppuccin-frappe, catppuccin-latte, gruvbox-dark, gruvbox-light, dracula,
# nord, one-dark, solarized-dark, solarized-light, rose-pine, rose-pine-moon, rose-pine-dawn,
# kanagawa, everforest-dark, everforest-light, github-dark, github-light, monokai, ayu-dark,
# nightfox, material-ocean, ansi (16 terminal colors).
# Custom themes: put <name>.toml (quarry palette) or base16/base24 <name>.yaml schemes in the
# `themes/` directory next to this file and use <name> here. Switch at runtime with \theme.
theme = "tokyo-night"

# Result table format: rounded, psql, ascii, unicode, double, minimal, plain, simple, markdown,
# csv, tsv, json, jsonl, html, vertical, sql-insert, sql-update. Change at runtime with \T.
table_format = "rounded"

# Draw a line between result rows in the boxed formats (rounded, unicode, double, ascii).
row_lines = true

# Icons in the prompt, the TUI and messages: auto | nerd | unicode | ascii.
# nerd needs a Nerd Font (https://www.nerdfonts.com) in the terminal. auto uses nerd unless
# quarry can tell the glyphs aren't there; if you still see boxes or question marks instead of
# icons, use unicode.
icons = "auto"

# Keep the terminal's own background in the TUI instead of the theme's, e.g. to let a
# translucent terminal show through. Panels and bars go transparent too; selections keep
# their colour. Toggle at runtime from the command palette.
transparent = false

# Expanded (vertical, one column per line) output: on | off | auto.
# auto switches to vertical when a table is wider than the terminal. Toggle with \x.
expanded = "auto"

# Text shown for NULL values.
null_string = "NULL"

# Truncate displayed cell values to this many columns (0 = no limit).
# Machine formats (csv, json, …) are never truncated.
max_field_width = 500

# Row limit for interactive results; quarry asks before showing more.
row_limit = 1000

# Show how long each query took. Toggle with \timing.
timing = true

# Multi-line editing: Enter submits only once the statement ends with ; (or \G, or the delimiter).
# Special commands (\dt, use db, …) always run on Enter.
multi_line = true

# Vi key bindings instead of Emacs.
vi = false

# Context-aware completion (tables after FROM, columns after SELECT, …).
smart_completion = true

# Case of completed keywords: upper | lower | auto (follow what you type).
keyword_casing = "auto"

# Fish-style inline suggestions from history.
auto_suggest = true

# Suggest JOIN clauses and conditions from foreign keys.
join_suggestions = true

# Open the completion menu while typing (Tab always opens it).
complete_while_typing = true

# Page long results.
enable_pager = true

# Pager command. Defaults to $PAGER, then `less -SRXF`.
# pager = "less -SRXF"

# Skip the intro and goodbye messages.
less_chatty = false

# Ask for confirmation before statements matching these rules. A rule is a statement verb in
# lowercase (drop, truncate, alter, delete, update, shutdown, grant, revoke, rename, …) or
# unconditional_update / unconditional_delete (UPDATE / DELETE without a WHERE clause).
# Use [] to never ask.
destructive_warning = ["drop", "truncate", "shutdown", "unconditional_update", "unconditional_delete"]

# Prompt: "auto" draws a two-line themed prompt. Or a format with mycli-style escapes:
#   \u user   \h host   \p port   \d database   \t product (PostgreSQL, MySQL, SQLite)
#   \n newline   \T "*" inside a transaction   \x read-only marker   \D date-time   \R time
# Example: prompt = "\\t \\u@\\h:\\d\\T> "
prompt = "auto"
prompt_continuation = "… "

# Number of history entries to keep.
history_size = 10000

# Append every executed statement to the log file (in the data directory).
log_queries = false

# Mouse support in the TUI.
mouse = true

# Reload completion metadata automatically after CREATE / ALTER / DROP.
auto_refresh_catalog = true

[llm]
# How \llm / \ai reaches a model. `quarry --setup-llm` walks you through it.
#   anthropic     Claude API. Key from ANTHROPIC_API_KEY, or saved by --setup-llm.
#   openai        Any OpenAI-compatible API (OpenAI, OpenRouter, Ollama, LM Studio, …); set base_url.
#   claude-code   Your installed `claude` CLI, using its own login.
#   codex         Your installed `codex` CLI, using its own login.
# API keys saved by --setup-llm go in credentials.toml in the data directory, not in this file.
provider = "anthropic"

# Model name. Empty uses the CLI's own default (claude-code, codex).
model = "claude-opus-5-5"

# Endpoint for the openai provider.
# base_url = "http://localhost:11434/v1"

# TUI key bindings. Each line replaces an action's default keys: one key or a list, [] to unbind.
# Keys look like ctrl+enter, alt+f, shift+f7, f5, ctrl+pagedown or ?. Press F1 in the TUI for the
# list of actions and their current keys.
#
# [keys]
# run_statement = ["ctrl+enter", "ctrl+e"]
# run_all = "f5"
# themes = "alt+t"
# grid_copy = "c"

# Saved connections: `quarry <name>`, `\c <name>`, and the TUI connection manager.
#
# [connections.local]
# url = "postgres://me@localhost:5432/app"      # also mysql://…, sqlite:///path/to/file.db
# password_command = "pass show db/local"       # stdout is used as the password
# ssh = "deploy@bastion:22"                     # tunnel through `ssh -L`
# readonly = false                              # refuse writes (server-side where supported)
# color = "red"                                 # tag color in the TUI, e.g. for production
# init_commands = ["SET search_path TO app, public"]
#
# Passwords written into `url` work, but keep this file private (chmod 600): quarry warns otherwise.
"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("quarry-config-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn documented_default_file_matches_built_in_defaults() {
        assert_eq!(Config::parse(DEFAULT_CONFIG).unwrap(), Config::default());
    }

    #[test]
    fn default_file_documents_every_option() {
        let main = toml::to_string(&MainConfig { pager: Some("x".into()), ..Default::default() }).unwrap();
        let llm = toml::to_string(&LlmConfig { base_url: Some("x".into()), ..Default::default() }).unwrap();
        for text in [main, llm] {
            let table: toml::Table = toml::from_str(&text).unwrap();
            for key in table.keys() {
                assert!(DEFAULT_CONFIG.contains(&format!("{key} =")), "option `{key}` is not documented");
            }
        }
    }

    #[test]
    fn default_file_lists_every_builtin_theme() {
        let listed: String = DEFAULT_CONFIG.lines().filter(|l| l.starts_with('#')).collect::<Vec<_>>().join(" ");
        for name in crate::theme::builtin_names() {
            assert!(
                listed.split(|c: char| !(c.is_alphanumeric() || c == '-')).any(|w| w == *name),
                "theme `{name}` is missing from the list in DEFAULT_CONFIG"
            );
        }
    }

    #[test]
    fn partial_files_keep_other_defaults() {
        let cfg = Config::parse("[main]\ntheme = \"nord\"\n\n[connections.prod]\nurl = \"postgres://h/db\"\nreadonly = true\n")
            .unwrap();
        assert_eq!(cfg.main.theme, "nord");
        assert_eq!(cfg.main.table_format, MainConfig::default().table_format);
        let prod = &cfg.connections["prod"];
        assert!(prod.readonly && prod.init_commands.is_empty());
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn parse_errors_name_the_line() {
        let dir = tmp_dir("bad");
        let path = dir.join("config.toml");
        write_private(&path, "[main]\ntheme = nord\n").unwrap();
        let e = Config::load(Some(path.clone())).unwrap_err().to_string();
        assert!(e.contains(&path.display().to_string()) && e.contains("line 2"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn first_run_creates_commented_default_and_save_round_trips() {
        let dir = tmp_dir("first");
        let path = dir.join("config.toml");
        let mut cfg = Config::load(Some(path.clone())).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
        assert!(cfg.warnings.is_empty());
        assert_eq!(cfg.favorites_path(), dir.join("favorites.toml"));
        assert_eq!(cfg.themes_dir(), dir.join("themes"));

        cfg.connections.insert("x".into(), SavedConnection { url: "sqlite:///tmp/x.db".into(), ..Default::default() });
        cfg.main.max_field_width = None;
        cfg.save().unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("config.toml.bak")).unwrap(), DEFAULT_CONFIG);
        let again = Config::load(Some(path.clone())).unwrap();
        assert_eq!(again.connections, cfg.connections);
        assert_eq!(again.main, cfg.main);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn warns_when_readable_file_contains_passwords() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp_dir("perm");
        let path = dir.join("config.toml");
        write_private(&path, "[connections.prod]\nurl = \"postgres://u:secret@h/db\"\n").unwrap();
        assert!(Config::load(Some(path.clone())).unwrap().warnings.is_empty(), "0600 is fine");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let w = Config::load(Some(path.clone())).unwrap().warnings;
        assert!(w.len() == 1 && w[0].contains("prod") && !w[0].contains("secret"), "{w:?}");
        write_private(&path, "[connections.prod]\nurl = \"postgres://u@h/db\"\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Config::load(Some(path)).unwrap().warnings.is_empty(), "no password, no warning");
        let _ = std::fs::remove_dir_all(dir);
    }
}
