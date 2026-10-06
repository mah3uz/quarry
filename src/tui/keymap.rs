//! Key bindings: the app-level actions and the panes' own keys, their defaults, overrides from
//! `[keys]` in the config, and lookup.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::KeyBinding;

/// A key with modifiers, normalized so that `?` and `Shift+?` or `G` and `Shift+g` compare equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    code: KeyCode,
    mods: KeyModifiers,
}

impl Key {
    fn new(code: KeyCode, mods: KeyModifiers) -> Key {
        let mods = mods & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        match code {
            // Shift is already in the character (and some terminals report it, others don't)
            KeyCode::Char(c) if mods.contains(KeyModifiers::SHIFT) && !c.is_ascii_lowercase() => {
                Key { code, mods: mods - KeyModifiers::SHIFT }
            }
            KeyCode::Char(c) if mods.contains(KeyModifiers::SHIFT) => {
                Key { code: KeyCode::Char(c.to_ascii_uppercase()), mods: mods - KeyModifiers::SHIFT }
            }
            // Ctrl+letter arrives as lowercase; keep bindings case-insensitive there
            KeyCode::Char(c) if mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                Key { code: KeyCode::Char(c.to_ascii_lowercase()), mods }
            }
            KeyCode::BackTab => Key { code: KeyCode::Tab, mods: mods | KeyModifiers::SHIFT },
            _ => Key { code, mods },
        }
    }

    pub fn from_event(e: &KeyEvent) -> Key {
        Key::new(e.code, e.modifiers)
    }

    /// The key press a pane would have received for this key.
    fn to_event(self) -> KeyEvent {
        match self.code {
            KeyCode::Tab if self.mods.contains(KeyModifiers::SHIFT) => KeyEvent::new(KeyCode::BackTab, self.mods),
            code => KeyEvent::new(code, self.mods),
        }
    }

    /// `ctrl+enter`, `alt+f`, `f5`, `shift+f7`, `?`, `ctrl+pagedown`: modifiers joined with `+`.
    pub fn parse(spec: &str) -> Result<Key, String> {
        let spec = spec.trim();
        let bad = || format!("unknown key '{spec}'");
        let mut mods = KeyModifiers::NONE;
        let parts: Vec<&str> = if spec.ends_with("++") || spec == "+" {
            let mut p: Vec<&str> = spec.trim_end_matches('+').split('+').filter(|s| !s.is_empty()).collect();
            p.push("+");
            p
        } else {
            spec.split('+').collect()
        };
        let (last, modifiers) = parts.split_last().ok_or_else(bad)?;
        for m in modifiers {
            mods |= match m.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "c" => KeyModifiers::CONTROL,
                "alt" | "meta" | "option" | "m" => KeyModifiers::ALT,
                "shift" | "s" => KeyModifiers::SHIFT,
                _ => return Err(bad()),
            };
        }
        let name = last.trim();
        let lower = name.to_ascii_lowercase();
        let code = match lower.as_str() {
            "enter" | "return" | "cr" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "space" => KeyCode::Char(' '),
            "backspace" | "bs" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "insert" | "ins" => KeyCode::Insert,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" | "pgdown" => KeyCode::PageDown,
            f if f.len() >= 2 && f.starts_with('f') && f[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
                KeyCode::F(f[1..].parse().unwrap_or(1))
            }
            _ if name.chars().count() == 1 => KeyCode::Char(name.chars().next().unwrap_or(' ')),
            _ => return Err(bad()),
        };
        Ok(Key::new(code, mods))
    }

    /// For hints and help: `Ctrl+Enter`, `Shift+F7`, `?`.
    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            s.push_str("Ctrl+");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            s.push_str("Alt+");
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            s.push_str("Shift+");
        }
        match self.code {
            KeyCode::Char(' ') => s.push_str("Space"),
            KeyCode::Char(c) if self.mods.is_empty() => s.push(c),
            KeyCode::Char(c) => s.push(c.to_ascii_uppercase()),
            KeyCode::F(n) => s.push_str(&format!("F{n}")),
            KeyCode::Enter => s.push_str("Enter"),
            KeyCode::Esc => s.push_str("Esc"),
            KeyCode::Tab => s.push_str("Tab"),
            KeyCode::Backspace => s.push_str("Backspace"),
            KeyCode::Delete => s.push_str("Delete"),
            KeyCode::Insert => s.push_str("Insert"),
            KeyCode::Up => s.push('↑'),
            KeyCode::Down => s.push('↓'),
            KeyCode::Left => s.push('←'),
            KeyCode::Right => s.push('→'),
            KeyCode::Home => s.push_str("Home"),
            KeyCode::End => s.push_str("End"),
            KeyCode::PageUp => s.push_str("PgUp"),
            KeyCode::PageDown => s.push_str("PgDn"),
            other => s.push_str(&format!("{other:?}")),
        }
        s
    }

    /// Alt+1…9 always jump to a tab.
    fn is_tab_jump(&self) -> bool {
        self.mods == KeyModifiers::ALT && matches!(self.code, KeyCode::Char('1'..='9'))
    }

    /// A plain character, which belongs to whatever text field has focus.
    pub fn is_plain_char(&self) -> bool {
        matches!(self.code, KeyCode::Char(_)) && !self.mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    }

}

/// The pane whose own keys an action belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Editor,
    Grid,
    Table,
    Explorer,
}

impl Scope {
    pub fn section(self) -> &'static str {
        match self {
            Scope::Editor => "Editor",
            Scope::Grid => "Results grid",
            Scope::Table => "Table view",
            Scope::Explorer => "Explorer",
        }
    }

    /// A table view is a grid too, so the two can't give one key two meanings.
    fn overlaps(self, other: Scope) -> bool {
        self == other || matches!((self, other), (Scope::Grid, Scope::Table) | (Scope::Table, Scope::Grid))
    }
}

/// Something a pane does on a key of its own. The pane knows it by its first default key, so
/// another key bound to it is handed to the pane as that one.
pub struct PaneAction {
    /// The name used in `[keys]`.
    pub name: &'static str,
    pub scope: Scope,
    pub description: &'static str,
    defaults: &'static [&'static str],
}

const fn pane(name: &'static str, scope: Scope, description: &'static str, defaults: &'static [&'static str]) -> PaneAction {
    PaneAction { name, scope, description, defaults }
}

pub const PANE_ACTIONS: &[PaneAction] = &[
    pane("editor_complete", Scope::Editor, "Completion (it also opens as you type)", &["ctrl+space"]),
    pane("editor_select_all", Scope::Editor, "Select all", &["ctrl+a"]),
    pane("editor_copy", Scope::Editor, "Copy the selection", &["ctrl+c"]),
    pane("editor_cut", Scope::Editor, "Cut the selection", &["ctrl+x"]),
    pane("editor_paste", Scope::Editor, "Paste", &["ctrl+v"]),
    pane("editor_undo", Scope::Editor, "Undo", &["ctrl+z"]),
    pane("editor_redo", Scope::Editor, "Redo", &["ctrl+y", "ctrl+shift+z"]),
    pane("editor_duplicate", Scope::Editor, "Duplicate the line or selection", &["ctrl+d"]),
    pane("editor_comment", Scope::Editor, "Comment or uncomment", &["ctrl+/", "ctrl+7"]),
    pane("editor_line_up", Scope::Editor, "Move the line up", &["alt+up"]),
    pane("editor_line_down", Scope::Editor, "Move the line down", &["alt+down"]),
    pane("editor_delete_word_left", Scope::Editor, "Delete the word before the cursor", &["ctrl+backspace", "ctrl+h"]),
    pane("editor_delete_word_right", Scope::Editor, "Delete the word after the cursor", &["ctrl+delete"]),
    pane("grid_up", Scope::Grid, "Up", &["k", "up"]),
    pane("grid_down", Scope::Grid, "Down", &["j", "down"]),
    pane("grid_left", Scope::Grid, "Left", &["h", "left"]),
    pane("grid_right", Scope::Grid, "Right", &["l", "right"]),
    pane("grid_page_up", Scope::Grid, "Page up", &["pageup"]),
    pane("grid_page_down", Scope::Grid, "Page down", &["pagedown"]),
    pane("grid_half_page_up", Scope::Grid, "Half a page up", &["ctrl+u"]),
    pane("grid_half_page_down", Scope::Grid, "Half a page down", &["ctrl+d"]),
    pane("grid_first_row", Scope::Grid, "First row", &["g"]),
    pane("grid_last_row", Scope::Grid, "Last row", &["G"]),
    pane("grid_first_column", Scope::Grid, "First column", &["0", "home"]),
    pane("grid_last_column", Scope::Grid, "Last column", &["$", "end"]),
    pane("grid_columns_right", Scope::Grid, "A screen of columns right", &["w"]),
    pane("grid_columns_left", Scope::Grid, "A screen of columns left", &["b"]),
    pane("grid_next_cell", Scope::Grid, "Next cell", &["tab"]),
    pane("grid_prev_cell", Scope::Grid, "Previous cell", &["shift+tab"]),
    pane("grid_select", Scope::Grid, "Select a block of cells", &["v"]),
    pane("grid_select_rows", Scope::Grid, "Select whole rows", &["V"]),
    pane("grid_view_cell", Scope::Grid, "View the cell and its row", &["enter"]),
    pane("grid_copy", Scope::Grid, "Copy cells as TSV", &["y"]),
    pane("grid_copy_rows", Scope::Grid, "Copy rows with a header", &["Y"]),
    pane("grid_search", Scope::Grid, "Search in the results", &["/"]),
    pane("grid_search_next", Scope::Grid, "Next match", &["n"]),
    pane("grid_search_prev", Scope::Grid, "Previous match", &["N"]),
    pane("grid_narrow_column", Scope::Grid, "Narrow the column", &["<"]),
    pane("grid_widen_column", Scope::Grid, "Widen the column", &[">"]),
    pane("grid_fit_column", Scope::Grid, "Fit the column to its content", &["="]),
    pane("grid_prev_result", Scope::Grid, "Previous result set", &["["]),
    pane("grid_next_result", Scope::Grid, "Next result set", &["]"]),
    pane("grid_messages", Scope::Grid, "Messages", &["m"]),
    pane("grid_to_editor", Scope::Grid, "Back to the editor", &["i"]),
    pane("table_filter", Scope::Table, "Filter with a WHERE condition", &["f"]),
    pane("table_filter_by_value", Scope::Table, "Filter by the current cell's value", &["F"]),
    pane("table_sort", Scope::Table, "Sort by the column", &["s"]),
    pane("table_edit_cell", Scope::Table, "Edit the cell", &["e", "f2"]),
    pane("table_add_row", Scope::Table, "Add a row", &["o"]),
    pane("table_delete_rows", Scope::Table, "Mark rows for deletion", &["D", "delete"]),
    pane("table_apply", Scope::Table, "Review and apply staged changes", &["ctrl+s"]),
    pane("table_discard", Scope::Table, "Discard staged changes", &["u"]),
    pane("table_reload", Scope::Table, "Reload", &["r", "f5"]),
    pane("explorer_up", Scope::Explorer, "Up", &["k", "up"]),
    pane("explorer_down", Scope::Explorer, "Down", &["j", "down"]),
    pane("explorer_expand", Scope::Explorer, "Expand", &["l", "right"]),
    pane("explorer_collapse", Scope::Explorer, "Collapse, or go to the parent", &["h", "left"]),
    pane("explorer_toggle", Scope::Explorer, "Expand or collapse", &["space"]),
    pane("explorer_last", Scope::Explorer, "Last row", &["G", "end"]),
    pane("explorer_open", Scope::Explorer, "Open a table, show a function, insert a column, switch database", &["enter"]),
    pane("explorer_console", Scope::Explorer, "New query tab for this database or schema", &["c"]),
    pane("explorer_structure", Scope::Explorer, "Table structure", &["s"]),
    pane("explorer_insert_name", Scope::Explorer, "Insert the name into the editor", &["i"]),
    pane("explorer_script", Scope::Explorer, "Write a statement for the table, with s i u d c x n next", &["g"]),
    pane("explorer_filter", Scope::Explorer, "Filter the tree", &["/"]),
    pane("explorer_reload", Scope::Explorer, "Reload the schema", &["r", "f5"]),
    pane("explorer_new_connection", Scope::Explorer, "New connection", &["n"]),
    pane("explorer_disconnect", Scope::Explorer, "Disconnect", &["ctrl+x"]),
];

macro_rules! actions {
    ($($variant:ident, $name:literal, $section:literal, $desc:literal, [$($key:literal),*];)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Action {
            $($variant,)*
        }

        impl Action {
            pub const ALL: &[Action] = &[$(Action::$variant,)*];

            /// The name used in `[keys]`.
            pub fn name(self) -> &'static str {
                match self { $(Action::$variant => $name,)* }
            }

            pub fn section(self) -> &'static str {
                match self { $(Action::$variant => $section,)* }
            }

            pub fn description(self) -> &'static str {
                match self { $(Action::$variant => $desc,)* }
            }

            fn defaults(self) -> &'static [&'static str] {
                match self { $(Action::$variant => &[$($key),*],)* }
            }
        }
    };
}

actions! {
    Commands, "commands", "Global", "Command palette", ["ctrl+p"];
    Help, "help", "Global", "Keyboard shortcuts (this list)", ["f1", "?"];
    Connections, "connections", "Global", "Connections", ["ctrl+o"];
    NewQuery, "new_query", "Global", "New query tab (for the database selected in the explorer)", ["ctrl+t"];
    CloseTab, "close_tab", "Global", "Close tab", ["ctrl+w"];
    NextTab, "next_tab", "Global", "Next tab", ["alt+right", "ctrl+pagedown"];
    PrevTab, "prev_tab", "Global", "Previous tab", ["alt+left", "ctrl+pageup"];
    NextPane, "next_pane", "Global", "Focus next pane: explorer, editor, results", ["f6"];
    PrevPane, "prev_pane", "Global", "Focus previous pane", ["shift+f6"];
    FocusExplorer, "focus_explorer", "Global", "Focus the explorer", ["alt+0"];
    ToggleExplorer, "toggle_explorer", "Global", "Show or hide the explorer", ["ctrl+b"];
    GoToTable, "go_to_table", "Global", "Go to table", ["ctrl+g"];
    Themes, "themes", "Global", "Switch theme (live preview)", ["ctrl+y"];
    History, "history", "Global", "Query history", ["ctrl+r"];
    Quit, "quit", "Global", "Quit", ["ctrl+q"];
    RunStatement, "run_statement", "Query", "Run the statement under the cursor, or the selection", ["ctrl+enter", "alt+enter", "ctrl+e"];
    RunAll, "run_all", "Query", "Run everything in the editor", ["f5", "ctrl+shift+enter"];
    Cancel, "cancel", "Query", "Cancel the running query", ["esc", "ctrl+c"];
    Explain, "explain", "Query", "Explain", ["f7"];
    ExplainAnalyze, "explain_analyze", "Query", "Explain analyze", ["shift+f7"];
    FormatSql, "format_sql", "Query", "Format SQL", ["alt+f"];
    SaveFavorite, "save_favorite", "Query", "Save the query as a favorite", ["ctrl+s"];
    Export, "export", "Query", "Export results to a file", ["ctrl+x"];
    EditorSmaller, "editor_smaller", "Query", "Make the editor smaller", ["ctrl+up"];
    EditorLarger, "editor_larger", "Query", "Make the editor larger", ["ctrl+down"];
}

/// Keys of the panes' own actions, by index into `PANE_ACTIONS`.
type PaneKeys = Vec<(Key, usize)>;

pub struct Keymap {
    bindings: Vec<(Key, Action)>,
    pane: PaneKeys,
    /// Default pane keys whose action was given other keys, so they do nothing now.
    freed: Vec<(Key, Scope)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::new(&BTreeMap::new()).0
    }
}

impl Keymap {
    /// Defaults with `[keys]` overrides applied. An override replaces all of an action's keys, and a
    /// key you bind is taken away from any action that had it by default, so nothing is shadowed.
    /// Conflicts and keys that can't work come back as warnings.
    pub fn new(overrides: &BTreeMap<String, KeyBinding>) -> (Keymap, Vec<String>) {
        let mut warnings = Vec::new();
        for name in overrides.keys() {
            if !Action::ALL.iter().any(|a| a.name() == name) && !PANE_ACTIONS.iter().any(|a| a.name == name) {
                warnings.push(format!("[keys] {name}: no such action (F1 lists them)"));
            }
        }
        let (pane, freed) = pane_bindings(overrides, &mut warnings);
        let editor_uses = |key: &Key| pane.iter().any(|(k, i)| k == key && PANE_ACTIONS[*i].scope == Scope::Editor);
        let mut bindings: Vec<(Key, Action)> = Vec::new();
        for &action in Action::ALL {
            let Some(b) = overrides.get(action.name()) else { continue };
            for spec in b.keys().iter().filter(|s| !s.trim().is_empty()) {
                let key = match Key::parse(spec) {
                    Ok(k) => k,
                    Err(e) => {
                        warnings.push(format!("[keys] {}: {e}", action.name()));
                        continue;
                    }
                };
                if key.is_tab_jump() {
                    warnings.push(format!("[keys] {}: {} always jumps to a tab", action.name(), key.label()));
                    continue;
                }
                if let Some((_, other)) = bindings.iter().find(|(k, _)| *k == key) {
                    if *other != action {
                        warnings.push(format!("[keys] {} is bound to both {} and {}; {} keeps it", key.label(), other.name(), action.name(), other.name()));
                    }
                    continue;
                }
                if editor_uses(&key) {
                    warnings.push(format!("[keys] {}: {} is the editor's own shortcut there, so it works outside the editor only", action.name(), key.label()));
                } else if key.is_plain_char() {
                    warnings.push(format!("[keys] {}: {} works only when no text field has focus", action.name(), key.label()));
                }
                bindings.push((key, action));
            }
        }
        for &action in Action::ALL {
            if overrides.contains_key(action.name()) {
                continue;
            }
            let mut lost = Vec::new();
            let mut kept = 0;
            for spec in action.defaults() {
                let key = Key::parse(spec).expect("default keys parse");
                match bindings.iter().find(|(k, _)| *k == key) {
                    Some((_, taker)) => lost.push((key, *taker)),
                    None => {
                        bindings.push((key, action));
                        kept += 1;
                    }
                }
            }
            if kept == 0 && let Some((key, taker)) = lost.first() {
                warnings.push(format!(
                    "[keys] {} now has no key: {} runs {} instead; give it one under [keys]",
                    action.name(),
                    key.label(),
                    taker.name()
                ));
            }
        }
        for (key, i) in pane.iter().filter(|(_, i)| overrides.contains_key(PANE_ACTIONS[*i].name)) {
            let a = &PANE_ACTIONS[*i];
            let global = bindings.iter().find(|(k, action)| k == key && action.section() == "Global");
            if let Some((_, taker)) = global.filter(|_| a.scope != Scope::Editor) {
                warnings.push(format!("[keys] {}: {} runs {} everywhere, so it never reaches the pane", a.name, key.label(), taker.name()));
            }
        }
        (Keymap { bindings, pane, freed }, warnings)
    }

    /// The action bound to `key`. While typing (`typing`), plain characters are text, and in the
    /// SQL editor (`in_editor`) the editor's own shortcuts win.
    pub fn action(&self, key: &KeyEvent, typing: bool, in_editor: bool) -> Option<Action> {
        let k = Key::from_event(key);
        if typing && k.is_plain_char() || in_editor && self.pane_action(&[Scope::Editor], k).is_some() {
            return None;
        }
        self.bindings.iter().find(|(b, _)| *b == k).map(|(_, a)| *a)
    }

    fn pane_action(&self, scopes: &[Scope], key: Key) -> Option<&'static PaneAction> {
        self.pane.iter().find(|(k, i)| *k == key && scopes.contains(&PANE_ACTIONS[*i].scope)).map(|(_, i)| &PANE_ACTIONS[*i])
    }

    /// The key press to hand to a pane for `key`: the pane's own key for the action `key` is bound
    /// to, `key` itself when nothing is bound, or `None` for a default key that was given up.
    /// While typing, plain characters are text.
    pub fn pane_key(&self, scopes: &[Scope], key: KeyEvent, typing: bool) -> Option<KeyEvent> {
        let k = Key::from_event(&key);
        if typing && k.is_plain_char() {
            return Some(key);
        }
        match self.pane_action(scopes, k) {
            Some(a) if a.defaults.iter().any(|d| Key::parse(d) == Ok(k)) => Some(key),
            Some(a) => Key::parse(a.defaults[0]).ok().map(Key::to_event),
            None if self.freed.iter().any(|(f, s)| *f == k && scopes.contains(s)) => None,
            None => Some(key),
        }
    }

    /// The first key of the pane action called `name`, for hints; empty when it has none.
    pub fn pane_short(&self, name: &str) -> String {
        self.pane.iter().find(|(_, i)| PANE_ACTIONS[*i].name == name).map(|(k, _)| k.label()).unwrap_or_default()
    }

    /// `key what` for each action that has a key, e.g. `y copy  / search`.
    pub fn hints(&self, actions: &[(&str, &str)]) -> String {
        let hint = |(name, what): &(&str, &str)| Some(self.pane_short(name)).filter(|k| !k.is_empty()).map(|k| format!("{k} {what}"));
        actions.iter().filter_map(hint).collect::<Vec<_>>().join("  ")
    }

    /// A pane action's keys for display, e.g. `J · Down`; empty when unbound.
    pub fn pane_label(&self, action: &PaneAction) -> String {
        let keys = self.pane.iter().filter(|(_, i)| PANE_ACTIONS[*i].name == action.name).map(|(k, _)| k.label());
        keys.collect::<Vec<_>>().join(" · ")
    }

    pub fn keys(&self, action: Action) -> Vec<Key> {
        self.bindings.iter().filter(|(_, a)| *a == action).map(|(k, _)| *k).collect()
    }

    /// All of an action's keys for display, e.g. `Ctrl+Enter · Alt+Enter`; empty when unbound.
    pub fn label(&self, action: Action) -> String {
        self.keys(action).iter().map(Key::label).collect::<Vec<_>>().join(" · ")
    }

    /// The first key only, for tight spaces like the status bar.
    pub fn short(&self, action: Action) -> String {
        self.keys(action).first().map(Key::label).unwrap_or_default()
    }
}

/// The panes' keys with `[keys]` overrides applied, and the default keys those overrides gave up.
/// As with app actions, an override replaces all of an action's keys and takes a key away from the
/// action in the same pane that had it by default.
fn pane_bindings(overrides: &BTreeMap<String, KeyBinding>, warnings: &mut Vec<String>) -> (PaneKeys, Vec<(Key, Scope)>) {
    let mut pane = PaneKeys::new();
    let holder = |pane: &[(Key, usize)], key: Key, scope: Scope| {
        pane.iter().find(|(k, i)| *k == key && PANE_ACTIONS[*i].scope.overlaps(scope)).map(|(_, i)| PANE_ACTIONS[*i].name)
    };
    for (i, a) in PANE_ACTIONS.iter().enumerate() {
        let Some(b) = overrides.get(a.name) else { continue };
        for spec in b.keys().iter().filter(|s| !s.trim().is_empty()) {
            match Key::parse(spec) {
                Err(e) => warnings.push(format!("[keys] {}: {e}", a.name)),
                Ok(key) if key.is_tab_jump() => warnings.push(format!("[keys] {}: {} always jumps to a tab", a.name, key.label())),
                Ok(key) => match holder(&pane, key, a.scope) {
                    Some(other) if other != a.name => {
                        warnings.push(format!("[keys] {} is bound to both {other} and {}; {other} keeps it", key.label(), a.name));
                    }
                    Some(_) => {}
                    None => {
                        if a.scope == Scope::Editor && key.is_plain_char() {
                            warnings.push(format!("[keys] {}: {} is a character you type in the editor, so it can't be a shortcut there", a.name, key.label()));
                            continue;
                        }
                        pane.push((key, i));
                    }
                },
            }
        }
    }
    let mut freed = Vec::new();
    for (i, a) in PANE_ACTIONS.iter().enumerate() {
        let overridden = overrides.contains_key(a.name);
        for spec in a.defaults {
            let key = Key::parse(spec).expect("default keys parse");
            if overridden {
                if holder(&pane, key, a.scope).is_none() {
                    freed.push((key, a.scope));
                }
            } else if holder(&pane, key, a.scope).is_none() {
                pane.push((key, i));
            }
        }
    }
    freed.retain(|(key, scope)| holder(&pane, *key, *scope).is_none());
    for (i, a) in PANE_ACTIONS.iter().enumerate() {
        if !overrides.contains_key(a.name) && !pane.iter().any(|(_, held)| *held == i) {
            warnings.push(format!("[keys] {} now has no key: another action took it; give it one under [keys]", a.name));
        }
    }
    (pane, freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn ch(c: char) -> KeyEvent {
        ev(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn with(overrides: &[(&str, &str)]) -> (Keymap, Vec<String>) {
        Keymap::new(&overrides.iter().map(|(a, k)| (a.to_string(), KeyBinding::One(k.to_string()))).collect())
    }

    /// A pane is told which action to run by being handed that action's first default key, so
    /// two actions of one pane can never share a default.
    #[test]
    fn pane_defaults_are_unambiguous_and_reach_the_pane_untouched() {
        let (map, warnings) = with(&[]);
        assert_eq!(warnings, Vec::<String>::new());
        for (i, a) in PANE_ACTIONS.iter().enumerate() {
            for spec in a.defaults {
                let key = Key::parse(spec).unwrap_or_else(|e| panic!("{}: {e}", a.name));
                let clash = PANE_ACTIONS.iter().enumerate().find(|(j, b)| {
                    *j != i && b.scope.overlaps(a.scope) && b.defaults.iter().any(|d| Key::parse(d) == Ok(key))
                });
                assert!(clash.is_none(), "{} and {} both default to {spec}", a.name, clash.unwrap().1.name);
                assert_eq!(map.pane_key(&[a.scope], key.to_event(), false), Some(key.to_event()), "{}", a.name);
            }
        }
        let shift_down = ev(KeyCode::Down, KeyModifiers::SHIFT);
        assert_eq!(map.pane_key(&[Scope::Grid], shift_down, false), Some(shift_down), "keys no action names pass through");
    }

    #[test]
    fn a_rebound_pane_action_runs_on_its_new_key_only() {
        let (map, warnings) = with(&[("grid_copy", "c"), ("grid_prev_cell", "ctrl+k")]);
        assert_eq!(warnings, Vec::<String>::new());
        assert_eq!(map.pane_key(&[Scope::Grid], ch('c'), false), Some(ch('y')), "the grid knows copy as y");
        assert_eq!(map.pane_key(&[Scope::Grid], ch('y'), false), None, "the old key no longer copies");
        assert_eq!(map.pane_key(&[Scope::Table, Scope::Grid], ch('c'), false), Some(ch('y')), "a table view is a grid too");
        assert_eq!(map.pane_key(&[Scope::Explorer], ch('c'), false), Some(ch('c')), "the explorer's own c is another pane's business");
        let back = map.pane_key(&[Scope::Grid], ev(KeyCode::Char('k'), KeyModifiers::CONTROL), false);
        assert_eq!(back.map(|k| k.code), Some(KeyCode::BackTab), "Shift+Tab reaches a pane as BackTab");
        assert_eq!(map.pane_label(PANE_ACTIONS.iter().find(|a| a.name == "grid_copy").unwrap()), "c");

        let (_, warnings) = with(&[("grid_copy", "ctrl+b")]);
        assert!(warnings.iter().any(|w| w.contains("toggle_explorer everywhere")), "an app-wide key never reaches a pane: {warnings:?}");
    }

    #[test]
    fn hints_name_the_keys_that_work_now() {
        let shown = [("grid_copy", "copy"), ("grid_search", "search"), ("grid_messages", "messages")];
        assert_eq!(with(&[]).0.hints(&shown), "y copy  / search  m messages");
        let mut overrides: BTreeMap<String, KeyBinding> = [("grid_copy".to_string(), KeyBinding::One("c".into()))].into();
        overrides.insert("grid_search".into(), KeyBinding::Many(Vec::new()));
        assert_eq!(Keymap::new(&overrides).0.hints(&shown), "c copy  m messages", "an action with no key has no hint");
    }

    #[test]
    fn a_pane_key_given_to_another_action_is_taken_from_its_old_one() {
        let (map, warnings) = with(&[("grid_down", "n")]);
        assert_eq!(map.pane_key(&[Scope::Grid], ch('n'), false), Some(ch('j')));
        assert_eq!(map.pane_key(&[Scope::Grid], ch('j'), false), None);
        assert!(warnings.iter().any(|w| w.contains("grid_search_next now has no key")), "{warnings:?}");
        assert_eq!(map.pane_key(&[Scope::Explorer], ch('n'), false), Some(ch('n')), "other panes keep their n");
    }

    #[test]
    fn text_being_typed_is_never_a_pane_shortcut() {
        let (map, warnings) = with(&[("explorer_filter", "f"), ("editor_copy", "c")]);
        assert_eq!(map.pane_key(&[Scope::Explorer], ch('f'), true), Some(ch('f')), "typing into the tree filter");
        assert_eq!(map.pane_key(&[Scope::Explorer], ch('f'), false), Some(ch('/')));
        assert!(warnings.iter().any(|w| w.contains("editor_copy") && w.contains("character you type")), "{warnings:?}");
        let ctrl_c = ev(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map.pane_key(&[Scope::Editor], ctrl_c, true), None, "an override that names no usable key still unbinds");
    }

    #[test]
    fn the_editors_keys_win_over_app_actions_while_it_has_focus() {
        let ctrl = |c| ev(KeyCode::Char(c), KeyModifiers::CONTROL);
        let (map, _) = with(&[]);
        assert_eq!(map.action(&ctrl('y'), true, true), None, "Ctrl+Y is redo in the editor");
        assert_eq!(map.action(&ctrl('y'), false, false), Some(Action::Themes));
        let (map, _) = with(&[("editor_redo", "ctrl+shift+z")]);
        assert_eq!(map.action(&ctrl('y'), true, true), Some(Action::Themes), "once redo lets go of it, the app action has it");
    }

    #[test]
    fn key_specs_parse_and_print_back() {
        for (spec, label) in [
            ("ctrl+enter", "Ctrl+Enter"),
            ("Ctrl+Shift+Enter", "Ctrl+Shift+Enter"),
            ("alt+f", "Alt+F"),
            ("f5", "F5"),
            ("shift+f7", "Shift+F7"),
            ("?", "?"),
            ("ctrl+pagedown", "Ctrl+PgDn"),
            ("alt+0", "Alt+0"),
            ("ctrl++", "Ctrl++"),
            ("G", "G"),
        ] {
            assert_eq!(Key::parse(spec).map(|k| k.label()), Ok(label.to_string()), "{spec}");
        }
        assert!(Key::parse("hyper+x").is_err());
        assert!(Key::parse("ctrl+nosuchkey").is_err());
    }

    /// Terminals disagree on whether Shift comes with `?` or `G`; a binding must match either way.
    #[test]
    fn events_match_bindings_whatever_the_terminal_reports_for_shift() {
        let (map, _) = Keymap::new(&BTreeMap::new());
        let q = ev(KeyCode::Char('?'), KeyModifiers::NONE);
        let shifted = ev(KeyCode::Char('?'), KeyModifiers::SHIFT);
        assert_eq!(map.action(&q, false, false), Some(Action::Help));
        assert_eq!(map.action(&shifted, false, false), Some(Action::Help));
        assert_eq!(Key::parse("G"), Ok(Key::from_event(&ev(KeyCode::Char('g'), KeyModifiers::SHIFT))));
        assert_eq!(Key::parse("ctrl+p"), Ok(Key::from_event(&ev(KeyCode::Char('P'), KeyModifiers::CONTROL))));
    }

    fn map(entries: &[(&str, &[&str])]) -> (Keymap, Vec<String>) {
        let o = entries
            .iter()
            .map(|(name, keys)| (name.to_string(), KeyBinding::Many(keys.iter().map(|k| k.to_string()).collect())))
            .collect();
        Keymap::new(&o)
    }

    #[test]
    fn an_override_replaces_the_actions_defaults() {
        let (m, warnings) = map(&[("run_all", &["ctrl+shift+r"]), ("quit", &[])]);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(m.keys(Action::RunAll), vec![Key::parse("ctrl+shift+r").unwrap()]);
        assert!(m.action(&ev(KeyCode::F(5), KeyModifiers::NONE), false, false).is_none(), "the default F5 is gone");
        assert!(m.keys(Action::Quit).is_empty(), "an empty list unbinds");
    }

    /// Binding a key that another action has by default must not leave two actions on one key:
    /// the user's binding wins, and the other action keeps its remaining keys.
    #[test]
    fn a_bound_key_is_taken_from_the_default_owner_so_nothing_is_shadowed() {
        let (m, warnings) = map(&[("run_all", &["ctrl+r"])]);
        let ctrl_r = ev(KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert_eq!(m.action(&ctrl_r, false, false), Some(Action::RunAll));
        assert!(m.keys(Action::History).is_empty());
        assert_eq!(warnings.len(), 1, "history lost its only key: {warnings:?}");
        assert!(warnings[0].contains("history"), "{warnings:?}");

        let (m, warnings) = map(&[("close_tab", &["alt+right"])]);
        assert_eq!(m.keys(Action::NextTab), vec![Key::parse("ctrl+pagedown").unwrap()], "next_tab keeps its other key");
        assert!(warnings.is_empty(), "no warning while the action still has a key: {warnings:?}");
    }

    #[test]
    fn conflicting_or_impossible_bindings_are_reported() {
        let (m, warnings) = map(&[
            ("run_all", &["f9"]),
            ("explain", &["f9"]),
            ("themes", &["alt+2"]),
            ("history", &["ctrl+c"]),
            ("help", &["h", "hyper+h"]),
            ("fly", &["f10"]),
        ]);
        let f9 = ev(KeyCode::F(9), KeyModifiers::NONE);
        // TOML tables don't keep their order, so the winner is the action that comes first in F1's list
        assert_eq!(m.action(&f9, false, false), Some(Action::RunAll));
        assert!(m.keys(Action::Explain).is_empty());
        assert!(m.keys(Action::Themes).is_empty(), "Alt+2 stays a tab jump");
        assert!(m.action(&ev(KeyCode::Char('c'), KeyModifiers::CONTROL), true, true).is_none(), "Ctrl+C stays copy in the editor");
        let text = warnings.join("\n");
        for needle in ["both", "tab", "editor", "text field", "hyper+h", "fly"] {
            assert!(text.contains(needle), "no warning mentioning {needle}: {text}");
        }
    }

    #[test]
    fn typing_keeps_plain_characters_and_the_editor_keeps_its_own_shortcuts() {
        let (map, _) = Keymap::new(&BTreeMap::new());
        let q = ev(KeyCode::Char('?'), KeyModifiers::NONE);
        assert_eq!(map.action(&q, true, false), None, "? is text in a filter");
        let ctrl_y = ev(KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert_eq!(map.action(&ctrl_y, false, false), Some(Action::Themes));
        assert_eq!(map.action(&ctrl_y, true, true), None, "Ctrl+Y is redo in the editor");
        let f1 = ev(KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(map.action(&f1, true, true), Some(Action::Help));
    }

    #[test]
    fn every_action_has_a_unique_name_and_a_default_key() {
        let mut names: Vec<&str> = Action::ALL.iter().map(|a| a.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Action::ALL.len());
        let (map, warnings) = Keymap::new(&BTreeMap::new());
        assert!(warnings.is_empty(), "{warnings:?}");
        for &a in Action::ALL {
            assert!(!map.keys(a).is_empty(), "{a:?} has no default key");
        }
    }
}
