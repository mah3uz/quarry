use std::sync::Arc;

use crate::db::{Backend, Catalog};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SuggestionKind {
    Keyword,
    Table,
    View,
    Column,
    Schema,
    Database,
    Function,
    DataType,
    Alias,
    Join,
    JoinCondition,
    Special,
    Favorite,
    File,
    User,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// Text inserted in place of `replace_start..cursor`.
    pub text: String,
    /// Label shown in the menu (usually == text).
    pub display: String,
    pub kind: SuggestionKind,
    /// Right-hand detail: column type, function signature, table kind, command help…
    pub detail: Option<String>,
    /// Higher is better; items are returned sorted.
    pub score: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Completions {
    /// Byte offset in the input where the word being completed starts.
    pub replace_start: usize,
    pub items: Vec<Suggestion>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeywordCasing {
    Upper,
    Lower,
    /// Match the case the user typed.
    #[default]
    Auto,
}

#[derive(Clone, Debug)]
pub struct CompleteOptions {
    pub keyword_casing: KeywordCasing,
    /// false → keywords + every name, no context analysis (pgcli `smart_completion = False`).
    pub smart: bool,
    /// Suggest `JOIN b ON a.id = b.a_id` style completions from foreign keys.
    pub join_suggestions: bool,
    pub generate_aliases: bool,
    pub max_items: usize,
}

impl Default for CompleteOptions {
    fn default() -> Self {
        CompleteOptions { keyword_casing: KeywordCasing::Auto, smart: true, join_suggestions: true, generate_aliases: false, max_items: 200 }
    }
}

/// Extra, non-catalog candidates (special commands, favorite queries).
#[derive(Clone, Debug, Default)]
pub struct Extras {
    /// (command, description) e.g. ("\\dt", "List tables").
    pub specials: Vec<(String, String)>,
    pub favorites: Vec<String>,
}

pub struct Completer {
    pub backend: Backend,
    pub catalog: Arc<Catalog>,
    pub options: CompleteOptions,
    pub extras: Extras,
}

impl Completer {
    pub fn new(backend: Backend, catalog: Arc<Catalog>, options: CompleteOptions, extras: Extras) -> Self {
        Completer { backend, catalog, options, extras }
    }

    /// Context-aware completion for `text` with the cursor at byte offset `cursor`.
    /// `text` may contain several statements; only the one under the cursor matters.
    pub fn complete(&self, _text: &str, _cursor: usize) -> Completions {
        todo!()
    }
}
