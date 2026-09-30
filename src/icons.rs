use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

use crate::complete::SuggestionKind;
use crate::db::Backend;

/// Which glyphs the REPL and TUI draw. `nerd` needs a Nerd Font (v3) in the terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum IconSet {
    #[default]
    Nerd,
    Unicode,
    Ascii,
}

impl IconSet {
    pub fn icons(self) -> &'static Icons {
        match self {
            IconSet::Nerd => &NERD,
            IconSet::Unicode => &UNICODE,
            IconSet::Ascii => &ASCII,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(IconSet::Nerd as u8);

pub fn set(set: IconSet) {
    CURRENT.store(set as u8, Ordering::Relaxed);
}

pub fn current() -> IconSet {
    match CURRENT.load(Ordering::Relaxed) {
        1 => IconSet::Unicode,
        2 => IconSet::Ascii,
        _ => IconSet::Nerd,
    }
}

pub fn get() -> &'static Icons {
    current().icons()
}

/// Every icon is one terminal cell wide (or empty), so callers put a space after it.
#[derive(Debug)]
pub struct Icons {
    pub postgres: &'static str,
    pub mysql: &'static str,
    pub mariadb: &'static str,
    pub sqlite: &'static str,

    pub expanded: &'static str,
    pub collapsed: &'static str,
    pub connection: &'static str,
    pub databases: &'static str,
    /// Tables, Views, Functions… under a schema.
    pub group: &'static str,
    pub database: &'static str,
    pub database_current: &'static str,
    pub schema: &'static str,
    pub table: &'static str,
    pub view: &'static str,
    pub matview: &'static str,
    pub foreign_table: &'static str,
    pub system_table: &'static str,
    pub column: &'static str,
    pub key: &'static str,
    pub function: &'static str,
    pub not_null: &'static str,

    pub query: &'static str,
    pub structure: &'static str,
    pub activity: &'static str,
    pub text: &'static str,
    pub explain: &'static str,
    pub history: &'static str,

    pub ok: &'static str,
    pub error: &'static str,
    pub warning: &'static str,
    pub info: &'static str,
    pub notice: &'static str,
    pub note: &'static str,

    pub logo: &'static str,
    pub dirty: &'static str,
    pub search: &'static str,
    pub filter: &'static str,
    pub sort: &'static str,
    pub asc: &'static str,
    pub desc: &'static str,
    pub more_left: &'static str,
    pub more_right: &'static str,
    pub run: &'static str,
    pub ask: &'static str,
    pub readonly_mark: &'static str,
    pub prompt: &'static str,
    pub prompt_vi: &'static str,
    pub prompt_db: &'static str,

    /// Leading icons for the status badges; empty keeps the badge text alone.
    pub tx: &'static str,
    pub ro: &'static str,
    pub tls: &'static str,
    pub ssh: &'static str,

    pub kinds: Kinds,
}

/// Completion menu markers, one per `SuggestionKind`.
#[derive(Debug)]
pub struct Kinds {
    pub keyword: &'static str,
    pub table: &'static str,
    pub view: &'static str,
    pub column: &'static str,
    pub schema: &'static str,
    pub database: &'static str,
    pub function: &'static str,
    pub datatype: &'static str,
    pub alias: &'static str,
    pub join: &'static str,
    pub command: &'static str,
    pub favorite: &'static str,
    pub file: &'static str,
    pub user: &'static str,
}

impl Icons {
    /// Empty outside the nerd set: there is no Unicode or ASCII logo for a database.
    pub fn backend(&self, backend: Backend, mariadb: bool) -> &'static str {
        match backend {
            Backend::Postgres => self.postgres,
            Backend::MySql if mariadb => self.mariadb,
            Backend::MySql => self.mysql,
            Backend::Sqlite => self.sqlite,
        }
    }

    pub fn connection(&self, backend: Backend, mariadb: bool) -> &'static str {
        match self.backend(backend, mariadb) {
            "" => self.connection,
            b => b,
        }
    }

    pub fn kind(&self, kind: SuggestionKind) -> &'static str {
        let k = &self.kinds;
        match kind {
            SuggestionKind::Keyword => k.keyword,
            SuggestionKind::Table => k.table,
            SuggestionKind::View => k.view,
            SuggestionKind::Column => k.column,
            SuggestionKind::Schema => k.schema,
            SuggestionKind::Database => k.database,
            SuggestionKind::Function => k.function,
            SuggestionKind::DataType => k.datatype,
            SuggestionKind::Alias => k.alias,
            SuggestionKind::Join | SuggestionKind::JoinCondition => k.join,
            SuggestionKind::Special => k.command,
            SuggestionKind::Favorite => k.favorite,
            SuggestionKind::File => k.file,
            SuggestionKind::User => k.user,
        }
    }

    /// ` icon text ` for a status badge, or ` text ` when the set has no icon for it.
    pub fn badge(icon: &str, text: &str) -> String {
        if icon.is_empty() { format!(" {text} ") } else { format!(" {icon} {text} ") }
    }
}

pub static NERD: Icons = Icons {
    postgres: "\u{e76e}",      // dev-postgresql
    mysql: "\u{e704}",         // dev-mysql
    mariadb: "\u{e828}",       // dev-mariadb
    sqlite: "\u{e7c4}",        // dev-sqlite
    expanded: "\u{f0140}",     // md-chevron_down
    collapsed: "\u{f0142}",    // md-chevron_right
    connection: "\u{f048b}",   // md-server
    databases: "\u{f024b}",    // md-folder
    group: "\u{f024b}",        // md-folder
    database: "\u{f1632}",     // md-database_outline
    database_current: "\u{f01bc}", // md-database
    schema: "\u{ea8b}",        // cod-symbol_namespace
    table: "\u{f04eb}",        // md-table
    view: "\u{f1094}",         // md-table_eye
    matview: "\u{f13a0}",      // md-table_refresh
    foreign_table: "\u{f13bd}", // md-table_arrow_right
    system_table: "\u{f13c2}", // md-table_cog
    column: "\u{eb5f}",        // cod-symbol_field
    key: "\u{f030b}",          // md-key_variant
    function: "\u{f0871}",     // md-function_variant
    not_null: "\u{f06c4}",     // md-asterisk
    query: "\u{f0866}",        // md-database_search
    structure: "\u{f04aa}",    // md-sitemap
    activity: "\u{f0430}",     // md-pulse
    text: "\u{f021a}",         // md-text_box
    explain: "\u{f1049}",      // md-graph
    history: "\u{f02da}",      // md-history
    ok: "\u{f012c}",           // md-check
    error: "\u{f0156}",        // md-close
    warning: "\u{f0026}",      // md-alert
    info: "\u{f02fc}",         // md-information
    notice: "\u{f009a}",       // md-bell
    note: "\u{f09df}",         // md-circle_small
    logo: "\u{f08b7}",         // md-pickaxe
    dirty: "\u{f03eb}",        // md-pencil
    search: "\u{f0349}",       // md-magnify
    filter: "\u{f0232}",       // md-filter
    sort: "\u{f04ba}",         // md-sort
    asc: "\u{f0360}",          // md-menu_up
    desc: "\u{f035d}",         // md-menu_down
    more_left: "\u{f0141}",    // md-chevron_left
    more_right: "\u{f0142}",   // md-chevron_right
    run: "\u{f040a}",          // md-play
    ask: "\u{f0674}",          // md-creation
    readonly_mark: "\u{f03ef}", // md-pencil_off
    prompt: "❯",
    prompt_vi: "❮",
    prompt_db: "\u{f01bc}",    // md-database
    tx: "\u{f051f}",           // md-timer_sand
    ro: "\u{f03ef}",           // md-pencil_off
    tls: "\u{f033e}",          // md-lock
    ssh: "\u{f0318}",          // md-lan_connect
    kinds: Kinds {
        keyword: "\u{eb62}",   // cod-symbol_keyword
        table: "\u{f04eb}",    // md-table
        view: "\u{f1094}",     // md-table_eye
        column: "\u{eb5f}",    // cod-symbol_field
        schema: "\u{ea8b}",    // cod-symbol_namespace
        database: "\u{f01bc}", // md-database
        function: "\u{f0871}", // md-function_variant
        datatype: "\u{eb5b}",  // cod-symbol_class
        alias: "\u{ea88}",     // cod-symbol_variable
        join: "\u{f14e0}",     // md-set_merge
        command: "\u{f489}",   // oct-terminal
        favorite: "\u{f04ce}", // md-star
        file: "\u{f0214}",     // md-file
        user: "\u{f0004}",     // md-account
    },
};

pub static UNICODE: Icons = Icons {
    postgres: "",
    mysql: "",
    mariadb: "",
    sqlite: "",
    expanded: "▾",
    collapsed: "▸",
    connection: "◆",
    databases: "⛁",
    group: "",
    database: "○",
    database_current: "●",
    schema: "⬡",
    table: "▦",
    view: "◫",
    matview: "◩",
    foreign_table: "⇢",
    system_table: "▦",
    column: "·",
    key: "⚷",
    function: "ƒ",
    not_null: "✱",
    query: "⌘",
    structure: "⚙",
    activity: "↯",
    text: "≡",
    explain: "⊿",
    history: "↺",
    ok: "✓",
    error: "✗",
    warning: "⚠",
    info: "ℹ",
    notice: "!",
    note: "·",
    logo: "◆",
    dirty: "●",
    search: "⌕",
    filter: "⌕",
    sort: "⇅",
    asc: "▲",
    desc: "▼",
    more_left: "◂",
    more_right: "▸",
    run: "▶",
    ask: "✦",
    readonly_mark: "ʀ",
    prompt: "❯",
    prompt_vi: "❮",
    prompt_db: "▸",
    tx: "",
    ro: "",
    tls: "",
    ssh: "⇄",
    kinds: Kinds {
        keyword: "K",
        table: "T",
        view: "V",
        column: "C",
        schema: "S",
        database: "D",
        function: "ƒ",
        datatype: "τ",
        alias: "A",
        join: "⋈",
        command: "\\",
        favorite: "★",
        file: "/",
        user: "U",
    },
};

pub static ASCII: Icons = Icons {
    postgres: "",
    mysql: "",
    mariadb: "",
    sqlite: "",
    expanded: "-",
    collapsed: "+",
    connection: "@",
    databases: "=",
    group: "",
    database: "o",
    database_current: "*",
    schema: "#",
    table: "T",
    view: "V",
    matview: "M",
    foreign_table: "~",
    system_table: "S",
    column: ".",
    key: "k",
    function: "f",
    not_null: "*",
    query: ">",
    structure: "#",
    activity: "~",
    text: "=",
    explain: "?",
    history: "h",
    ok: "+",
    error: "x",
    warning: "!",
    info: "i",
    notice: "!",
    note: "-",
    logo: "#",
    dirty: "*",
    search: "/",
    filter: "/",
    sort: "^",
    asc: "^",
    desc: "v",
    more_left: "<",
    more_right: ">",
    run: ">",
    ask: "?",
    readonly_mark: "R",
    prompt: ">",
    prompt_vi: "<",
    prompt_db: ">",
    tx: "",
    ro: "",
    tls: "",
    ssh: "",
    kinds: Kinds {
        keyword: "K",
        table: "T",
        view: "V",
        column: "C",
        schema: "S",
        database: "D",
        function: "F",
        datatype: "t",
        alias: "A",
        join: "J",
        command: "\\",
        favorite: "*",
        file: "/",
        user: "U",
    },
};

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::*;

    /// Destructured without `..` so a new icon can't be added without being checked here.
    fn all(icons: &Icons) -> Vec<(&'static str, &'static str)> {
        let Icons {
            postgres, mysql, mariadb, sqlite, expanded, collapsed, connection, databases, group, database,
            database_current, schema, table, view, matview, foreign_table, system_table, column, key,
            function, not_null, query, structure, activity, text, explain, history, ok, error, warning,
            info, notice, note, logo, dirty, search, filter, sort, asc, desc, more_left, more_right, run,
            ask, readonly_mark, prompt, prompt_vi, prompt_db, tx, ro, tls, ssh, kinds,
        } = icons;
        let Kinds {
            keyword, table: k_table, view: k_view, column: k_column, schema: k_schema,
            database: k_database, function: k_function, datatype, alias, join, command, favorite, file, user,
        } = kinds;
        vec![
            ("postgres", postgres), ("mysql", mysql), ("mariadb", mariadb), ("sqlite", sqlite),
            ("expanded", expanded), ("collapsed", collapsed), ("connection", connection),
            ("databases", databases), ("group", group), ("database", database), ("database_current", database_current),
            ("schema", schema), ("table", table), ("view", view), ("matview", matview),
            ("foreign_table", foreign_table), ("system_table", system_table), ("column", column),
            ("key", key), ("function", function), ("not_null", not_null), ("query", query),
            ("structure", structure), ("activity", activity), ("text", text), ("explain", explain),
            ("history", history), ("ok", ok), ("error", error), ("warning", warning), ("info", info),
            ("notice", notice), ("note", note), ("logo", logo), ("dirty", dirty), ("search", search),
            ("filter", filter), ("sort", sort), ("asc", asc), ("desc", desc), ("more_left", more_left),
            ("more_right", more_right), ("run", run), ("ask", ask), ("readonly_mark", readonly_mark),
            ("prompt", prompt), ("prompt_vi", prompt_vi), ("prompt_db", prompt_db), ("tx", tx),
            ("ro", ro), ("tls", tls), ("ssh", ssh), ("kinds.keyword", keyword), ("kinds.table", k_table),
            ("kinds.view", k_view), ("kinds.column", k_column), ("kinds.schema", k_schema),
            ("kinds.database", k_database), ("kinds.function", k_function), ("kinds.datatype", datatype),
            ("kinds.alias", alias), ("kinds.join", join), ("kinds.command", command),
            ("kinds.favorite", favorite), ("kinds.file", file), ("kinds.user", user),
        ]
        .into_iter()
        .map(|(name, icon)| (name, *icon))
        .collect()
    }

    /// Grid headers, tree rows and badges reserve exactly one cell per icon; a wider glyph
    /// would shift every column after it.
    #[test]
    fn every_icon_is_at_most_one_cell_wide() {
        for set in [IconSet::Nerd, IconSet::Unicode, IconSet::Ascii] {
            for (name, icon) in all(set.icons()) {
                assert!(icon.width() <= 1 && icon.chars().count() <= 1, "{set:?}.{name} = {icon:?}");
            }
        }
    }

    /// The fallback for terminals without a Nerd Font or full Unicode coverage.
    #[test]
    fn ascii_set_is_pure_ascii() {
        for (name, icon) in all(&ASCII) {
            assert!(icon.is_ascii(), "ascii.{name} = {icon:?}");
        }
    }

    /// Only the badge icons and backend logos may be blank; a blank tree or status icon
    /// would leave a stray space where the marker should be.
    #[test]
    fn only_badges_and_logos_may_be_blank() {
        let optional = ["postgres", "mysql", "mariadb", "sqlite", "group", "tx", "ro", "tls", "ssh"];
        for set in [IconSet::Nerd, IconSet::Unicode, IconSet::Ascii] {
            for (name, icon) in all(set.icons()) {
                assert!(!icon.is_empty() || optional.contains(&name), "{set:?}.{name} is blank");
            }
        }
        for (name, icon) in all(&NERD) {
            assert!(!icon.is_empty(), "nerd.{name} is blank");
        }
    }

    #[test]
    fn connection_icon_falls_back_when_the_set_has_no_backend_logo() {
        assert_eq!(NERD.connection(Backend::MySql, true), NERD.mariadb);
        assert_eq!(NERD.connection(Backend::MySql, false), NERD.mysql);
        assert_eq!(UNICODE.connection(Backend::Postgres, false), "◆");
    }

    #[test]
    fn badge_drops_the_gap_when_there_is_no_icon() {
        assert_eq!(Icons::badge("", "TX"), " TX ");
        assert_eq!(Icons::badge(NERD.tx, "TX"), format!(" {} TX ", NERD.tx));
    }

    #[test]
    fn config_names_are_lowercase() {
        #[derive(serde::Deserialize)]
        struct W {
            icons: IconSet,
        }
        let w: W = toml::from_str("icons = \"unicode\"").unwrap();
        assert_eq!(w.icons, IconSet::Unicode);
        assert!(toml::from_str::<W>("icons = \"emoji\"").is_err());
    }
}
