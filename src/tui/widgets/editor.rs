use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::db::Backend;
use crate::sql::lexer::{Token, TokenKind, tokenize};
use crate::sql::split::split;
use crate::theme::Theme;

const INDENT: &str = "  ";
const TAB_WIDTH: usize = 4;
const UNDO_LIMIT: usize = 500;
const WHEEL_LINES: usize = 3;
const PLACEHOLDER: &str = "-- Write SQL here · Ctrl+Enter run statement · F5 run all · Ctrl+Space complete";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    /// Not consumed; the app may act on it.
    Unhandled,
    /// Consumed without changing the text (movement, selection, copy, scroll).
    Moved,
    Changed,
    /// Ctrl-Space, or a `.` typed after an identifier (the text changed too).
    RequestCompletion,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Pos {
    row: usize,
    col: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StepKind {
    Typing,
    DeleteBack,
    Other,
}

struct Edit {
    start: Pos,
    removed: String,
    inserted: String,
}

type CursorState = (Pos, Option<Pos>);

struct Step {
    edits: Vec<Edit>,
    kind: StepKind,
    before: CursorState,
    after: CursorState,
}

struct Tx {
    before: CursorState,
    edits: Vec<Edit>,
}

struct Highlight {
    generation: u64,
    text: String,
    line_starts: Vec<usize>,
    tokens: Vec<Token>,
}

/// Multi-line SQL editor. Draw it with [`Editor::render`]; `(row, col)` positions use byte columns.
pub struct Editor {
    lines: Vec<String>,
    cursor: Pos,
    /// Display column kept across vertical moves through shorter lines.
    want_col: Option<usize>,
    anchor: Option<Pos>,
    scroll_row: usize,
    scroll_col: usize,
    follow_cursor: bool,
    view_height: usize,
    undo: Vec<Step>,
    redo: Vec<Step>,
    backend: Backend,
    /// Blocks edits from keys and paste; the programmatic setters still work.
    pub read_only: bool,
    error_marker: Option<usize>,
    generation: u64,
    highlight: Option<Highlight>,
    register: String,
    clipboard: Option<arboard::Clipboard>,
    system_clipboard: bool,
    mouse_selecting: bool,
}

impl Editor {
    pub fn new(backend: Backend) -> Editor {
        Editor {
            lines: vec![String::new()],
            cursor: Pos::default(),
            want_col: None,
            anchor: None,
            scroll_row: 0,
            scroll_col: 0,
            follow_cursor: true,
            view_height: 10,
            undo: Vec::new(),
            redo: Vec::new(),
            backend,
            read_only: false,
            error_marker: None,
            generation: 0,
            highlight: None,
            register: String::new(),
            clipboard: None,
            // tests must not clobber the developer's clipboard
            system_clipboard: !cfg!(test),
            mouse_selecting: false,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Replaces the whole buffer, puts the cursor at the end and clears undo history.
    pub fn set_text(&mut self, text: &str) {
        self.lines = normalize_newlines(text).split('\n').map(str::to_string).collect();
        self.cursor = self.end_pos();
        self.anchor = None;
        self.want_col = None;
        self.undo.clear();
        self.redo.clear();
        self.scroll_row = 0;
        self.scroll_col = 0;
        self.follow_cursor = true;
        self.touch();
    }

    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn cursor_byte(&self) -> usize {
        self.pos_to_byte(self.cursor)
    }

    pub fn set_cursor_byte(&mut self, byte: usize) {
        let p = self.byte_to_pos(byte);
        self.move_to(p, false);
        self.want_col = None;
    }

    /// `(row, byte column)`.
    pub fn cursor_pos(&self) -> (usize, usize) {
        (self.cursor.row, self.cursor.col)
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn set_backend(&mut self, backend: Backend) {
        self.backend = backend;
        self.generation += 1;
    }

    /// Byte offset in `text()` to mark as an error (e.g. a Postgres error position). Cleared by any edit.
    pub fn set_error_marker(&mut self, byte: Option<usize>) {
        self.error_marker = byte;
    }

    pub fn insert_str(&mut self, s: &str) {
        self.replace_selection(&normalize_newlines(s), StepKind::Other);
    }

    /// Replaces a byte range of `text()` as one undo step and leaves the cursor after the new text.
    pub fn replace_range(&mut self, range: Range<usize>, with: &str) {
        let start = self.byte_to_pos(range.start);
        let end = self.byte_to_pos(range.end.max(range.start));
        let mut tx = self.begin();
        let p = self.tx_replace(&mut tx, start, end, &normalize_newlines(with));
        self.cursor = p;
        self.anchor = None;
        self.commit(tx, StepKind::Other);
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(a, b)| self.text_between(a, b))
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(Pos::default());
        self.cursor = self.end_pos();
        self.follow_cursor = true;
    }

    /// Identifier characters immediately left of the cursor.
    pub fn word_before_cursor(&self) -> &str {
        let line = &self.lines[self.cursor.row][..self.cursor.col];
        let start = line
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_word_char(*c))
            .last()
            .map_or(line.len(), |(i, _)| i);
        &line[start..]
    }

    /// Byte range (without terminator) of the statement under the cursor; in the gap between two
    /// statements the preceding one wins. Empty when the buffer has no statement.
    pub fn current_statement_range(&self) -> Range<usize> {
        let text = self.text();
        let c = self.cursor_byte();
        let stmts = split(&text, self.backend, ";");
        let idx = stmts.iter().rposition(|s| s.start <= c).unwrap_or(0);
        stmts.get(idx).map_or(c..c, |s| s.start..s.end)
    }

    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else { return false };
        for e in step.edits.iter().rev() {
            let end = end_of(e.start, &e.inserted);
            self.splice(e.start, end, &e.removed);
        }
        (self.cursor, self.anchor) = step.before;
        self.redo.push(step);
        self.after_history_jump();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(step) = self.redo.pop() else { return false };
        for e in &step.edits {
            let end = end_of(e.start, &e.removed);
            self.splice(e.start, end, &e.inserted);
        }
        (self.cursor, self.anchor) = step.after;
        self.undo.push(step);
        self.after_history_jump();
        true
    }

    fn after_history_jump(&mut self) {
        self.want_col = None;
        self.follow_cursor = true;
    }

    /// Inserts bracketed-paste text as a single undo step.
    pub fn handle_paste(&mut self, text: &str) -> EditorEvent {
        if self.read_only {
            return EditorEvent::Unhandled;
        }
        let text = normalize_newlines(text).replace('\t', &" ".repeat(TAB_WIDTH));
        self.replace_selection(&text, StepKind::Other);
        EditorEvent::Changed
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> EditorEvent {
        use EditorEvent::*;
        if key.kind == KeyEventKind::Release {
            return Unhandled;
        }
        let m = key.modifiers;
        let ctrl = m.contains(KeyModifiers::CONTROL);
        let alt = m.contains(KeyModifiers::ALT);
        let shift = m.contains(KeyModifiers::SHIFT);
        let edit = !self.read_only;
        match (key.code, ctrl, alt) {
            (KeyCode::Up, false, true) if edit => self.move_lines(-1),
            (KeyCode::Down, false, true) if edit => self.move_lines(1),
            (KeyCode::Left, _, false) => {
                let p = match self.selection() {
                    Some((a, _)) if !shift && !ctrl => a,
                    _ if ctrl => self.word_left(self.cursor),
                    _ => self.left_of(self.cursor),
                };
                self.move_horizontal(p, shift)
            }
            (KeyCode::Right, _, false) => {
                let p = match self.selection() {
                    Some((_, b)) if !shift && !ctrl => b,
                    _ if ctrl => self.word_right(self.cursor),
                    _ => self.right_of(self.cursor),
                };
                self.move_horizontal(p, shift)
            }
            (KeyCode::Up, false, false) => self.move_vertical(-1, shift),
            (KeyCode::Down, false, false) => self.move_vertical(1, shift),
            (KeyCode::PageUp, false, false) => {
                let h = self.view_height.max(1);
                self.scroll_row = self.scroll_row.saturating_sub(h);
                self.move_vertical(-(h as isize), shift)
            }
            (KeyCode::PageDown, false, false) => {
                let h = self.view_height.max(1);
                self.scroll_row = (self.scroll_row + h).min(self.lines.len() - 1);
                self.move_vertical(h as isize, shift)
            }
            (KeyCode::Home, true, false) => self.move_horizontal(Pos::default(), shift),
            (KeyCode::End, true, false) => self.move_horizontal(self.end_pos(), shift),
            (KeyCode::Home, false, false) => {
                let line = &self.lines[self.cursor.row];
                let first = line.len() - line.trim_start().len();
                let col = if self.cursor.col == first { 0 } else { first };
                self.move_horizontal(Pos { row: self.cursor.row, col }, shift)
            }
            (KeyCode::End, false, false) => {
                let col = self.lines[self.cursor.row].len();
                self.move_horizontal(Pos { row: self.cursor.row, col }, shift)
            }
            (KeyCode::Backspace, true, _) | (KeyCode::Backspace, _, true) if edit => {
                let to = self.word_left(self.cursor);
                self.delete_to(to)
            }
            (KeyCode::Backspace, false, false) if edit => self.backspace(),
            (KeyCode::Delete, true, false) if edit => {
                let to = self.word_right(self.cursor);
                self.delete_to(to)
            }
            (KeyCode::Delete, false, false) if edit => {
                let to = self.right_of(self.cursor);
                self.delete_to(to)
            }
            (KeyCode::Enter, false, false) if edit => self.newline(),
            (KeyCode::Tab, false, false) if edit => self.tab(),
            (KeyCode::BackTab, false, false) if edit => self.shift_lines(false),
            (KeyCode::Char(' ') | KeyCode::Null, true, false) => RequestCompletion,
            (KeyCode::Char(c), true, false) => match c.to_ascii_lowercase() {
                'a' => {
                    self.select_all();
                    Moved
                }
                'c' => match self.selected_text() {
                    Some(t) => {
                        self.copy(t);
                        Moved
                    }
                    None => Unhandled,
                },
                'x' if edit => match self.selected_text() {
                    Some(t) => {
                        self.copy(t);
                        self.replace_selection("", StepKind::Other);
                        Changed
                    }
                    None => Unhandled,
                },
                'v' if edit => {
                    let text = self.paste_source();
                    if text.is_empty() { Moved } else { self.handle_paste(&text) }
                }
                'z' if edit && (shift || c == 'Z') => changed_if(self.redo()),
                'z' if edit => changed_if(self.undo()),
                'y' if edit => changed_if(self.redo()),
                'd' if edit => self.duplicate_lines(),
                '/' | '7' if edit => self.toggle_comment(),
                // most terminals send Ctrl-Backspace as ^H
                'h' if edit => {
                    let to = self.word_left(self.cursor);
                    self.delete_to(to)
                }
                _ => Unhandled,
            },
            (KeyCode::Char(c), false, false) if edit => self.type_char(c),
            _ => Unhandled,
        }
    }

    /// `area` must be the rect last passed to [`Editor::render`].
    pub fn handle_mouse(&mut self, ev: MouseEvent, area: Rect) -> EditorEvent {
        let inside = area.contains(Position { x: ev.column, y: ev.row });
        let last = self.lines.len() - 1;
        match ev.kind {
            MouseEventKind::ScrollUp if inside => {
                self.scroll_row = self.scroll_row.saturating_sub(WHEEL_LINES);
                self.follow_cursor = false;
            }
            MouseEventKind::ScrollDown if inside => {
                self.scroll_row = (self.scroll_row + WHEEL_LINES).min(last);
                self.follow_cursor = false;
            }
            MouseEventKind::ScrollLeft if inside => {
                self.scroll_col = self.scroll_col.saturating_sub(WHEEL_LINES * 2);
                self.follow_cursor = false;
            }
            MouseEventKind::ScrollRight if inside => {
                self.scroll_col += WHEEL_LINES * 2;
                self.follow_cursor = false;
            }
            MouseEventKind::Down(MouseButton::Left) if inside => {
                let p = self.pos_at(ev.column, ev.row, area);
                if ev.modifiers.contains(KeyModifiers::SHIFT) {
                    self.anchor.get_or_insert(self.cursor);
                } else {
                    self.anchor = Some(p);
                }
                self.cursor = p;
                self.want_col = None;
                self.mouse_selecting = true;
                self.follow_cursor = true;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.mouse_selecting => {
                self.cursor = self.pos_at(ev.column, ev.row, area);
                self.want_col = None;
                self.follow_cursor = true;
            }
            MouseEventKind::Up(MouseButton::Left) if self.mouse_selecting => {
                self.mouse_selecting = false;
                if self.anchor == Some(self.cursor) {
                    self.anchor = None;
                }
            }
            _ => return EditorEvent::Unhandled,
        }
        EditorEvent::Moved
    }

    /// Terminal cell of the cursor inside `area`, or `None` when scrolled out of view.
    pub fn cursor_screen_position(&self, area: Rect) -> Option<(u16, u16)> {
        let gutter = self.gutter_width(area.width);
        let (w, h) = ((area.width - gutter) as usize, area.height as usize);
        if w == 0 || h == 0 {
            return None;
        }
        let (sr, sc) = if self.follow_cursor { self.scroll_for(h, w) } else { (self.scroll_row, self.scroll_col) };
        let dcol = display_col(&self.lines[self.cursor.row], self.cursor.col);
        let row = self.cursor.row;
        if row < sr || row >= sr + h || dcol < sc || dcol >= sc + w {
            return None;
        }
        Some((area.x + gutter + (dcol - sc) as u16, area.y + (row - sr) as u16))
    }

    /// Draws into `buf`; takes `&mut self` because it scrolls to the cursor and caches tokens.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme, focused: bool) {
        let area = area.intersection(buf.area);
        if area.is_empty() {
            return;
        }
        self.refresh_highlight();
        let gutter = self.gutter_width(area.width);
        let text_w = (area.width - gutter) as usize;
        let h = area.height as usize;
        self.view_height = h;
        if self.follow_cursor {
            (self.scroll_row, self.scroll_col) = self.scroll_for(h, text_w);
            self.follow_cursor = false;
        }
        self.scroll_row = self.scroll_row.min(self.lines.len() - 1);
        let (sr, sc) = (self.scroll_row, self.scroll_col);
        let Some(hl) = self.highlight.as_ref() else { return };
        let flat = |p: Pos| hl.line_starts[p.row] + p.col;
        let sel = self.selection().map(|(a, b)| flat(a)..flat(b));
        let brackets = matching_brackets(&hl.tokens, flat(self.cursor));
        let error_at = self.error_marker;
        let text_x = area.x + gutter;

        for i in 0..h {
            let row = sr + i;
            let y = area.y + i as u16;
            let current = focused && row == self.cursor.row;
            let line_bg = if current { theme.highlight } else { theme.bg };
            let base = Style::default().fg(theme.fg).bg(line_bg);
            for x in area.x..area.right() {
                buf[(x, y)].reset();
                buf[(x, y)].set_style(base);
            }
            if row >= self.lines.len() {
                continue;
            }
            if gutter > 0 {
                let num_style = if row == self.cursor.row {
                    base.fg(theme.accent).add_modifier(Modifier::BOLD)
                } else {
                    base.fg(theme.muted)
                };
                let digits = gutter as usize - 2;
                buf.set_stringn(area.x, y, format!(" {:>digits$} ", row + 1), gutter as usize, num_style);
            }
            if row == 0 && self.is_empty() {
                let style = base.fg(theme.muted).add_modifier(Modifier::ITALIC);
                buf.set_stringn(text_x, y, PLACEHOLDER, text_w, style);
            }

            let line = &self.lines[row];
            let ls = hl.line_starts[row];
            let mut ti = hl.tokens.partition_point(|t| t.end <= ls);
            let mut tok_style = (usize::MAX, base);
            let mut dcol = 0usize;
            for (bi, g) in line.grapheme_indices(true) {
                if dcol >= sc + text_w {
                    break;
                }
                let w = grapheme_width(g, dcol);
                if dcol + w > sc {
                    let abs = ls + bi;
                    while ti < hl.tokens.len() && hl.tokens[ti].end <= abs {
                        ti += 1;
                    }
                    if tok_style.0 != ti {
                        tok_style = (ti, token_style(&hl.tokens, ti, &hl.text, theme, base));
                    }
                    let style = decorate(tok_style.1, abs, sel.as_ref(), brackets, error_at, theme);
                    let x0 = dcol.max(sc) - sc;
                    let clipped = dcol < sc || dcol + w > sc + text_w;
                    let x = text_x + x0 as u16;
                    if g == "\t" || clipped || needs_placeholder(g) {
                        let sym = if needs_placeholder(g) { "·" } else { " " };
                        let visible = (dcol + w).min(sc + text_w) - dcol.max(sc);
                        for k in 0..visible as u16 {
                            buf[(x + k, y)].set_symbol(if k == 0 { sym } else { " " }).set_style(style);
                        }
                    } else {
                        buf[(x, y)].set_symbol(g).set_style(style);
                        for k in 1..w as u16 {
                            buf[(x + k, y)].reset();
                            buf[(x + k, y)].set_style(style);
                        }
                    }
                }
                dcol += w;
            }
            let eol = ls + line.len();
            if dcol >= sc && dcol < sc + text_w {
                let style = decorate(base, eol, sel.as_ref(), None, error_at, theme);
                if style != base {
                    buf[(text_x + (dcol - sc) as u16, y)].set_style(style);
                }
            }
        }
    }

    fn refresh_highlight(&mut self) {
        if self.highlight.as_ref().is_some_and(|h| h.generation == self.generation) {
            return;
        }
        let text = self.text();
        let mut line_starts = Vec::with_capacity(self.lines.len());
        let mut off = 0;
        for l in &self.lines {
            line_starts.push(off);
            off += l.len() + 1;
        }
        let tokens = tokenize(&text, self.backend);
        self.highlight = Some(Highlight { generation: self.generation, text, line_starts, tokens });
    }

    fn gutter_width(&self, area_width: u16) -> u16 {
        let digits = self.lines.len().to_string().len().max(2) as u16;
        let g = digits + 2;
        if area_width <= g + 1 { 0 } else { g }
    }

    fn scroll_for(&self, h: usize, w: usize) -> (usize, usize) {
        let row = self.cursor.row;
        let mut sr = self.scroll_row.min(self.lines.len() - 1);
        if row < sr {
            sr = row;
        } else if row >= sr + h {
            sr = row + 1 - h;
        }
        let dcol = display_col(&self.lines[row], self.cursor.col);
        let mut sc = self.scroll_col;
        if dcol < sc {
            // leave some context left of the cursor instead of pinning it to the edge
            sc = dcol.saturating_sub(w / 4);
        } else if dcol >= sc + w {
            sc = dcol + 1 - w;
        }
        (sr, sc)
    }

    fn pos_at(&self, x: u16, y: u16, area: Rect) -> Pos {
        let row = if y < area.y {
            self.scroll_row.saturating_sub(1)
        } else {
            (self.scroll_row + (y - area.y) as usize).min(self.lines.len() - 1)
        };
        let dx = x.saturating_sub(area.x + self.gutter_width(area.width)) as usize;
        Pos { row, col: col_at_display(&self.lines[row], self.scroll_col + dx) }
    }

    fn touch(&mut self) {
        self.generation += 1;
        self.error_marker = None;
    }

    fn end_pos(&self) -> Pos {
        let row = self.lines.len() - 1;
        Pos { row, col: self.lines[row].len() }
    }

    fn pos_to_byte(&self, p: Pos) -> usize {
        self.lines[..p.row].iter().map(|l| l.len() + 1).sum::<usize>() + p.col
    }

    fn byte_to_pos(&self, mut b: usize) -> Pos {
        for (row, l) in self.lines.iter().enumerate() {
            if b <= l.len() {
                return Pos { row, col: floor_boundary(l, b) };
            }
            b -= l.len() + 1;
        }
        self.end_pos()
    }

    fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        (a != self.cursor).then(|| (a.min(self.cursor), a.max(self.cursor)))
    }

    fn text_between(&self, a: Pos, b: Pos) -> String {
        if a.row == b.row {
            return self.lines[a.row][a.col..b.col].to_string();
        }
        let mut s = self.lines[a.row][a.col..].to_string();
        for line in &self.lines[a.row + 1..b.row] {
            s.push('\n');
            s.push_str(line);
        }
        s.push('\n');
        s.push_str(&self.lines[b.row][..b.col]);
        s
    }

    fn char_before(&self, p: Pos) -> Option<char> {
        self.lines[p.row][..p.col].chars().next_back()
    }

    fn char_at(&self, p: Pos) -> Option<char> {
        self.lines[p.row][p.col..].chars().next()
    }

    /// Raw buffer mutation; returns the position just after the inserted text.
    fn splice(&mut self, start: Pos, end: Pos, text: &str) -> Pos {
        let tail = self.lines[end.row][end.col..].to_string();
        self.lines[start.row].truncate(start.col);
        if end.row > start.row {
            self.lines.drain(start.row + 1..=end.row);
        }
        let mut parts = text.split('\n');
        self.lines[start.row].push_str(parts.next().unwrap_or(""));
        let rest: Vec<String> = parts.map(str::to_string).collect();
        let row = start.row + rest.len();
        self.lines.splice(start.row + 1..start.row + 1, rest);
        let col = self.lines[row].len();
        self.lines[row].push_str(&tail);
        self.touch();
        Pos { row, col }
    }

    fn begin(&self) -> Tx {
        Tx { before: (self.cursor, self.anchor), edits: Vec::new() }
    }

    fn tx_replace(&mut self, tx: &mut Tx, start: Pos, end: Pos, text: &str) -> Pos {
        let removed = self.text_between(start, end);
        let p = self.splice(start, end, text);
        tx.edits.push(Edit { start, removed, inserted: text.to_string() });
        p
    }

    fn commit(&mut self, tx: Tx, kind: StepKind) {
        self.want_col = None;
        self.follow_cursor = true;
        if tx.edits.is_empty() {
            return;
        }
        self.redo.clear();
        let after = (self.cursor, self.anchor);
        if kind != StepKind::Other
            && let [e] = tx.edits.as_slice()
            && let Some(last) = self.undo.last_mut()
            && last.kind == kind
            && last.after == tx.before
            && let [prev] = last.edits.as_mut_slice()
        {
            let merged = match kind {
                StepKind::Typing => e.removed.is_empty() && end_of(prev.start, &prev.inserted) == e.start,
                _ => e.inserted.is_empty() && prev.inserted.is_empty() && end_of(e.start, &e.removed) == prev.start,
            };
            if merged {
                if kind == StepKind::Typing {
                    prev.inserted.push_str(&e.inserted);
                } else {
                    prev.start = e.start;
                    prev.removed.insert_str(0, &e.removed);
                }
                last.after = after;
                return;
            }
        }
        self.undo.push(Step { edits: tx.edits, kind, before: tx.before, after });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    fn replace_selection(&mut self, text: &str, kind: StepKind) {
        let (a, b) = self.selection().unwrap_or((self.cursor, self.cursor));
        let mut tx = self.begin();
        self.cursor = self.tx_replace(&mut tx, a, b, text);
        self.anchor = None;
        self.commit(tx, kind);
    }

    fn move_to(&mut self, p: Pos, select: bool) {
        if select {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = p;
        self.follow_cursor = true;
    }

    fn move_horizontal(&mut self, p: Pos, select: bool) -> EditorEvent {
        self.move_to(p, select);
        self.want_col = None;
        EditorEvent::Moved
    }

    fn move_vertical(&mut self, delta: isize, select: bool) -> EditorEvent {
        let Pos { row, col } = self.cursor;
        let want = self.want_col.unwrap_or_else(|| display_col(&self.lines[row], col));
        let target = row.saturating_add_signed(delta).min(self.lines.len() - 1);
        let p = if target == row {
            Pos { row, col: if delta < 0 { 0 } else { self.lines[row].len() } }
        } else {
            Pos { row: target, col: col_at_display(&self.lines[target], want) }
        };
        self.move_to(p, select);
        self.want_col = Some(want);
        EditorEvent::Moved
    }

    fn left_of(&self, p: Pos) -> Pos {
        if p.col > 0 {
            let col = self.lines[p.row][..p.col].grapheme_indices(true).next_back().map_or(0, |(i, _)| i);
            Pos { row: p.row, col }
        } else if p.row > 0 {
            Pos { row: p.row - 1, col: self.lines[p.row - 1].len() }
        } else {
            p
        }
    }

    fn right_of(&self, p: Pos) -> Pos {
        let line = &self.lines[p.row];
        if p.col < line.len() {
            Pos { row: p.row, col: p.col + line[p.col..].graphemes(true).next().map_or(0, str::len) }
        } else if p.row + 1 < self.lines.len() {
            Pos { row: p.row + 1, col: 0 }
        } else {
            p
        }
    }

    fn word_left(&self, p: Pos) -> Pos {
        if p.col == 0 {
            return self.left_of(p);
        }
        let mut it = self.lines[p.row][..p.col].char_indices().rev().peekable();
        let mut col = p.col;
        while let Some(&(i, c)) = it.peek()
            && c.is_whitespace()
        {
            col = i;
            it.next();
        }
        if let Some(&(_, c)) = it.peek() {
            let class = char_class(c);
            while let Some(&(i, c)) = it.peek()
                && char_class(c) == class
            {
                col = i;
                it.next();
            }
        }
        Pos { row: p.row, col }
    }

    fn word_right(&self, p: Pos) -> Pos {
        let line = &self.lines[p.row];
        if p.col >= line.len() {
            return self.right_of(p);
        }
        let rest = &line[p.col..];
        let mut chars = rest.char_indices().peekable();
        if let Some(&(_, c)) = chars.peek()
            && !c.is_whitespace()
        {
            let class = char_class(c);
            while chars.next_if(|&(_, c)| char_class(c) == class).is_some() {}
        }
        while chars.next_if(|&(_, c)| c.is_whitespace()).is_some() {}
        Pos { row: p.row, col: p.col + chars.peek().map_or(rest.len(), |&(i, _)| i) }
    }

    fn delete_to(&mut self, to: Pos) -> EditorEvent {
        if self.selection().is_some() {
            self.replace_selection("", StepKind::Other);
            return EditorEvent::Changed;
        }
        if to == self.cursor {
            return EditorEvent::Moved;
        }
        let (a, b) = (to.min(self.cursor), to.max(self.cursor));
        let mut tx = self.begin();
        self.cursor = self.tx_replace(&mut tx, a, b, "");
        self.anchor = None;
        self.commit(tx, StepKind::Other);
        EditorEvent::Changed
    }

    fn backspace(&mut self) -> EditorEvent {
        if self.selection().is_some() {
            return self.delete_to(self.cursor);
        }
        let Pos { row, col } = self.cursor;
        if row == 0 && col == 0 {
            return EditorEvent::Moved;
        }
        let before = &self.lines[row][..col];
        let mut start = self.left_of(self.cursor);
        let mut end = self.cursor;
        if col > 0 && before.bytes().all(|b| b == b' ') {
            start.col = col - ((col - 1) % INDENT.len() + 1);
        } else if let (Some(open), Some(close)) = (self.char_before(self.cursor), self.char_at(self.cursor))
            && closer_for(open) == Some(close)
        {
            end.col += close.len_utf8();
        }
        let mut tx = self.begin();
        self.cursor = self.tx_replace(&mut tx, start, end, "");
        self.anchor = None;
        self.commit(tx, StepKind::DeleteBack);
        EditorEvent::Changed
    }

    fn type_char(&mut self, c: char) -> EditorEvent {
        let has_sel = self.selection().is_some();
        let prev = self.char_before(self.cursor);
        let next = self.char_at(self.cursor);
        if !has_sel && matches!(c, ')' | '\'' | '"' | '`') && next == Some(c) {
            let p = self.right_of(self.cursor);
            return self.move_horizontal(p, false);
        }
        let word_adjacent = |ch: Option<char>| ch.is_some_and(is_word_char);
        let pair = match closer_for(c) {
            Some(close) if !has_sel && !word_adjacent(next) && (c == '(' || !word_adjacent(prev)) => Some(close),
            _ => None,
        };
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        match pair {
            Some(close) => {
                let text = format!("{c}{close}");
                self.replace_selection(&text, StepKind::Typing);
                self.cursor.col -= close.len_utf8();
            }
            None => self.replace_selection(s, StepKind::Typing),
        }
        if pair.is_some() && let Some(last) = self.undo.last_mut() {
            last.after.0 = self.cursor;
        }
        let after_ident = prev.is_some_and(|p| is_word_char(p) || matches!(p, '"' | '`' | ']'));
        if c == '.' && !has_sel && after_ident { EditorEvent::RequestCompletion } else { EditorEvent::Changed }
    }

    fn newline(&mut self) -> EditorEvent {
        let start = self.selection().map_or(self.cursor, |(a, _)| a);
        let before = &self.lines[start.row][..start.col];
        let indent: String = before.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let depth = before.chars().fold(0i32, |d, ch| match ch {
            '(' => d + 1,
            ')' => d - 1,
            _ => d,
        });
        let mut text = format!("\n{indent}");
        if depth > 0 {
            text.push_str(INDENT);
        }
        let caret = text.len() - 1;
        if depth > 0 && self.selection().is_none() && self.char_at(self.cursor) == Some(')') {
            text.push('\n');
            text.push_str(&indent);
        }
        self.replace_selection(&text, StepKind::Other);
        self.cursor = Pos { row: start.row + 1, col: caret };
        if let Some(last) = self.undo.last_mut() {
            last.after.0 = self.cursor;
        }
        EditorEvent::Changed
    }

    fn tab(&mut self) -> EditorEvent {
        match self.selection() {
            Some((a, b)) if a.row != b.row => self.shift_lines(true),
            _ => {
                self.replace_selection(INDENT, StepKind::Other);
                EditorEvent::Changed
            }
        }
    }

    /// Rows touched by the selection (a selection ending at column 0 excludes that row), else the cursor row.
    fn selected_rows(&self) -> Range<usize> {
        match self.selection() {
            Some((a, b)) if b.row > a.row && b.col == 0 => a.row..b.row,
            Some((a, b)) => a.row..b.row + 1,
            None => self.cursor.row..self.cursor.row + 1,
        }
    }

    /// Applies per-line `(row, col, remove_len, insert)` edits as one undo step, keeping cursor and
    /// anchor attached to the same text.
    fn apply_line_edits(&mut self, edits: Vec<(usize, usize, usize, &str)>) -> EditorEvent {
        if edits.is_empty() {
            return EditorEvent::Moved;
        }
        let mut tx = self.begin();
        for (row, col, remove, insert) in edits {
            self.tx_replace(&mut tx, Pos { row, col }, Pos { row, col: col + remove }, insert);
            let shift = |p: &mut Pos| {
                if p.row == row && p.col >= col {
                    p.col = if p.col >= col + remove { p.col - remove + insert.len() } else { col };
                }
            };
            shift(&mut self.cursor);
            if let Some(a) = self.anchor.as_mut() {
                shift(a);
            }
        }
        self.commit(tx, StepKind::Other);
        EditorEvent::Changed
    }

    fn shift_lines(&mut self, indent: bool) -> EditorEvent {
        let edits = self
            .selected_rows()
            .filter_map(|row| {
                let line = &self.lines[row];
                if indent {
                    (!line.is_empty()).then_some((row, 0, 0, INDENT))
                } else {
                    let n = line.bytes().take(INDENT.len()).take_while(|b| *b == b' ').count();
                    (n > 0).then_some((row, 0, n, ""))
                }
            })
            .collect();
        self.apply_line_edits(edits)
    }

    fn toggle_comment(&mut self) -> EditorEvent {
        let rows = self.selected_rows();
        let indent_of = |l: &str| l.len() - l.trim_start().len();
        let mut targets: Vec<usize> = rows.clone().filter(|&r| !self.lines[r].trim().is_empty()).collect();
        if targets.is_empty() {
            targets = rows.collect();
        }
        let commented = targets.iter().all(|&r| self.lines[r].trim_start().starts_with("--"));
        let edits = if commented {
            targets
                .iter()
                .map(|&r| {
                    let l = &self.lines[r];
                    let i = indent_of(l);
                    let n = if l[i..].starts_with("-- ") { 3 } else { 2 };
                    (r, i, n, "")
                })
                .collect()
        } else {
            let col = targets.iter().map(|&r| indent_of(&self.lines[r])).min().unwrap_or(0);
            targets.iter().map(|&r| (r, col, 0, "-- ")).collect()
        };
        self.apply_line_edits(edits)
    }

    fn duplicate_lines(&mut self) -> EditorEvent {
        let rows = self.selected_rows();
        let n = rows.len();
        let last = rows.end - 1;
        let block = format!("\n{}", self.lines[rows].join("\n"));
        let mut tx = self.begin();
        let at = Pos { row: last, col: self.lines[last].len() };
        self.tx_replace(&mut tx, at, at, &block);
        self.cursor.row += n;
        if let Some(a) = self.anchor.as_mut() {
            a.row += n;
        }
        self.commit(tx, StepKind::Other);
        EditorEvent::Changed
    }

    fn move_lines(&mut self, dir: isize) -> EditorEvent {
        let rows = self.selected_rows();
        let (first, last) = (rows.start, rows.end - 1);
        if (dir < 0 && first == 0) || (dir > 0 && last + 1 >= self.lines.len()) {
            return EditorEvent::Moved;
        }
        let (lo, hi) = if dir < 0 { (first - 1, last) } else { (first, last + 1) };
        let mut region: Vec<&str> = self.lines[lo..=hi].iter().map(String::as_str).collect();
        if dir < 0 {
            region.rotate_left(1);
        } else {
            region.rotate_right(1);
        }
        let text = region.join("\n");
        let mut tx = self.begin();
        let end = Pos { row: hi, col: self.lines[hi].len() };
        self.tx_replace(&mut tx, Pos { row: lo, col: 0 }, end, &text);
        let moved = |p: &mut Pos| p.row = p.row.saturating_add_signed(dir);
        moved(&mut self.cursor);
        if let Some(a) = self.anchor.as_mut() {
            moved(a);
        }
        self.commit(tx, StepKind::Other);
        EditorEvent::Changed
    }

    fn clipboard_handle(&mut self) -> Option<&mut arboard::Clipboard> {
        if !self.system_clipboard {
            return None;
        }
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(cb) => self.clipboard = Some(cb),
                Err(_) => {
                    self.system_clipboard = false;
                    return None;
                }
            }
        }
        self.clipboard.as_mut()
    }

    fn copy(&mut self, text: String) {
        if let Some(cb) = self.clipboard_handle() {
            let _ = cb.set_text(text.clone());
        }
        self.register = text;
    }

    fn paste_source(&mut self) -> String {
        let system = self.clipboard_handle().and_then(|cb| cb.get_text().ok());
        system.unwrap_or_else(|| self.register.clone())
    }
}

fn changed_if(changed: bool) -> EditorEvent {
    if changed { EditorEvent::Changed } else { EditorEvent::Moved }
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn end_of(start: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        None => Pos { row: start.row, col: start.col + text.len() },
        Some(i) => Pos { row: start.row + text.matches('\n').count(), col: text.len() - i - 1 },
    }
}

fn floor_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum CharClass {
    Space,
    Word,
    Punct,
}

fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Space
    } else if is_word_char(c) {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

fn closer_for(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '\'' | '"' | '`' => Some(open),
        _ => None,
    }
}

fn needs_placeholder(g: &str) -> bool {
    g != "\t" && (g.width() == 0 || g.chars().any(char::is_control))
}

fn grapheme_width(g: &str, dcol: usize) -> usize {
    if g == "\t" {
        TAB_WIDTH - dcol % TAB_WIDTH
    } else if needs_placeholder(g) {
        1
    } else {
        g.width()
    }
}

fn display_col(line: &str, col: usize) -> usize {
    line[..col].graphemes(true).fold(0, |d, g| d + grapheme_width(g, d))
}

/// Byte column of the grapheme covering display column `target` (end of line if past it).
fn col_at_display(line: &str, target: usize) -> usize {
    let mut d = 0;
    for (i, g) in line.grapheme_indices(true) {
        let w = grapheme_width(g, d);
        if d + w > target {
            return i;
        }
        d += w;
    }
    line.len()
}

fn token_style(tokens: &[Token], i: usize, text: &str, theme: &Theme, base: Style) -> Style {
    let Some(t) = tokens.get(i) else { return base };
    let next_is_paren = tokens.get(i + 1).is_some_and(|n| n.kind == TokenKind::LParen);
    match t.kind {
        TokenKind::Keyword => {
            let word = t.text(text);
            if word.eq_ignore_ascii_case("null") {
                base.fg(theme.null).add_modifier(Modifier::BOLD)
            } else if word.eq_ignore_ascii_case("true") || word.eq_ignore_ascii_case("false") {
                base.fg(theme.boolean).add_modifier(Modifier::BOLD)
            } else {
                base.fg(theme.keyword).add_modifier(Modifier::BOLD)
            }
        }
        TokenKind::DataType => base.fg(theme.datatype),
        TokenKind::Builtin => base.fg(theme.function),
        TokenKind::Ident if next_is_paren => base.fg(theme.function),
        TokenKind::Ident => base.fg(theme.identifier),
        TokenKind::QuotedIdent => base.fg(theme.quoted_ident),
        TokenKind::String => base.fg(theme.string),
        TokenKind::Number => base.fg(theme.number),
        TokenKind::LineComment | TokenKind::BlockComment => base.fg(theme.comment).add_modifier(Modifier::ITALIC),
        TokenKind::Operator => base.fg(theme.operator),
        TokenKind::Parameter | TokenKind::Variable => base.fg(theme.parameter),
        TokenKind::Comma
        | TokenKind::Dot
        | TokenKind::LParen
        | TokenKind::RParen
        | TokenKind::LBracket
        | TokenKind::RBracket
        | TokenKind::Semicolon
        | TokenKind::Backslash => base.fg(theme.punctuation),
        TokenKind::Whitespace | TokenKind::Unknown => base,
    }
}

fn decorate(
    style: Style,
    abs: usize,
    sel: Option<&Range<usize>>,
    brackets: Option<(usize, usize)>,
    error_at: Option<usize>,
    theme: &Theme,
) -> Style {
    let mut style = style;
    if brackets.is_some_and(|(a, b)| abs == a || abs == b) {
        style = style.bg(theme.border).add_modifier(Modifier::BOLD);
    }
    if sel.is_some_and(|r| r.contains(&abs)) {
        style = style.bg(theme.selection);
    }
    if error_at == Some(abs) {
        style = style.fg(theme.error).add_modifier(Modifier::UNDERLINED | Modifier::REVERSED);
    }
    style
}

/// Byte offsets of the bracket at/before `cursor` and its partner, ignoring brackets in strings/comments.
fn matching_brackets(tokens: &[Token], cursor: usize) -> Option<(usize, usize)> {
    let is_bracket = |t: &Token| {
        matches!(t.kind, TokenKind::LParen | TokenKind::RParen | TokenKind::LBracket | TokenKind::RBracket)
    };
    let i = tokens.partition_point(|t| t.start < cursor);
    let idx = if tokens.get(i).is_some_and(|t| t.start == cursor && is_bracket(t)) {
        i
    } else if i > 0 && tokens[i - 1].end == cursor && is_bracket(&tokens[i - 1]) {
        i - 1
    } else {
        return None;
    };
    let (open, close, forward) = match tokens[idx].kind {
        TokenKind::LParen => (TokenKind::LParen, TokenKind::RParen, true),
        TokenKind::RParen => (TokenKind::LParen, TokenKind::RParen, false),
        TokenKind::LBracket => (TokenKind::LBracket, TokenKind::RBracket, true),
        _ => (TokenKind::LBracket, TokenKind::RBracket, false),
    };
    let (inc, dec) = if forward { (open, close) } else { (close, open) };
    let mut depth = 0usize;
    let mut scan = |j: usize| {
        let k = tokens[j].kind;
        if k == inc {
            depth += 1;
        } else if k == dec {
            if depth == 0 {
                return Some(tokens[j].start);
            }
            depth -= 1;
        }
        None
    };
    let partner = if forward {
        (idx + 1..tokens.len()).find_map(&mut scan)
    } else {
        (0..idx).rev().find_map(&mut scan)
    };
    partner.map(|p| (tokens[idx].start, p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
    const ALT: KeyModifiers = KeyModifiers::ALT;

    fn key(e: &mut Editor, code: KeyCode, m: KeyModifiers) -> EditorEvent {
        e.handle_key(KeyEvent::new(code, m))
    }

    fn typ(e: &mut Editor, s: &str) {
        for c in s.chars() {
            let ev = if c == '\n' { key(e, KeyCode::Enter, NONE) } else { key(e, KeyCode::Char(c), NONE) };
            assert_ne!(ev, EditorEvent::Unhandled, "typing {c:?}");
        }
    }

    fn editor(text: &str) -> Editor {
        let mut e = Editor::new(Backend::Postgres);
        e.set_text(text);
        e
    }

    fn render(e: &mut Editor, w: u16, h: u16) -> (Buffer, Rect) {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        e.render(area, &mut buf, &Theme::default(), true);
        (buf, area)
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn consecutive_typing_is_one_undo_step_but_a_cursor_move_splits_it() {
        let mut e = Editor::new(Backend::Postgres);
        typ(&mut e, "select");
        assert!(e.undo());
        assert_eq!(e.text(), "");
        assert!(e.redo());
        assert_eq!(e.text(), "select");
        assert_eq!(e.cursor_byte(), 6);

        key(&mut e, KeyCode::Left, NONE);
        typ(&mut e, "X");
        assert_eq!(e.text(), "selecXt");
        e.undo();
        assert_eq!(e.text(), "select", "only the typing after the move is undone");
    }

    #[test]
    fn consecutive_backspaces_undo_together_and_new_edits_clear_redo() {
        let mut e = editor("select 1");
        key(&mut e, KeyCode::Backspace, NONE);
        key(&mut e, KeyCode::Backspace, NONE);
        key(&mut e, KeyCode::Backspace, NONE);
        assert_eq!(e.text(), "selec");
        e.undo();
        assert_eq!(e.text(), "select 1");
        assert_eq!(e.cursor_byte(), 8);

        e.undo();
        typ(&mut e, "z");
        assert!(!e.redo(), "a fresh edit invalidates the redo branch");
    }

    #[test]
    fn undo_history_is_capped() {
        let mut e = Editor::new(Backend::Postgres);
        for _ in 0..UNDO_LIMIT + 50 {
            key(&mut e, KeyCode::Enter, NONE);
        }
        let mut n = 0;
        while e.undo() {
            n += 1;
        }
        assert_eq!(n, UNDO_LIMIT);
        assert_eq!(e.line_count(), 51);
    }

    #[test]
    fn set_text_resets_history() {
        let mut e = Editor::new(Backend::Postgres);
        typ(&mut e, "abc");
        e.set_text("select 1");
        assert!(!e.undo());
        assert_eq!(e.cursor_pos(), (0, 8));
    }

    #[test]
    fn replace_range_applies_a_completion_as_one_undoable_step() {
        let mut e = editor("SELECT * FROM us WHERE 1");
        e.set_cursor_byte(16);
        assert_eq!(e.word_before_cursor(), "us");
        e.replace_range(14..16, "users");
        assert_eq!(e.text(), "SELECT * FROM users WHERE 1");
        assert_eq!(e.cursor_byte(), 19, "cursor lands after the inserted text");
        e.undo();
        assert_eq!(e.text(), "SELECT * FROM us WHERE 1");

        let mut m = editor("select\n  fr");
        m.replace_range(9..11, "FROM\n  users");
        assert_eq!(m.text(), "select\n  FROM\n  users");
        assert_eq!(m.cursor_pos(), (2, 7));
    }

    #[test]
    fn current_statement_follows_the_cursor_across_statements() {
        let src = "select 1;\nselect 'a;b' as x;\n\nselect 3";
        let mut e = editor(src);
        let stmt = |e: &Editor| src[e.current_statement_range()].to_string();

        e.set_cursor_byte(3);
        assert_eq!(stmt(&e), "select 1");
        e.set_cursor_byte(src.find("a;b").unwrap() + 2);
        assert_eq!(stmt(&e), "select 'a;b' as x", "a ; inside a string does not split");
        e.set_cursor_byte(src.find("x;").unwrap() + 2);
        assert_eq!(stmt(&e), "select 'a;b' as x", "right after the terminator");
        e.set_cursor_byte(src.find("\n\n").unwrap() + 1);
        assert_eq!(stmt(&e), "select 'a;b' as x", "blank line after a statement picks the previous one");
        e.set_cursor_byte(src.len());
        assert_eq!(stmt(&e), "select 3");

        let empty = editor("  \n ");
        assert!(empty.current_statement_range().is_empty());
    }

    #[test]
    fn cut_and_paste_round_trip_through_the_internal_register() {
        let mut e = editor("select a, b from t");
        e.set_cursor_byte(7);
        for _ in 0..3 {
            key(&mut e, KeyCode::Right, SHIFT);
        }
        assert_eq!(e.selected_text().as_deref(), Some("a, "));
        assert_eq!(key(&mut e, KeyCode::Char('x'), CTRL), EditorEvent::Changed);
        assert_eq!(e.text(), "select b from t");
        assert_eq!(e.selected_text(), None);

        key(&mut e, KeyCode::End, NONE);
        typ(&mut e, " ");
        assert_eq!(key(&mut e, KeyCode::Char('v'), CTRL), EditorEvent::Changed);
        assert_eq!(e.text(), "select b from t a, ");

        assert_eq!(key(&mut e, KeyCode::Char('c'), CTRL), EditorEvent::Unhandled, "Ctrl-C without selection cancels queries");
        key(&mut e, KeyCode::Char('a'), CTRL);
        assert_eq!(key(&mut e, KeyCode::Char('c'), CTRL), EditorEvent::Moved);
        assert_eq!(e.register, "select b from t a, ");
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut e = editor("select 1");
        key(&mut e, KeyCode::Home, SHIFT);
        typ(&mut e, "x");
        assert_eq!(e.text(), "x");
        key(&mut e, KeyCode::Char('a'), CTRL);
        key(&mut e, KeyCode::Backspace, NONE);
        assert!(e.is_empty());
    }

    #[test]
    fn enter_keeps_indentation_and_indents_inside_open_parens() {
        let mut e = editor("  select");
        typ(&mut e, "\n");
        assert_eq!(e.text(), "  select\n  ");

        let mut e = editor("insert into t ");
        typ(&mut e, "(\n");
        assert_eq!(e.text(), "insert into t (\n  \n)", "closing paren moves to its own line");
        assert_eq!(e.cursor_pos(), (1, 2));
        typ(&mut e, "a,\n");
        assert_eq!(e.cursor_pos(), (2, 2), "still inside the open paren: keep the indent");

        let mut e = editor("values (1,");
        typ(&mut e, "\n");
        assert_eq!(e.text(), "values (1,\n  ");
    }

    #[test]
    fn brackets_and_quotes_auto_close_and_are_typed_over() {
        let mut e = Editor::new(Backend::Postgres);
        typ(&mut e, "count(");
        assert_eq!(e.text(), "count()");
        typ(&mut e, "*)");
        assert_eq!(e.text(), "count(*)", "typing ) next to the auto-inserted one skips over it");
        assert_eq!(e.cursor_byte(), 8);

        let mut e = Editor::new(Backend::Postgres);
        typ(&mut e, "'abc'");
        assert_eq!(e.text(), "'abc'");
        typ(&mut e, " don't");
        assert_eq!(e.text(), "'abc' don't", "no auto-close for an apostrophe inside a word");

        let mut e = Editor::new(Backend::Postgres);
        typ(&mut e, "(");
        key(&mut e, KeyCode::Backspace, NONE);
        assert!(e.is_empty(), "backspace in an empty pair removes both halves");

        let mut e = editor("abc");
        e.set_cursor_byte(0);
        typ(&mut e, "(");
        assert_eq!(e.text(), "(abc", "no auto-close right before a word");
    }

    #[test]
    fn comment_toggle_round_trips_selected_lines() {
        let src = "select a,\n    b\nfrom t";
        let mut e = editor(src);
        e.set_cursor_byte(2);
        key(&mut e, KeyCode::Down, SHIFT);
        key(&mut e, KeyCode::Char('/'), CTRL);
        assert_eq!(e.text(), "-- select a,\n--     b\nfrom t");
        assert_eq!(e.selected_text().as_deref(), Some("lect a,\n--   "), "selection stays on the same text");
        key(&mut e, KeyCode::Char('/'), CTRL);
        assert_eq!(e.text(), src);

        let mut indented = editor("  x\n    y");
        key(&mut indented, KeyCode::Char('a'), CTRL);
        key(&mut indented, KeyCode::Char('7'), CTRL);
        assert_eq!(indented.text(), "  -- x\n  --   y", "comment markers align at the shallowest indent");
        indented.undo();
        assert_eq!(indented.text(), "  x\n    y", "toggle is a single undo step");
    }

    #[test]
    fn cursor_bytes_map_to_rows_and_columns_with_multibyte_text() {
        let mut e = editor("héllo\n日本語 x");
        e.set_cursor_byte(3);
        assert_eq!(e.cursor_pos(), (0, 3));
        e.set_cursor_byte(2);
        assert_eq!(e.cursor_pos(), (0, 1), "a byte inside a char snaps to its start");
        e.set_cursor_byte(7 + 3);
        assert_eq!(e.cursor_pos(), (1, 3));
        assert_eq!(e.cursor_byte(), 10);
        key(&mut e, KeyCode::Right, NONE);
        assert_eq!(e.cursor_pos(), (1, 6));
        key(&mut e, KeyCode::Up, NONE);
        assert_eq!(e.cursor_pos(), (0, 5), "display column 4 of the CJK line maps back by width");
        e.set_cursor_byte(999);
        assert_eq!(e.cursor_byte(), e.text().len());
    }

    #[test]
    fn vertical_moves_remember_the_preferred_column() {
        let mut e = editor("abcdef\nab\nabcdef");
        e.set_cursor_byte(5);
        key(&mut e, KeyCode::Down, NONE);
        assert_eq!(e.cursor_pos(), (1, 2));
        key(&mut e, KeyCode::Down, NONE);
        assert_eq!(e.cursor_pos(), (2, 5));
    }

    #[test]
    fn smart_home_and_word_motion() {
        let mut e = editor("    select foo_bar, baz");
        key(&mut e, KeyCode::Home, NONE);
        assert_eq!(e.cursor_pos(), (0, 4));
        key(&mut e, KeyCode::Home, NONE);
        assert_eq!(e.cursor_pos(), (0, 0));
        key(&mut e, KeyCode::Right, CTRL);
        assert_eq!(e.cursor_pos(), (0, 4));
        key(&mut e, KeyCode::Right, CTRL);
        assert_eq!(e.cursor_pos(), (0, 11));
        key(&mut e, KeyCode::End, NONE);
        key(&mut e, KeyCode::Left, CTRL);
        assert_eq!(e.cursor_pos(), (0, 20));
        key(&mut e, KeyCode::End, NONE);
        key(&mut e, KeyCode::Backspace, CTRL);
        assert_eq!(e.text(), "    select foo_bar, ");
        key(&mut e, KeyCode::Backspace, CTRL);
        assert_eq!(e.text(), "    select foo_bar");
        key(&mut e, KeyCode::Backspace, CTRL);
        assert_eq!(e.text(), "    select ", "word delete removes the identifier as a unit");
        key(&mut e, KeyCode::Home, NONE);
        key(&mut e, KeyCode::Delete, CTRL);
        assert_eq!(e.text(), "    ");
    }

    #[test]
    fn duplicate_and_move_lines() {
        let mut e = editor("a\nb\nc");
        e.set_cursor_byte(2);
        key(&mut e, KeyCode::Char('d'), CTRL);
        assert_eq!(e.text(), "a\nb\nb\nc");
        assert_eq!(e.cursor_pos(), (2, 0));
        key(&mut e, KeyCode::Down, ALT);
        assert_eq!(e.text(), "a\nb\nc\nb");
        assert_eq!(e.cursor_pos(), (3, 0));
        key(&mut e, KeyCode::Down, ALT);
        assert_eq!(e.text(), "a\nb\nc\nb", "last line cannot move further down");
        e.set_cursor_byte(0);
        key(&mut e, KeyCode::Up, ALT);
        assert_eq!(e.text(), "a\nb\nc\nb");
    }

    #[test]
    fn tab_indents_and_shift_tab_dedents_selected_lines() {
        let mut e = editor("a\nb");
        key(&mut e, KeyCode::Char('a'), CTRL);
        key(&mut e, KeyCode::Tab, NONE);
        assert_eq!(e.text(), "  a\n  b");
        key(&mut e, KeyCode::BackTab, SHIFT);
        assert_eq!(e.text(), "a\nb");
        e.set_cursor_byte(1);
        key(&mut e, KeyCode::Tab, NONE);
        assert_eq!(e.text(), "a  \nb");
    }

    #[test]
    fn app_level_keys_are_left_unhandled() {
        let mut e = editor("select 1");
        for (code, m) in [
            (KeyCode::F(5), NONE),
            (KeyCode::Esc, NONE),
            (KeyCode::Enter, CTRL),
            (KeyCode::Char('e'), CTRL),
            (KeyCode::Char('p'), CTRL),
            (KeyCode::Char('t'), CTRL),
            (KeyCode::Char('w'), CTRL),
            (KeyCode::Char('l'), CTRL),
        ] {
            assert_eq!(key(&mut e, code, m), EditorEvent::Unhandled, "{code:?} {m:?}");
        }
        assert_eq!(e.text(), "select 1");
        let release = KeyEvent::new_with_kind(KeyCode::Char('x'), NONE, KeyEventKind::Release);
        assert_eq!(e.handle_key(release), EditorEvent::Unhandled);
    }

    #[test]
    fn completion_is_requested_on_ctrl_space_and_dot_after_identifier() {
        let mut e = editor("select u");
        assert_eq!(key(&mut e, KeyCode::Char(' '), CTRL), EditorEvent::RequestCompletion);
        assert_eq!(key(&mut e, KeyCode::Char('.'), NONE), EditorEvent::RequestCompletion);
        assert_eq!(e.text(), "select u.");
        let mut n = editor("select 1 ");
        assert_eq!(key(&mut n, KeyCode::Char('.'), NONE), EditorEvent::Changed);
    }

    #[test]
    fn read_only_allows_navigation_but_not_edits() {
        let mut e = editor("select 1");
        e.read_only = true;
        assert_eq!(key(&mut e, KeyCode::Char('x'), NONE), EditorEvent::Unhandled);
        assert_eq!(key(&mut e, KeyCode::Backspace, NONE), EditorEvent::Unhandled);
        assert_eq!(e.handle_paste("x"), EditorEvent::Unhandled);
        assert_eq!(key(&mut e, KeyCode::Left, NONE), EditorEvent::Moved);
        assert_eq!(e.text(), "select 1");
    }

    #[test]
    fn paste_normalizes_line_endings_and_tabs_as_one_step() {
        let mut e = Editor::new(Backend::Postgres);
        e.handle_paste("select\r\n\t1\rfrom t");
        assert_eq!(e.text(), "select\n    1\nfrom t");
        e.undo();
        assert!(e.is_empty());
    }

    #[test]
    fn render_colors_keywords_and_draws_the_gutter() {
        let mut e = editor("SELECT count(x), my_fn(1) FROM t -- hi\nwhere");
        let t = Theme::default();
        let (buf, _) = render(&mut e, 60, 4);
        assert_eq!(row_text(&buf, 0).get(..4), Some("  1 "));
        assert_eq!(row_text(&buf, 1).get(..4), Some("  2 "));
        assert_eq!(buf[(2, 1)].fg, t.accent, "current line number uses the accent");
        assert_eq!(buf[(2, 0)].fg, t.muted);

        let x0 = 4;
        for x in x0..x0 + 6 {
            assert_eq!(buf[(x, 0)].fg, t.keyword);
            assert!(buf[(x, 0)].modifier.contains(Modifier::BOLD));
        }
        assert_eq!(buf[(x0 + 7, 0)].fg, t.function, "builtin count");
        assert_eq!(buf[(x0 + 12, 0)].fg, t.punctuation);
        assert_eq!(buf[(x0 + 13, 0)].fg, t.identifier);
        assert_eq!(buf[(x0 + 17, 0)].fg, t.function, "identifier followed by ( is a call");
        assert_eq!(buf[(x0 + 23, 0)].fg, t.number);
        assert_eq!(buf[(x0 + 33, 0)].fg, t.comment);
        assert!(buf[(x0 + 33, 0)].modifier.contains(Modifier::ITALIC));
        assert_eq!(buf[(x0, 1)].fg, t.keyword, "lowercase keywords too");
        assert_eq!(buf[(x0, 1)].bg, t.highlight, "focused current line is highlighted");
        assert_eq!(buf[(x0, 0)].bg, t.bg);
        assert_eq!(buf[(0, 3)].symbol(), " ", "rows past the end stay blank");
    }

    #[test]
    fn render_marks_selection_error_and_matching_bracket() {
        let t = Theme::default();
        let mut e = editor("select (1) frm");
        e.set_cursor_byte(10);
        e.set_error_marker(Some(11));
        let (buf, _) = render(&mut e, 40, 2);
        let x0 = 4;
        assert_eq!(buf[(x0 + 7, 0)].bg, t.border, "partner of the bracket before the cursor");
        assert_eq!(buf[(x0 + 9, 0)].bg, t.border);
        assert_eq!(buf[(x0 + 8, 0)].bg, t.highlight);
        let err = &buf[(x0 + 11, 0)];
        assert_eq!(err.fg, t.error);
        assert!(err.modifier.contains(Modifier::UNDERLINED | Modifier::REVERSED));

        typ(&mut e, " ");
        let (buf, _) = render(&mut e, 40, 2);
        assert!(!buf[(x0 + 12, 0)].modifier.contains(Modifier::REVERSED), "edits clear the error marker");

        key(&mut e, KeyCode::Home, SHIFT);
        let (buf, _) = render(&mut e, 40, 2);
        assert_eq!(buf[(x0, 0)].bg, t.selection);
        assert_eq!(buf[(x0 + 10, 0)].bg, t.selection);
    }

    #[test]
    fn empty_editor_shows_placeholder() {
        let t = Theme::default();
        let mut e = Editor::new(Backend::Sqlite);
        let (buf, _) = render(&mut e, 30, 2);
        assert!(row_text(&buf, 0)[4..].starts_with("-- Write SQL"));
        assert_eq!(buf[(5, 0)].fg, t.muted);
        assert!(buf[(5, 0)].modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn horizontal_scroll_keeps_the_cursor_visible() {
        let long = format!("select '{}' as v", "x".repeat(100));
        let mut e = editor(&long);
        let (buf, area) = render(&mut e, 24, 3);
        let (cx, cy) = e.cursor_screen_position(area).expect("cursor visible after scrolling");
        assert_eq!(cy, 0);
        assert!(cx < 24);
        assert_eq!(buf[(cx - 1, 0)].symbol(), "v", "the char left of the cursor is the line's last");
        assert_eq!(row_text(&buf, 0).get(..4), Some("  1 "), "gutter does not scroll");

        key(&mut e, KeyCode::Home, NONE);
        let (buf, area) = render(&mut e, 24, 3);
        assert_eq!(e.cursor_screen_position(area), Some((4, 0)));
        assert_eq!(buf[(4, 0)].symbol(), "s");
    }

    #[test]
    fn wide_characters_occupy_two_cells_and_scroll_correctly() {
        let mut e = editor("'日本語' x");
        let (buf, area) = render(&mut e, 30, 2);
        assert_eq!(buf[(5, 0)].symbol(), "日");
        assert_eq!(buf[(7, 0)].symbol(), "本");
        assert_eq!(e.cursor_screen_position(area), Some((4 + 10, 0)));
        assert_eq!(buf[(5, 0)].fg, Theme::default().string);
    }

    #[test]
    fn vertical_scroll_follows_cursor_and_wheel_scrolls_freely() {
        let text = (1..=100).map(|i| format!("select {i};")).collect::<Vec<_>>().join("\n");
        let mut e = editor(&text);
        let (buf, area) = render(&mut e, 30, 10);
        assert!(row_text(&buf, 9).contains("100"), "cursor at the end scrolls the last line into view");
        assert_eq!(e.cursor_screen_position(area), Some((5 + 11, 9)), "gutter widens for 3-digit line numbers");

        let wheel = |kind| MouseEvent { kind, column: 5, row: 5, modifiers: NONE };
        for _ in 0..40 {
            e.handle_mouse(wheel(MouseEventKind::ScrollUp), area);
        }
        let (buf, _) = render(&mut e, 30, 10);
        assert!(row_text(&buf, 0).starts_with("   1 select 1;"), "wheel scrolling is not undone by the cursor");
        assert_eq!(e.cursor_screen_position(area), None);
    }

    #[test]
    fn mouse_click_places_cursor_and_drag_selects() {
        let mut e = editor("select a\nfrom t");
        let (_, area) = render(&mut e, 30, 5);
        let ev = |kind, column, row| MouseEvent { kind, column, row, modifiers: NONE };
        e.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), 4 + 2, 0), area);
        assert_eq!(e.cursor_pos(), (0, 2));
        e.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 4 + 4, 1), area);
        e.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), 4 + 4, 1), area);
        assert_eq!(e.selected_text().as_deref(), Some("lect a\nfrom"));
        e.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), 29, 4), area);
        assert_eq!(e.cursor_pos(), (1, 6), "clicks past the text clamp to the last line end");
        e.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), 29, 4), area);
        assert_eq!(e.selected_text(), None);
        assert_eq!(
            e.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), 40, 0), area),
            EditorEvent::Unhandled,
            "clicks outside the editor belong to the app"
        );
    }

    #[test]
    fn tokens_are_cached_until_the_text_changes() {
        let mut e = editor("select 1");
        render(&mut e, 20, 2);
        let g = e.highlight.as_ref().unwrap().generation;
        key(&mut e, KeyCode::Left, NONE);
        render(&mut e, 20, 2);
        assert_eq!(e.highlight.as_ref().unwrap().generation, g, "cursor movement does not re-tokenize");
        typ(&mut e, "x");
        render(&mut e, 20, 2);
        assert_ne!(e.highlight.as_ref().unwrap().generation, g);
        e.set_backend(Backend::MySql);
        render(&mut e, 20, 2);
        assert_eq!(e.highlight.as_ref().unwrap().generation, e.generation);
    }

    #[test]
    #[ignore = "timing; run with --release --ignored"]
    fn rendering_a_large_buffer_is_fast() {
        let text = (0..10_000)
            .map(|i| format!("select id, name, count(*) from table_{i} where x = 'v{i}' group by 1; -- c"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut e = editor(&text);
        let area = Rect::new(0, 0, 200, 60);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        e.render(area, &mut buf, &theme, true);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            key(&mut e, KeyCode::Up, NONE);
            e.render(area, &mut buf, &theme, true);
        }
        let per = start.elapsed() / 100;
        assert!(per.as_micros() < 2000, "render took {per:?}");
        assert_ne!(buf[(10, 10)].fg, Color::Reset);
    }
}
