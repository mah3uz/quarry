use std::sync::{Arc, RwLock};

use reedline::{
    Completer as RlCompleter, CompletionResult, History, HistoryItem, HistoryItemId, HistorySessionId,
    SearchQuery, Span, Suggestion as RlSuggestion, ValidationResult, Validator,
};

use super::style::Palette;
use crate::complete::{Completer, SuggestionKind};
use crate::db::Backend;
use crate::special;
use crate::sql::split;

/// Line-editor state that special commands and key toggles change at runtime.
#[derive(Clone)]
pub struct EditState {
    pub backend: Backend,
    pub delimiter: String,
    pub multi_line: bool,
    pub palette: Palette,
    pub completer: Option<Arc<Completer>>,
    pub smart_completion: bool,
}

pub type SharedEdit = Arc<RwLock<EditState>>;

pub struct ReplCompleter {
    pub state: SharedEdit,
}

impl RlCompleter for ReplCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> CompletionResult {
        let st = self.state.read().unwrap();
        // Pending (not an empty Fresh) makes reedline draw nothing instead of "NO RECORDS FOUND",
        // which matters because the menu opens on every keystroke while typing.
        let Some(completer) = st.completer.clone() else {
            return CompletionResult::Pending;
        };
        let p = st.palette.clone();
        drop(st);
        let result = completer.complete(line, pos);
        let start = result.replace_start.min(pos);
        let items: Vec<RlSuggestion> = result
            .items
            .into_iter()
            .map(|s| {
                let color = match s.kind {
                    SuggestionKind::Keyword => p.theme.keyword,
                    SuggestionKind::Table | SuggestionKind::View => p.theme.accent,
                    SuggestionKind::Column => p.theme.identifier,
                    SuggestionKind::Function => p.theme.function,
                    SuggestionKind::DataType => p.theme.datatype,
                    SuggestionKind::Schema | SuggestionKind::Database => p.theme.accent2,
                    SuggestionKind::Join | SuggestionKind::JoinCondition => p.theme.success,
                    SuggestionKind::Special | SuggestionKind::Favorite => p.theme.accent2,
                    SuggestionKind::Alias | SuggestionKind::User => p.theme.parameter,
                    SuggestionKind::File => p.theme.string,
                };
                let label = kind_label(s.kind);
                let description = match &s.detail {
                    Some(d) if !d.is_empty() && d != label => format!("{label} · {d}"),
                    _ => label.to_string(),
                };
                RlSuggestion {
                    value: s.text,
                    display_override: (s.display.as_str() != "").then_some(s.display),
                    description: Some(description),
                    style: Some(p.fg(color)),
                    extra: None,
                    span: Span::new(start, pos),
                    append_whitespace: false,
                    match_indices: None,
                }
            })
            .collect();
        if items.is_empty() {
            return CompletionResult::Pending;
        }
        CompletionResult::fresh(items)
    }
}

fn kind_label(kind: SuggestionKind) -> &'static str {
    match kind {
        SuggestionKind::Keyword => "keyword",
        SuggestionKind::Table => "table",
        SuggestionKind::View => "view",
        SuggestionKind::Column => "column",
        SuggestionKind::Schema => "schema",
        SuggestionKind::Database => "database",
        SuggestionKind::Function => "function",
        SuggestionKind::DataType => "type",
        SuggestionKind::Alias => "alias",
        SuggestionKind::Join => "join",
        SuggestionKind::JoinCondition => "join condition",
        SuggestionKind::Special => "command",
        SuggestionKind::Favorite => "favorite",
        SuggestionKind::File => "file",
        SuggestionKind::User => "user",
    }
}

pub struct ReplValidator {
    pub state: SharedEdit,
}

impl Validator for ReplValidator {
    fn validate(&self, line: &str) -> ValidationResult {
        let st = self.state.read().unwrap();
        if is_complete(line, st.backend, &st.delimiter, st.multi_line) {
            ValidationResult::Complete
        } else {
            ValidationResult::Incomplete
        }
    }
}

pub fn is_complete(line: &str, backend: Backend, delimiter: &str, multi_line: bool) -> bool {
    let t = line.trim();
    if t.is_empty() || !multi_line {
        return true;
    }
    if matches!(t.to_ascii_lowercase().as_str(), "exit" | "quit" | "help" | "\\q" | "\\?") {
        return true;
    }
    if special::submits_immediately(t, backend) {
        return true;
    }
    split::ends_with_terminator(line, backend, delimiter)
}

/// Statements that carry credentials never reach the history file.
pub fn is_sensitive(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    ["identified by", "password", "set password", "encrypted", "secret"].iter().any(|k| lower.contains(k))
}

pub struct SafeHistory<H: History> {
    pub inner: H,
}

impl<H: History> History for SafeHistory<H> {
    fn save(&mut self, h: HistoryItem) -> reedline::Result<HistoryItem> {
        if is_sensitive(&h.command_line) {
            return Ok(h);
        }
        self.inner.save(h)
    }

    fn load(&self, id: HistoryItemId) -> reedline::Result<HistoryItem> {
        self.inner.load(id)
    }

    fn count(&self, query: SearchQuery) -> reedline::Result<i64> {
        self.inner.count(query)
    }

    fn search(&self, query: SearchQuery) -> reedline::Result<Vec<HistoryItem>> {
        self.inner.search(query)
    }

    fn update(
        &mut self,
        id: HistoryItemId,
        updater: &dyn Fn(HistoryItem) -> HistoryItem,
    ) -> reedline::Result<()> {
        self.inner.update(id, updater)
    }

    fn clear(&mut self) -> reedline::Result<()> {
        self.inner.clear()
    }

    fn delete(&mut self, h: HistoryItemId) -> reedline::Result<()> {
        self.inner.delete(h)
    }

    fn sync(&mut self) -> std::io::Result<()> {
        self.inner.sync()
    }

    fn session(&self) -> Option<HistorySessionId> {
        self.inner.session()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_waits_for_terminator_but_commands_submit() {
        assert!(!is_complete("select 1", Backend::Postgres, ";", true));
        assert!(is_complete("select 1;", Backend::Postgres, ";", true));
        assert!(is_complete("select 1", Backend::Postgres, ";", false));
        assert!(is_complete("select 1\\G", Backend::MySql, ";", true));
        assert!(is_complete("quit", Backend::MySql, ";", true));
    }

    #[test]
    fn credentials_are_not_remembered() {
        assert!(is_sensitive("CREATE USER bob IDENTIFIED BY 'x'"));
        assert!(is_sensitive("alter role bob with password 'x'"));
        assert!(!is_sensitive("select * from users"));
    }
}
