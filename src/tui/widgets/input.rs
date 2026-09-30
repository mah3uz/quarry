use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;

use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Unhandled,
    Moved,
    Changed,
    Submit,
    Cancel,
}

#[derive(Clone, Debug, Default)]
pub struct Input {
    chars: Vec<char>,
    cursor: usize,
    pub masked: bool,
    pub placeholder: String,
    scroll: usize,
}

impl Input {
    pub fn new(value: &str) -> Self {
        let chars: Vec<char> = value.chars().collect();
        Input { cursor: chars.len(), chars, ..Default::default() }
    }

    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    pub fn with_placeholder(mut self, p: &str) -> Self {
        self.placeholder = p.to_string();
        self
    }

    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn set_value(&mut self, v: &str) {
        self.chars = v.chars().collect();
        self.cursor = self.chars.len();
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            self.chars.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> InputEvent {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Enter => InputEvent::Submit,
            KeyCode::Esc => InputEvent::Cancel,
            KeyCode::Char('u') if ctrl => {
                self.chars.drain(..self.cursor);
                self.cursor = 0;
                InputEvent::Changed
            }
            KeyCode::Char('k') if ctrl => {
                self.chars.truncate(self.cursor);
                InputEvent::Changed
            }
            KeyCode::Char('a') if ctrl => {
                self.cursor = 0;
                InputEvent::Moved
            }
            KeyCode::Char('e') if ctrl => {
                self.cursor = self.chars.len();
                InputEvent::Moved
            }
            KeyCode::Char('w') if ctrl => {
                self.delete_word_back();
                InputEvent::Changed
            }
            KeyCode::Backspace if ctrl || alt => {
                self.delete_word_back();
                InputEvent::Changed
            }
            KeyCode::Char('v') if ctrl => {
                if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    self.insert_str(&text.replace(['\n', '\r'], " "));
                }
                InputEvent::Changed
            }
            KeyCode::Char(c) if !ctrl => {
                self.chars.insert(self.cursor, c);
                self.cursor += 1;
                InputEvent::Changed
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.chars.remove(self.cursor);
                    InputEvent::Changed
                } else {
                    InputEvent::Moved
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.chars.len() {
                    self.chars.remove(self.cursor);
                    InputEvent::Changed
                } else {
                    InputEvent::Moved
                }
            }
            KeyCode::Left if ctrl => {
                self.cursor = self.word_start();
                InputEvent::Moved
            }
            KeyCode::Right if ctrl => {
                while self.cursor < self.chars.len() && !self.chars[self.cursor].is_alphanumeric() {
                    self.cursor += 1;
                }
                while self.cursor < self.chars.len() && self.chars[self.cursor].is_alphanumeric() {
                    self.cursor += 1;
                }
                InputEvent::Moved
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                InputEvent::Moved
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.chars.len());
                InputEvent::Moved
            }
            KeyCode::Home => {
                self.cursor = 0;
                InputEvent::Moved
            }
            KeyCode::End => {
                self.cursor = self.chars.len();
                InputEvent::Moved
            }
            _ => InputEvent::Unhandled,
        }
    }

    fn word_start(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && !self.chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        while i > 0 && self.chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        i
    }

    fn delete_word_back(&mut self) {
        let start = self.word_start();
        self.chars.drain(start..self.cursor);
        self.cursor = start;
    }

    /// Draws the value (or placeholder) into one row; returns the terminal cursor column when focused.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme, focused: bool) -> Option<(u16, u16)> {
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let width = area.width as usize;
        let base = Style::default().fg(theme.fg);
        if self.chars.is_empty() && !self.placeholder.is_empty() {
            buf.set_stringn(
                area.x,
                area.y,
                &self.placeholder,
                width,
                Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC),
            );
            return focused.then_some((area.x, area.y));
        }
        let shown: Vec<char> = if self.masked { vec!['•'; self.chars.len()] } else { self.chars.clone() };
        let cw = |c: &char| c.width().unwrap_or(0);
        let before: usize = shown[..self.cursor].iter().map(cw).sum();
        let scrolled: usize = shown[..self.scroll.min(shown.len())].iter().map(cw).sum();
        if before < scrolled {
            self.scroll = self.cursor;
        } else if before - scrolled >= width {
            let mut s = self.scroll;
            let mut w = before - scrolled;
            while w >= width && s < self.cursor {
                w -= cw(&shown[s]);
                s += 1;
            }
            self.scroll = s;
        }
        let mut x = area.x;
        let mut cursor_x = area.x;
        for (i, c) in shown.iter().enumerate().skip(self.scroll) {
            if i == self.cursor {
                cursor_x = x;
            }
            let w = cw(c) as u16;
            if x + w > area.x + area.width {
                break;
            }
            buf.set_string(x, area.y, c.to_string(), base);
            x += w;
        }
        if self.cursor >= shown.len() {
            cursor_x = x.min(area.x + area.width.saturating_sub(1));
        }
        focused.then_some((cursor_x, area.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn typing_and_word_delete() {
        let mut i = Input::new("select foo");
        assert_eq!(i.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)), InputEvent::Changed);
        assert_eq!(i.value(), "select ");
        i.handle_key(key(KeyCode::Char('é')));
        assert_eq!(i.value(), "select é");
        i.handle_key(key(KeyCode::Home));
        i.handle_key(key(KeyCode::Delete));
        assert_eq!(i.value(), "elect é");
    }

    #[test]
    fn masked_render_hides_secret() {
        let mut i = Input::new("hunter2").masked();
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        i.render(area, &mut buf, &Theme::default(), true);
        let row: String = (0..7).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert_eq!(row, "•••••••");
    }

    #[test]
    fn long_values_scroll_to_keep_cursor_visible() {
        let mut i = Input::new(&"x".repeat(50));
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        let (cx, _) = i.render(area, &mut buf, &Theme::default(), true).unwrap();
        assert!(cx < 10);
    }
}
