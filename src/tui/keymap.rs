//! App-level key bindings: defaults, overrides from `[keys]` in the config, and lookup.

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

    /// Keys the SQL editor uses itself (clipboard, undo, comment, completion); an app binding on one
    /// of them is skipped while the editor has focus.
    pub fn editor_uses(&self) -> bool {
        self.mods == KeyModifiers::CONTROL
            && matches!(self.code, KeyCode::Char('a' | 'c' | 'x' | 'v' | 'z' | 'y' | 'd' | 'h' | '/' | '7' | ' '))
    }
}

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

pub struct Keymap {
    bindings: Vec<(Key, Action)>,
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
            if !Action::ALL.iter().any(|a| a.name() == name) {
                warnings.push(format!("[keys] {name}: no such action (F1 lists them)"));
            }
        }
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
                if key.editor_uses() {
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
        (Keymap { bindings }, warnings)
    }

    /// The action bound to `key`. While typing (`typing`), plain characters are text, and in the
    /// SQL editor (`in_editor`) the editor's own shortcuts win.
    pub fn action(&self, key: &KeyEvent, typing: bool, in_editor: bool) -> Option<Action> {
        let k = Key::from_event(key);
        if typing && k.is_plain_char() || in_editor && k.editor_uses() {
            return None;
        }
        self.bindings.iter().find(|(b, _)| *b == k).map(|(_, a)| *a)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
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
