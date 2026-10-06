use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::tabs::{Scope, Tab, TabKind};
use super::worker::ConnId;

/// Larger editor text is not kept: the state file is rewritten whenever a tab changes.
const MAX_TEXT: usize = 1 << 20;

/// What the TUI remembers between runs.
#[derive(Default, Serialize, Deserialize)]
pub struct UiState {
    pub theme: Option<String>,
    /// The query tabs to reopen for a connection, by its name.
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub title: String,
    pub text: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub file: Option<PathBuf>,
}

impl SavedTab {
    pub fn scope(&self) -> Option<Scope> {
        self.database.clone().map(Scope::Database).or_else(|| self.schema.clone().map(Scope::Schema))
    }
}

impl SavedTabs {
    /// The query tabs of `conn` worth reopening: those with text or a file.
    pub fn of(tabs: &[Tab], active: usize, conn: ConnId) -> SavedTabs {
        let mut saved = SavedTabs::default();
        for (i, tab) in tabs.iter().enumerate() {
            let TabKind::Query(q) = &tab.kind else { continue };
            let text = q.editor.text();
            if tab.conn != Some(conn) || text.len() > MAX_TEXT || (text.trim().is_empty() && q.file.is_none()) {
                continue;
            }
            if i <= active {
                saved.active = saved.tabs.len();
            }
            let (database, schema) = match q.scope.clone() {
                Some(Scope::Database(d)) => (Some(d), None),
                Some(Scope::Schema(s)) => (None, Some(s)),
                None => (None, None),
            };
            saved.tabs.push(SavedTab { title: tab.title.clone(), text, database, schema, file: q.file.clone() });
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
    }
}
