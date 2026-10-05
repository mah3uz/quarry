use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use super::widgets::input::{Input, InputEvent};
use crate::theme::Theme;

/// Subsequence fuzzy match. Rewards consecutive runs, word starts and prefix hits.
/// Returns (score, matched char indices).
pub fn fuzzy(pattern: &str, text: &str) -> Option<(i64, Vec<usize>)> {
    if pattern.is_empty() {
        return Some((0, Vec::new()));
    }
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.chars().collect();
    let tl: Vec<char> = text.to_lowercase().chars().collect();
    if tl.len() != t.len() {
        return None;
    }
    let mut positions = Vec::with_capacity(p.len());
    let mut score = 0i64;
    let mut ti = 0;
    let mut prev: Option<usize> = None;
    for &pc in &p {
        let mut found = None;
        while ti < tl.len() {
            if tl[ti] == pc {
                found = Some(ti);
                ti += 1;
                break;
            }
            ti += 1;
        }
        let i = found?;
        let boundary = i == 0 || !t[i - 1].is_alphanumeric() || (t[i].is_uppercase() && t[i - 1].is_lowercase());
        score += 1;
        if boundary {
            score += 8;
        }
        if prev.is_some_and(|pv| pv + 1 == i) {
            score += 6;
        }
        if i == 0 {
            score += 10;
        }
        prev = Some(i);
        positions.push(i);
    }
    score -= (t.len() as i64) / 8;
    if tl.iter().collect::<String>().contains(&pattern.to_lowercase()) {
        score += 15;
    }
    Some((score, positions))
}

#[derive(Clone, Debug)]
pub struct Item<T> {
    pub label: String,
    pub category: String,
    pub hint: String,
    pub value: T,
}

pub enum PaletteEvent<T> {
    None,
    Execute(T),
    /// Selection moved (live preview, e.g. themes).
    Preview(T),
    Close,
}

pub struct Palette<T: Clone> {
    pub title: String,
    input: Input,
    items: Vec<Item<T>>,
    filtered: Vec<(usize, Vec<usize>)>,
    selected: usize,
    offset: usize,
    /// Where the modal and its rows were last drawn, for clicks.
    area: Rect,
    list: Rect,
}

impl<T: Clone> Palette<T> {
    pub fn new(title: &str, placeholder: &str, items: Vec<Item<T>>) -> Self {
        let mut p = Palette {
            title: title.to_string(),
            input: Input::new("").with_placeholder(placeholder),
            items,
            filtered: Vec::new(),
            selected: 0,
            offset: 0,
            area: Rect::default(),
            list: Rect::default(),
        };
        p.refilter();
        p
    }

    pub fn select_label(&mut self, label: &str) {
        if let Some(i) = self.filtered.iter().position(|(idx, _)| self.items[*idx].label == label) {
            self.selected = i;
        }
    }

    fn refilter(&mut self) {
        let q = self.input.value();
        let mut scored: Vec<(i64, usize, Vec<usize>)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                let hay = if it.category.is_empty() { it.label.clone() } else { format!("{}: {}", it.category, it.label) };
                let offset = hay.chars().count() - it.label.chars().count();
                fuzzy(&q, &hay).map(|(s, pos)| {
                    let label_pos = pos.into_iter().filter(|p| *p >= offset).map(|p| p - offset).collect();
                    (s, i, label_pos)
                })
            })
            .collect();
        if !q.is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        self.filtered = scored.into_iter().map(|(_, i, pos)| (i, pos)).collect();
        self.selected = 0;
        self.offset = 0;
    }

    pub fn current(&self) -> Option<&Item<T>> {
        self.filtered.get(self.selected).map(|(i, _)| &self.items[*i])
    }

    pub fn handle_paste(&mut self, text: &str) -> PaletteEvent<T> {
        self.input.handle_paste(text);
        self.refilter();
        self.current().map_or(PaletteEvent::None, |i| PaletteEvent::Preview(i.value.clone()))
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PaletteEvent<T> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let before = self.selected;
        match key.code {
            KeyCode::Down | KeyCode::Tab => self.move_by(1),
            KeyCode::Char('n') | KeyCode::Char('j') if ctrl => self.move_by(1),
            KeyCode::Up | KeyCode::BackTab => self.move_by(-1),
            KeyCode::Char('p') | KeyCode::Char('k') if ctrl => self.move_by(-1),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::PageUp => self.move_by(-10),
            _ => match self.input.handle_key(key) {
                InputEvent::Submit => {
                    return self.current().map(|i| PaletteEvent::Execute(i.value.clone())).unwrap_or(PaletteEvent::Close);
                }
                InputEvent::Cancel => return PaletteEvent::Close,
                InputEvent::Changed => self.refilter(),
                _ => {}
            },
        }
        if (self.selected != before || key.code != KeyCode::Enter)
            && let Some(i) = self.current() {
                return PaletteEvent::Preview(i.value.clone());
            }
        PaletteEvent::None
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    /// Selects the row under (x, y); false when that isn't a row.
    pub fn select_at(&mut self, x: u16, y: u16) -> bool {
        let l = self.list;
        if x < l.x || x >= l.x + l.width || y < l.y || y >= l.y + l.height {
            return false;
        }
        let row = self.offset + (y - l.y) as usize;
        if row >= self.filtered.len() {
            return false;
        }
        self.selected = row;
        true
    }

    fn move_by(&mut self, d: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let n = self.filtered.len() as isize;
        self.selected = ((self.selected as isize + d).rem_euclid(n)) as usize;
    }

    pub fn render(&mut self, screen: Rect, buf: &mut Buffer, theme: &Theme) -> Option<(u16, u16)> {
        let width = (screen.width * 3 / 5).clamp(40.min(screen.width), 90);
        let list_h = (self.filtered.len() as u16).clamp(1, 14);
        let height = (list_h + 4).min(screen.height);
        let area = Rect {
            x: screen.x + (screen.width.saturating_sub(width)) / 2,
            y: screen.y + (screen.height / 6).min(screen.height.saturating_sub(height)),
            width,
            height,
        };
        let title = Span::styled(format!(" {} ", self.title), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD));
        let inner = super::dialogs::modal(area, buf, theme, Some(title), theme.border_focus);
        self.area = area;
        if inner.height < 2 {
            return None;
        }
        buf.set_string(inner.x + 1, inner.y, format!("{} ", crate::icons::get().prompt), Style::default().fg(theme.accent));
        let cursor = self.input.render(Rect { x: inner.x + 3, y: inner.y, width: inner.width.saturating_sub(4), height: 1 }, buf, theme, true);
        let count = format!("{}/{}", self.filtered.len(), self.items.len());
        buf.set_string(inner.x + inner.width - count.width() as u16 - 1, inner.y, &count, Style::default().fg(theme.muted));
        for x in inner.x..inner.x + inner.width {
            buf[(x, inner.y + 1)].set_symbol("─").set_style(Style::default().fg(theme.border));
        }
        let list = Rect { y: inner.y + 2, height: inner.height - 2, ..inner };
        self.list = list;
        let h = list.height as usize;
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if h > 0 && self.selected >= self.offset + h {
            self.offset = self.selected + 1 - h;
        }
        if self.filtered.is_empty() {
            buf.set_string(list.x + 2, list.y, "No matches", Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC));
        }
        for (row, (idx, positions)) in self.filtered.iter().enumerate().skip(self.offset).take(h) {
            let it = &self.items[*idx];
            let y = list.y + (row - self.offset) as u16;
            let sel = row == self.selected;
            let row_area = Rect { y, height: 1, ..list };
            let base = if sel { Style::default().bg(theme.selection).fg(theme.fg) } else { Style::default().fg(theme.fg) };
            buf.set_style(row_area, base);
            let mut spans = vec![Span::styled(if sel { " ▍" } else { "  " }, Style::default().fg(theme.accent))];
            if !it.category.is_empty() {
                spans.push(Span::styled(format!("{} ", it.category), Style::default().fg(theme.muted)));
            }
            for (ci, ch) in it.label.chars().enumerate() {
                let st = if positions.contains(&ci) {
                    Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(ch.to_string(), st));
            }
            Line::from(spans).render(row_area, buf);
            if !it.hint.is_empty() {
                let w = it.hint.width() as u16;
                if w + 2 < list.width {
                    buf.set_string(list.x + list.width - w - 1, y, &it.hint, Style::default().fg(theme.muted));
                }
            }
        }
        cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_word_starts_and_contiguous() {
        let (a, _) = fuzzy("nq", "New query tab").unwrap();
        let (b, _) = fuzzy("nq", "Run equal").unwrap();
        assert!(a > b);
        assert!(fuzzy("xyz", "New query").is_none());
        let (_, pos) = fuzzy("tab", "New query tab").unwrap();
        assert_eq!(pos, vec![10, 11, 12]);
    }

    #[test]
    fn palette_filters_and_executes_selection() {
        let items = vec![
            Item { label: "New query tab".into(), category: String::new(), hint: "Ctrl+T".into(), value: 1 },
            Item { label: "Toggle sidebar".into(), category: String::new(), hint: String::new(), value: 2 },
        ];
        let mut p = Palette::new("Commands", "type", items);
        for c in "side".chars() {
            p.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        match p.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            PaletteEvent::Execute(v) => assert_eq!(v, 2),
            _ => panic!("expected execute"),
        }
    }

    /// The theme picker previews whatever the event names, so a paste has to report the new top match.
    #[test]
    fn a_paste_refilters_and_previews_the_top_match() {
        let items = vec![
            Item { label: "New query tab".into(), category: String::new(), hint: String::new(), value: 1 },
            Item { label: "Toggle sidebar".into(), category: String::new(), hint: String::new(), value: 2 },
        ];
        let mut p = Palette::new("Commands", "type", items);
        assert!(matches!(p.handle_paste("side\n"), PaletteEvent::Preview(2)));
        assert!(matches!(p.handle_paste("zzz"), PaletteEvent::None));
    }
}
