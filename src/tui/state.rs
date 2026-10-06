use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::tabs::{Scope, Tab, TabKind};
use super::worker::ConnId;
use crate::repl::editing::has_credentials;

/// Larger editor text is not kept: the state file is rewritten whenever a tab changes.
const MAX_TEXT: usize = 1 << 20;

/// What the TUI remembers between runs.
#[derive(Default, Serialize, Deserialize)]
pub struct UiState {
    pub theme: Option<String>,
    /// The tabs to reopen for a connection, by its saved name or else where it points.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tabs: BTreeMap<String, SavedTabs>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedTabs {
    /// Which of `tabs` was in front.
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedKind {
    #[default]
    Query,
    Table,
    Structure,
}

/// A query tab with its text, or a table or structure tab by the relation it shows (`schema`, `name`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedTab {
    pub kind: SavedKind,
    pub title: String,
    pub text: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: Option<String>,
    /// A table tab's WHERE filter.
    pub filter: Option<String>,
    pub file: Option<PathBuf>,
}

impl SavedTab {
    pub fn scope(&self) -> Option<Scope> {
        self.database.clone().map(Scope::Database).or_else(|| self.schema.clone().map(Scope::Schema))
    }
}

impl SavedTabs {
    /// The tabs of `conn` worth reopening: query tabs with text or a file, and table and structure
    /// tabs. A query tab that spells out a credential is left out, as such a statement is left out
    /// of the history.
    pub fn of(tabs: &[Tab], active: usize, conn: ConnId) -> SavedTabs {
        let mut saved = SavedTabs::default();
        for (i, tab) in tabs.iter().enumerate().filter(|(_, t)| t.conn == Some(conn)) {
            let title = tab.title.clone();
            let relation = |kind, schema: &str, name: &str| SavedTab {
                kind,
                title: title.clone(),
                schema: Some(schema.into()),
                name: Some(name.into()),
                ..Default::default()
            };
            let tab = match &tab.kind {
                TabKind::Query(q) => {
                    let text = q.editor.text();
                    if text.len() > MAX_TEXT || (text.trim().is_empty() && q.file.is_none()) || has_credentials(&text) {
                        continue;
                    }
                    let (database, schema) = match q.scope.clone() {
                        Some(Scope::Database(d)) => (Some(d), None),
                        Some(Scope::Schema(s)) => (None, Some(s)),
                        None => (None, None),
                    };
                    SavedTab { title, text, database, schema, file: q.file.clone(), ..Default::default() }
                }
                TabKind::Table(t) => {
                    SavedTab { filter: Some(t.filter.clone()).filter(|f| !f.is_empty()), ..relation(SavedKind::Table, &t.schema, &t.name) }
                }
                TabKind::Structure(s) => relation(SavedKind::Structure, &s.schema, &s.name),
                _ => continue,
            };
            if i <= active {
                saved.active = saved.tabs.len();
            }
            saved.tabs.push(tab);
        }
        saved
    }
}

fn path() -> PathBuf {
    crate::config::data_dir().join("ui-state.toml")
}

pub fn load() -> UiState {
    std::fs::read_to_string(path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(st: &UiState) {
    if let Ok(s) = toml::to_string(st) {
        let _ = crate::config::write_private(&path(), &s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Backend;
    use crate::tui::tabs::QueryTab;

    fn query_tab(conn: ConnId, title: &str, text: &str) -> Tab {
        let mut q = QueryTab::new(Backend::Postgres);
        q.editor.set_text(text);
        Tab { id: 0, conn: Some(conn), title: title.into(), kind: TabKind::Query(Box::new(q)) }
    }

    #[test]
    fn only_a_connections_own_tabs_with_something_in_them_are_kept() {
        let mut scoped = query_tab(0, "reports", "select 1;\n-- two lines");
        if let TabKind::Query(q) = &mut scoped.kind {
            q.scope = Some(Scope::Schema("reports".into()));
        }
        let tabs = vec![query_tab(0, "Query 1", "  \n"), query_tab(1, "Query 2", "select 'other connection'"), scoped];
        let saved = SavedTabs::of(&tabs, 2, 0);
        assert_eq!(saved.tabs.len(), 1, "an empty tab and another connection's tab are not this connection's work");
        assert_eq!(saved.tabs[0].text, "select 1;\n-- two lines");
        assert_eq!(saved.tabs[0].scope(), Some(Scope::Schema("reports".into())));
        assert_eq!(saved.active, 0, "the tab in front is found again after the empty one is dropped");
        assert!(SavedTabs::of(&tabs[..1], 0, 0).tabs.is_empty());
        let table = Tab { id: 0, conn: Some(0), title: "orders".into(), kind: TabKind::Table(Box::new(crate::tui::tabs::TableTab::new("shop", "orders"))) };
        let saved = SavedTabs::of(&[table], 0, 0);
        assert_eq!((saved.tabs[0].kind, saved.tabs[0].schema.as_deref(), saved.tabs[0].name.as_deref()), (SavedKind::Table, Some("shop"), Some("orders")));
        let secret = [query_tab(0, "Query 1", "alter role app password 'hunter2'")];
        assert!(SavedTabs::of(&secret, 0, 0).tabs.is_empty(), "a password must not end up in the state file");
    }

    #[test]
    fn the_state_file_gives_back_what_was_saved() {
        let tabs = vec![query_tab(0, "Query 1", "select 'it''s';\n\tselect \"x\" -- ünïcödé\n"), query_tab(0, "Query 2", "select 2")];
        let mut st = UiState { theme: Some("nord".into()), ..Default::default() };
        st.tabs.insert("deploy@db.internal/app".into(), SavedTabs::of(&tabs, 1, 0));
        let back: UiState = toml::from_str(&toml::to_string(&st).unwrap()).unwrap();
        assert_eq!(back.theme.as_deref(), Some("nord"));
        assert_eq!(back.tabs, st.tabs);
        assert_eq!(back.tabs["deploy@db.internal/app"].active, 1);
    }

    #[test]
    fn a_state_file_from_before_tabs_were_kept_still_loads() {
        let st: UiState = toml::from_str("theme = \"nord\"\n").unwrap();
        assert_eq!(st.theme.as_deref(), Some("nord"));
        assert!(st.tabs.is_empty());
        let st: UiState = toml::from_str("[[tabs.db.tabs]]\ntitle = \"Query 1\"\ntext = \"select 1\"\n").unwrap();
        assert_eq!(st.tabs["db"].tabs[0].kind, SavedKind::Query, "a tab saved before there were kinds is a query tab");
    }
}
