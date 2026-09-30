use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::ops::Range;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::db::{Column, Row, TypeKind, Value};
use crate::theme::Theme;

const MIN_WIDTH: u16 = 3;
const AUTO_MAX_WIDTH: u16 = 60;
const FIT_MAX_WIDTH: u16 = 120;
const RESIZE_MAX_WIDTH: u16 = 250;
const RESIZE_STEP: u16 = 2;
/// Rows measured by `set_data`; streamed batches keep being measured until `SAMPLE_LIMIT`.
const SAMPLE_ROWS: usize = 1000;
const SAMPLE_LIMIT: usize = 10_000;
const BYTES_PREVIEW: usize = 32;
const WHEEL_ROWS: usize = 3;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyKind {
    /// The selected rectangle (or the cursor cell).
    Cells,
    /// Whole rows covered by the selection (or the cursor row).
    Rows,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GridEvent {
    /// Consumed by the grid; nothing for the app to do beyond redrawing.
    Handled,
    Unhandled,
    StartSearch,
    /// `n`/`N` found no match for the active search.
    NoMatch,
    Copy(CopyKind),
    OpenCell(usize, usize),
    SortRequested(usize),
    EditCell(usize, usize),
    InsertRow,
    DeleteRows(Range<usize>),
    FilterRequested(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectMode {
    /// Shift+move / mouse drag: cleared by the next plain move.
    Extend,
    Visual,
    VisualRows,
}

#[derive(Clone, Copy, Debug)]
struct Selection {
    anchor: (usize, usize),
    mode: SelectMode,
}

/// Geometry of the last render, used for paging and mouse hit-testing.
#[derive(Debug, Default)]
struct Layout {
    header: Rect,
    body: Rect,
    /// (column index, x, span width including padding) of every drawn column.
    cols: Vec<(usize, u16, u16)>,
    page_rows: usize,
    full_cols: usize,
}

#[derive(Debug)]
pub struct GridState {
    columns: Vec<Column>,
    rows: Vec<Row>,
    widths: Vec<u16>,
    manual: Vec<bool>,
    sampled: usize,
    cursor: (usize, usize),
    selection: Option<Selection>,
    row_offset: usize,
    col_offset: usize,
    follow_cursor: bool,
    sort: Option<(usize, bool)>,
    edited: HashSet<(usize, usize)>,
    deleted: HashSet<usize>,
    new_rows: HashSet<usize>,
    search: Option<String>,
    empty_message: String,
    layout: Layout,
    last_click: Option<(Instant, (usize, usize))>,
    drag_anchor: Option<(usize, usize)>,
    /// Shown for NULL cells. Set before `set_data` so column widths account for it.
    pub null_text: String,
    /// Absolute index of the first row (pagination): the gutter shows `row_offset_label + i + 1`.
    pub row_offset_label: u64,
    /// Adds a muted second header line with each column's type name (set before `set_data` to size for it).
    pub show_types: bool,
}

impl Default for GridState {
    fn default() -> Self {
        GridState {
            columns: Vec::new(),
            rows: Vec::new(),
            widths: Vec::new(),
            manual: Vec::new(),
            sampled: 0,
            cursor: (0, 0),
            selection: None,
            row_offset: 0,
            col_offset: 0,
            follow_cursor: true,
            sort: None,
            edited: HashSet::new(),
            deleted: HashSet::new(),
            new_rows: HashSet::new(),
            search: None,
            empty_message: "No rows".into(),
            layout: Layout { page_rows: 20, ..Layout::default() },
            last_click: None,
            drag_anchor: None,
            null_text: "NULL".into(),
            row_offset_label: 0,
            show_types: false,
        }
    }
}

impl GridState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the data. When the column names are unchanged (refresh, re-sort, next page) the
    /// cursor column, horizontal scroll and user-resized widths are kept; otherwise view state resets.
    pub fn set_data(&mut self, columns: Vec<Column>, rows: Vec<Row>) {
        let same =
            columns.len() == self.columns.len() && columns.iter().zip(&self.columns).all(|(a, b)| a.name == b.name);
        let old = std::mem::take(&mut self.widths);
        let old_manual = std::mem::take(&mut self.manual);
        self.columns = columns;
        self.rows = rows;
        self.clear_marks();
        self.selection = None;
        self.drag_anchor = None;
        self.follow_cursor = true;
        if same {
            self.cursor.0 = self.cursor.0.min(self.rows.len().saturating_sub(1));
        } else {
            self.cursor = (0, 0);
            self.row_offset = 0;
            self.col_offset = 0;
        }
        if self.sort.is_some_and(|(c, _)| c >= self.columns.len()) {
            self.sort = None;
        }
        let types = self.show_types;
        self.widths = self
            .columns
            .iter()
            .map(|c| header_width(&c.name).max(if types { header_width(&c.type_name) } else { 0 }))
            .collect();
        self.manual = vec![false; self.columns.len()];
        if same {
            for (i, &m) in old_manual.iter().enumerate().filter(|(_, m)| **m) {
                self.widths[i] = old[i];
                self.manual[i] = m;
            }
        }
        self.sampled = 0;
        self.measure(0..self.rows.len().min(SAMPLE_ROWS));
    }

    /// Appends a streamed batch; widths grow from the new rows while under the sampling budget.
    pub fn push_rows(&mut self, rows: Vec<Row>) {
        let start = self.rows.len();
        self.rows.extend(rows);
        let end = self.rows.len();
        let budget = SAMPLE_LIMIT.saturating_sub(self.sampled);
        if budget > 0 {
            self.measure(start..end.min(start + budget));
        } else if end > start {
            // Past the sample budget, the edges of each batch still catch growing values (ids, counters).
            self.measure(start..start + 1);
            self.measure(end - 1..end);
        }
    }

    /// Replaces one cell in place (staged table edits).
    pub fn set_cell(&mut self, row: usize, col: usize, value: Value) {
        if let Some(cell) = self.rows.get_mut(row).and_then(|r| r.get_mut(col)) {
            *cell = value;
            self.measure(row..row + 1);
        }
    }

    pub fn clear(&mut self) {
        self.columns.clear();
        self.rows.clear();
        self.widths.clear();
        self.manual.clear();
        self.sampled = 0;
        self.cursor = (0, 0);
        self.selection = None;
        self.row_offset = 0;
        self.col_offset = 0;
        self.follow_cursor = true;
        self.sort = None;
        self.drag_anchor = None;
        self.clear_marks();
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    pub fn column_width(&self, col: usize) -> Option<u16> {
        self.widths.get(col).copied()
    }

    pub fn selected_cell(&self) -> Option<(usize, usize)> {
        (!self.rows.is_empty() && !self.columns.is_empty()).then_some(self.cursor)
    }

    pub fn selected_value(&self) -> Option<&Value> {
        let (r, c) = self.selected_cell()?;
        self.rows.get(r)?.get(c)
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected_cell()?.0)
    }

    /// Selected rows and columns; the cursor cell alone when nothing is selected.
    pub fn selection_range(&self) -> Option<(Range<usize>, Range<usize>)> {
        let (r, c) = self.selected_cell()?;
        let Some(sel) = self.selection else {
            return Some((r..r + 1, c..c + 1));
        };
        let (ar, ac) = sel.anchor;
        let rows = ar.min(r)..ar.max(r) + 1;
        let cols = match sel.mode {
            SelectMode::VisualRows => 0..self.columns.len(),
            _ => ac.min(c)..ac.max(c) + 1,
        };
        Some((rows, cols))
    }

    /// Rows drawn by the last render.
    pub fn visible_row_range(&self) -> Range<usize> {
        let start = self.row_offset.min(self.rows.len());
        start..(start + self.layout.page_rows).min(self.rows.len())
    }

    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.selection = None;
        self.move_to(row, col, false);
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Only shows ▲/▼ in the header; sorting is done by the caller.
    pub fn set_sort(&mut self, col: usize, asc: bool) {
        self.sort = Some((col, asc));
    }

    pub fn clear_sort(&mut self) {
        self.sort = None;
    }

    pub fn sort(&self) -> Option<(usize, bool)> {
        self.sort
    }

    pub fn mark_edited(&mut self, row: usize, col: usize) {
        self.edited.insert((row, col));
    }

    pub fn mark_deleted_row(&mut self, row: usize) {
        self.deleted.insert(row);
    }

    pub fn mark_new_row(&mut self, row: usize) {
        self.new_rows.insert(row);
    }

    pub fn clear_marks(&mut self) {
        self.edited.clear();
        self.deleted.clear();
        self.new_rows.clear();
    }

    /// Case-insensitive; matching cells are highlighted. Does not move the cursor (see `search_next`).
    pub fn set_search(&mut self, query: Option<String>) {
        self.search = query.filter(|q| !q.is_empty()).map(|q| q.to_lowercase());
    }

    pub fn set_empty_message(&mut self, message: impl Into<String>) {
        self.empty_message = message.into();
    }

    /// Moves the cursor to the next (or previous) matching cell in row-major order, wrapping.
    pub fn search_next(&mut self, forward: bool) -> bool {
        let Some(needle) = self.search.as_deref() else { return false };
        let ncols = self.columns.len();
        let total = self.rows.len() * ncols;
        if total == 0 {
            return false;
        }
        let start = self.cursor.0 * ncols + self.cursor.1;
        let found = (1..=total)
            .map(|k| if forward { (start + k) % total } else { (start + total - k) % total })
            .find(|&i| self.rows[i / ncols].get(i % ncols).is_some_and(|v| value_matches(v, needle)));
        match found {
            Some(i) => {
                self.move_to(i / ncols, i % ncols, false);
                true
            }
            None => false,
        }
    }

    fn measure(&mut self, range: Range<usize>) {
        self.sampled += range.len();
        let cap = AUTO_MAX_WIDTH as usize;
        for row in &self.rows[range] {
            for (c, v) in row.iter().enumerate().take(self.widths.len()) {
                if self.manual[c] || self.widths[c] >= AUTO_MAX_WIDTH {
                    continue;
                }
                let w = text_width(&cell_text(v, &self.null_text), cap) as u16;
                self.widths[c] = self.widths[c].max(w.clamp(MIN_WIDTH, AUTO_MAX_WIDTH));
            }
        }
    }

    fn fit_column(&mut self, col: usize) {
        let cap = FIT_MAX_WIDTH as usize;
        let mut w = text_width(&self.columns[col].name, cap) + 2;
        for row in &self.rows {
            if let Some(v) = row.get(col) {
                w = w.max(text_width(&cell_text(v, &self.null_text), cap));
            }
            if w >= cap {
                break;
            }
        }
        self.widths[col] = (w as u16).clamp(MIN_WIDTH, FIT_MAX_WIDTH);
        self.manual[col] = true;
        self.follow_cursor = true;
    }

    fn resize_column(&mut self, col: usize, grow: bool) {
        let w = self.widths[col];
        let w = if grow { w.saturating_add(RESIZE_STEP) } else { w.saturating_sub(RESIZE_STEP) };
        self.widths[col] = w.clamp(MIN_WIDTH, RESIZE_MAX_WIDTH);
        self.manual[col] = true;
        self.follow_cursor = true;
    }

    fn move_to(&mut self, row: usize, col: usize, extend: bool) {
        let row = row.min(self.rows.len().saturating_sub(1));
        let col = col.min(self.columns.len().saturating_sub(1));
        if extend {
            if self.selection.is_none() {
                self.selection = Some(Selection { anchor: self.cursor, mode: SelectMode::Extend });
            }
        } else if self.selection.is_some_and(|s| s.mode == SelectMode::Extend) {
            self.selection = None;
        }
        self.cursor = (row, col);
        self.follow_cursor = true;
    }

    fn toggle_visual(&mut self, mode: SelectMode) {
        self.selection = match self.selection {
            Some(s) if s.mode == mode => None,
            Some(s) => Some(Selection { mode, ..s }),
            None => Some(Selection { anchor: self.cursor, mode }),
        };
    }

    fn scroll_rows(&mut self, down: bool, n: usize) {
        let max = self.rows.len().saturating_sub(self.layout.page_rows);
        self.row_offset = if down { (self.row_offset + n).min(max) } else { self.row_offset.saturating_sub(n) };
        self.follow_cursor = false;
    }

    fn scroll_cols(&mut self, right: bool) {
        let max = self.columns.len().saturating_sub(1);
        self.col_offset = if right { (self.col_offset + 1).min(max) } else { self.col_offset.saturating_sub(1) };
        self.follow_cursor = false;
    }

    /// Keys: hjkl/arrows, PgUp/PgDn, Ctrl-U/D, Home/End/0/$, g/G, Ctrl-Home/End, Tab/Shift-Tab,
    /// w/b (page of columns), Shift+arrows, v/V, Esc, </>/=, n/N, /, y/Y, Enter, s, e/F2, o, D/Delete, f.
    pub fn handle_key(&mut self, key: KeyEvent) -> GridEvent {
        use KeyCode::*;
        if key.kind == KeyEventKind::Release
            || self.columns.is_empty()
            || key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return GridEvent::Unhandled;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let (r, c) = self.cursor;
        let has_rows = !self.rows.is_empty();
        let last_row = self.rows.len().saturating_sub(1);
        let last_col = self.columns.len() - 1;
        let page = self.layout.page_rows.max(1);
        let half = (page / 2).max(1);
        let jump = self.layout.full_cols.max(1);

        if ctrl {
            match key.code {
                Char('u') => self.move_to(r.saturating_sub(half), c, false),
                Char('d') => self.move_to(r + half, c, false),
                Home => self.move_to(0, c, shift),
                End => self.move_to(last_row, c, shift),
                _ => return GridEvent::Unhandled,
            }
            return GridEvent::Handled;
        }

        match key.code {
            Up | Char('k') => self.move_to(r.saturating_sub(1), c, shift && key.code == Up),
            Down | Char('j') => self.move_to(r + 1, c, shift && key.code == Down),
            Left | Char('h') => self.move_to(r, c.saturating_sub(1), shift && key.code == Left),
            Right | Char('l') => self.move_to(r, c + 1, shift && key.code == Right),
            PageUp => self.move_to(r.saturating_sub(page), c, shift),
            PageDown => self.move_to(r + page, c, shift),
            Home | Char('0') => self.move_to(r, 0, shift && key.code == Home),
            End | Char('$') => self.move_to(r, last_col, shift && key.code == End),
            Char('g') => self.move_to(0, c, false),
            Char('G') => self.move_to(last_row, c, false),
            Tab if c == last_col && r < last_row => self.move_to(r + 1, 0, false),
            Tab => self.move_to(r, c + 1, false),
            BackTab if c == 0 && r > 0 => self.move_to(r - 1, last_col, false),
            BackTab => self.move_to(r, c.saturating_sub(1), false),
            Char('w') => self.move_to(r, c + jump, false),
            Char('b') => self.move_to(r, c.saturating_sub(jump), false),
            Char('v') => self.toggle_visual(SelectMode::Visual),
            Char('V') => self.toggle_visual(SelectMode::VisualRows),
            Esc if self.selection.is_some() => self.selection = None,
            Char('<') => self.resize_column(c, false),
            Char('>') => self.resize_column(c, true),
            Char('=') => self.fit_column(c),
            Char('n') | Char('N') if self.search.is_some() => {
                if !self.search_next(key.code == Char('n')) {
                    return GridEvent::NoMatch;
                }
            }
            Char('/') => return GridEvent::StartSearch,
            Char('y') if has_rows => return GridEvent::Copy(CopyKind::Cells),
            Char('Y') if has_rows => return GridEvent::Copy(CopyKind::Rows),
            Enter if has_rows => return GridEvent::OpenCell(r, c),
            Char('s') => return GridEvent::SortRequested(c),
            Char('e') | F(2) if has_rows => return GridEvent::EditCell(r, c),
            Char('o') => return GridEvent::InsertRow,
            Char('D') | Delete if has_rows => {
                let (rows, _) = self.selection_range().unwrap_or((r..r + 1, c..c + 1));
                return GridEvent::DeleteRows(rows);
            }
            Char('f') => return GridEvent::FilterRequested(c),
            _ => return GridEvent::Unhandled,
        }
        GridEvent::Handled
    }

    /// `area` is the rect last passed to `render`. Click selects (header click requests a sort),
    /// drag selects a range, double-click opens the cell, wheel scrolls (Shift+wheel horizontally).
    pub fn handle_mouse(&mut self, ev: MouseEvent, area: Rect) -> GridEvent {
        if self.columns.is_empty() {
            return GridEvent::Unhandled;
        }
        let pos = Position { x: ev.column, y: ev.row };
        let dragging = self.drag_anchor.is_some() && matches!(ev.kind, MouseEventKind::Drag(_) | MouseEventKind::Up(_));
        if !area.contains(pos) && !dragging {
            return GridEvent::Unhandled;
        }
        let shift = ev.modifiers.contains(KeyModifiers::SHIFT);
        match ev.kind {
            MouseEventKind::ScrollDown if shift => self.scroll_cols(true),
            MouseEventKind::ScrollUp if shift => self.scroll_cols(false),
            MouseEventKind::ScrollRight => self.scroll_cols(true),
            MouseEventKind::ScrollLeft => self.scroll_cols(false),
            MouseEventKind::ScrollDown => self.scroll_rows(true, WHEEL_ROWS),
            MouseEventKind::ScrollUp => self.scroll_rows(false, WHEEL_ROWS),
            MouseEventKind::Down(MouseButton::Left) => {
                if self.layout.header.contains(pos) {
                    return match self.col_at(pos.x) {
                        Some(c) => GridEvent::SortRequested(c),
                        None => GridEvent::Handled,
                    };
                }
                if !self.layout.body.contains(pos) {
                    return GridEvent::Handled;
                }
                let Some(cell) = self.cell_at(pos, false) else { return GridEvent::Handled };
                let now = Instant::now();
                if let Some((t, prev)) = self.last_click.take()
                    && prev == cell
                    && now.duration_since(t) <= DOUBLE_CLICK
                {
                    self.drag_anchor = None;
                    return GridEvent::OpenCell(cell.0, cell.1);
                }
                self.last_click = Some((now, cell));
                if !shift {
                    self.selection = None;
                }
                self.move_to(cell.0, cell.1, shift);
                self.drag_anchor = Some(self.selection.map_or(cell, |s| s.anchor));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(anchor) = self.drag_anchor else { return GridEvent::Handled };
                let Some(cell) = self.cell_at(pos, true) else { return GridEvent::Handled };
                if cell != anchor && self.selection.is_none() {
                    self.selection = Some(Selection { anchor, mode: SelectMode::Extend });
                }
                self.cursor = cell;
                self.follow_cursor = true;
            }
            MouseEventKind::Up(MouseButton::Left) => self.drag_anchor = None,
            _ => return GridEvent::Unhandled,
        }
        GridEvent::Handled
    }

    fn col_at(&self, x: u16) -> Option<usize> {
        self.layout.cols.iter().find(|(_, cx, span)| x >= *cx && x <= cx + span).map(|(c, ..)| *c)
    }

    /// With `clamp`, positions outside the body map to the nearest cell (just past the edge while
    /// dragging, so the view scrolls).
    fn cell_at(&self, pos: Position, clamp: bool) -> Option<(usize, usize)> {
        let body = self.layout.body;
        if self.rows.is_empty() {
            return None;
        }
        let row = if pos.y < body.y {
            if !clamp {
                return None;
            }
            self.row_offset.saturating_sub(1)
        } else if pos.y >= body.bottom() {
            if !clamp {
                return None;
            }
            self.row_offset + body.height as usize
        } else {
            self.row_offset + (pos.y - body.y) as usize
        };
        let col = match self.col_at(pos.x) {
            Some(c) => c,
            None if clamp => {
                let first = self.layout.cols.first()?;
                let last = self.layout.cols.last()?;
                if pos.x < first.1 { first.0 } else { last.0 }
            }
            None => return None,
        };
        if row >= self.rows.len() && !clamp {
            return None;
        }
        Some((row.min(self.rows.len() - 1), col))
    }

    /// Draws the grid into `area` and records its geometry for paging and mouse hit-testing.
    /// Only visible cells are formatted. `focused` controls the cursor emphasis.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme, focused: bool) {
        let area = area.intersection(*buf.area());
        let pal = Palette::new(theme, focused);
        buf.set_style(area, pal.base);
        self.layout.cols.clear();
        self.layout.header = Rect::default();
        self.layout.body = Rect::default();
        self.layout.full_cols = 0;
        if area.is_empty() {
            return;
        }
        if self.columns.is_empty() {
            draw_centered(buf, area, &self.empty_message, pal.muted);
            return;
        }

        let header_h = if self.show_types { 3 } else { 2 }.min(area.height);
        let header = Rect { height: header_h, ..area };
        let body = Rect { y: area.y + header_h, height: area.height - header_h, ..area };
        let nrows = self.rows.len();
        let page = body.height as usize;
        let right = area.right() - u16::from(nrows > page && area.width > 1);
        let digits = decimal_digits(self.row_offset_label + nrows as u64);
        let gutter_x = area.x + 1;
        let sep_x = area.x + digits + 2;
        let x0 = sep_x + 1;
        let data_w = right.saturating_sub(x0);
        self.layout.header = header;
        self.layout.body = body;
        self.layout.page_rows = page.max(1);
        self.scroll_into_view(page, data_w);

        let mut x = x0;
        for c in self.col_offset..self.columns.len() {
            if x >= right {
                break;
            }
            let full = self.widths[c] + 2;
            let span = full.min(right - x);
            self.layout.cols.push((c, x, span));
            self.layout.full_cols += usize::from(span == full);
            x = x.saturating_add(full + 1);
        }
        let seps: Vec<u16> = std::iter::once(sep_x)
            .chain(
                self.layout
                    .cols
                    .iter()
                    .filter(|&&(c, _, span)| span == self.widths[c] + 2)
                    .map(|&(_, x, span)| x + span),
            )
            .filter(|&x| x < right)
            .collect();
        let hidden_left = self.col_offset > 0;
        let hidden_right = self
            .layout
            .cols
            .last()
            .is_some_and(|&(c, _, span)| c + 1 < self.columns.len() || span < self.widths[c] + 2);

        let header_area = Rect { width: right - area.x, ..header };
        self.render_header(buf, header_area, &seps, &pal, (hidden_left, hidden_right), digits);

        if nrows == 0 {
            draw_centered(buf, body, &self.empty_message, pal.muted);
            return;
        }

        let range = self.selection_range();
        let (cur_r, cur_c) = self.cursor;
        for (i, r) in (self.row_offset..nrows).take(page).enumerate() {
            let y = body.y + i as u16;
            let row_rect = Rect::new(area.x, y, right - area.x, 1);
            let row_bg = if r == cur_r {
                Some(pal.current_row)
            } else if r % 2 == 1 {
                pal.zebra
            } else {
                None
            };
            if let Some(bg) = row_bg {
                buf.set_style(row_rect, Style::new().bg(bg));
            }
            let deleted = !self.deleted.is_empty() && self.deleted.contains(&r);
            let new = !self.new_rows.is_empty() && self.new_rows.contains(&r);

            let (marker, marker_style) = match (deleted, new) {
                (true, _) => ("-", pal.error),
                (_, true) => ("+", pal.success),
                _ => (" ", pal.muted),
            };
            buf[(area.x, y)].set_symbol(marker).set_style(marker_style);
            let num_style = if r == cur_r {
                pal.gutter_current
            } else if new {
                pal.success
            } else {
                pal.muted
            };
            let label = (self.row_offset_label + r as u64 + 1).to_string();
            let gutter = Rect::new(gutter_x, y, digits.min(right.saturating_sub(gutter_x)), 1);
            draw_text(buf, gutter, &label, num_style, num_style, true);
            for &sx in &seps {
                buf[(sx, y)].set_symbol("│").set_style(pal.border);
            }

            let row = &self.rows[r];
            for &(c, x, span) in &self.layout.cols {
                let Some(value) = row.get(c) else { continue };
                let column = &self.columns[c];
                let tone = Tone::of(value, column.kind);
                let is_cursor = r == cur_r && c == cur_c;
                let selected = range.as_ref().is_some_and(|(rs, cs)| rs.contains(&r) && cs.contains(&c));
                let matched =
                    !is_cursor && !selected && self.search.as_deref().is_some_and(|n| value_matches(value, n));
                let bg = if is_cursor {
                    Some(pal.cursor)
                } else if selected {
                    Some(pal.selection)
                } else if matched {
                    Some(pal.search)
                } else {
                    None
                };
                if let Some(bg) = bg {
                    buf.set_style(Rect::new(x, y, span, 1), Style::new().bg(bg));
                }
                let mut style = pal.tone(tone);
                if deleted {
                    style = style.fg(theme.error).add_modifier(Modifier::CROSSED_OUT);
                } else if !self.edited.is_empty() && self.edited.contains(&(r, c)) {
                    style = style.fg(theme.warning).add_modifier(Modifier::UNDERLINED);
                }
                if matched {
                    style = style.fg(pal.search_fg);
                }
                if is_cursor && focused {
                    style = style.add_modifier(Modifier::BOLD);
                }
                let right_align = tone == Tone::Number || (tone == Tone::Null && column.kind.is_numeric());
                let content_w = self.widths[c].min(span.saturating_sub(1));
                let text = cell_text(value, &self.null_text);
                draw_text(buf, Rect::new(x + 1, y, content_w, 1), &text, style, pal.special, right_align);
            }
        }

        if right < area.right() && page > 0 {
            let mut state =
                ScrollbarState::new(nrows - page + 1).position(self.row_offset).viewport_content_length(page);
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .track_style(pal.border)
                .thumb_symbol("┃")
                .thumb_style(pal.thumb)
                .render(Rect::new(right, body.y, 1, body.height), buf, &mut state);
        }
    }

    /// `area` spans the header lines, excluding the scrollbar column.
    fn render_header(
        &self,
        buf: &mut Buffer,
        area: Rect,
        seps: &[u16],
        pal: &Palette,
        (hidden_left, hidden_right): (bool, bool),
        digits: u16,
    ) {
        let (y, right, header_h) = (area.y, area.right(), area.height);
        let types_line = self.show_types && header_h >= 3;
        let label_rows = if types_line { 2 } else { 1 }.min(header_h);
        let gutter = Rect::new(area.x + 1, y, digits.min(right.saturating_sub(area.x + 1)), 1);
        draw_text(buf, gutter, "#", pal.muted, pal.muted, true);
        for dy in 0..label_rows {
            for &sx in seps {
                buf[(sx, y + dy)].set_symbol("│").set_style(pal.border);
            }
        }
        for &(c, x, span) in &self.layout.cols {
            let col = &self.columns[c];
            let content_w = self.widths[c].min(span.saturating_sub(1));
            let right_align = col.kind.is_numeric();
            match self.sort {
                Some((sc, asc)) if sc == c && content_w >= 3 => {
                    draw_text(
                        buf,
                        Rect::new(x + 1, y, content_w - 2, 1),
                        &col.name,
                        pal.header,
                        pal.header,
                        right_align,
                    );
                    buf[(x + content_w, y)].set_symbol(if asc { "▲" } else { "▼" }).set_style(pal.sort);
                }
                _ => draw_text(buf, Rect::new(x + 1, y, content_w, 1), &col.name, pal.header, pal.header, right_align),
            }
            if types_line {
                let at = Rect::new(x + 1, y + 1, content_w, 1);
                draw_text(buf, at, &col.type_name, pal.type_hint, pal.type_hint, right_align);
            }
        }
        if header_h < 2 {
            return;
        }
        let ry = y + header_h - 1;
        for x in area.x..right {
            buf[(x, ry)].set_symbol("─").set_style(pal.border);
        }
        for &sx in seps {
            buf[(sx, ry)].set_symbol("┼");
        }
        let x0 = area.x + digits + 3;
        if hidden_left && x0 < right {
            buf[(x0, ry)].set_symbol("◂").set_style(pal.hint);
        }
        if hidden_right && right > x0 + 1 {
            buf[(right - 1, ry)].set_symbol("▸").set_style(pal.hint);
        }
    }

    fn scroll_into_view(&mut self, page: usize, data_w: u16) {
        let (r, c) = self.cursor;
        if self.follow_cursor {
            if r < self.row_offset {
                self.row_offset = r;
            } else if page > 0 && r >= self.row_offset + page {
                self.row_offset = r + 1 - page;
            }
            if c < self.col_offset {
                self.col_offset = c;
            } else {
                let cost = |w: u16| w as usize + 3;
                let mut total: usize = self.widths[self.col_offset..=c].iter().map(|&w| cost(w)).sum();
                while self.col_offset < c && total > data_w as usize + 1 {
                    total -= cost(self.widths[self.col_offset]);
                    self.col_offset += 1;
                }
            }
            self.follow_cursor = false;
        }
        self.row_offset = self.row_offset.min(self.rows.len().saturating_sub(page));
        self.col_offset = self.col_offset.min(self.columns.len().saturating_sub(1));
    }

    fn selected_rows(&self) -> Range<usize> {
        self.selection_range().map_or(0..0, |(rows, _)| rows)
    }

    /// Selection as tab-separated values. A single cell without header is copied verbatim;
    /// otherwise fields containing tabs, newlines or quotes are quoted spreadsheet-style. NULL → empty.
    pub fn copy_cells_tsv(&self, include_header: bool) -> String {
        let Some((rows, cols)) = self.selection_range() else { return String::new() };
        if !include_header && rows.len() == 1 && cols.len() == 1 {
            return self.rows[rows.start].get(cols.start).map_or(String::new(), |v| v.display().into_owned());
        }
        self.tsv(rows, cols, include_header)
    }

    /// Whole selected rows (all columns) as TSV — the `Y` copy.
    pub fn copy_rows_tsv(&self, include_header: bool) -> String {
        self.tsv(self.selected_rows(), 0..self.columns.len(), include_header)
    }

    fn tsv(&self, rows: Range<usize>, cols: Range<usize>, include_header: bool) -> String {
        let mut lines = Vec::with_capacity(rows.len() + 1);
        if include_header {
            let names: Vec<_> = self.columns[cols.clone()].iter().map(|c| tsv_field(&c.name)).collect();
            lines.push(names.join("\t"));
        }
        for row in &self.rows[rows] {
            let fields: Vec<String> = cols
                .clone()
                .map(|c| row.get(c).map_or_else(String::new, |v| tsv_field(&v.display()).into_owned()))
                .collect();
            lines.push(fields.join("\t"));
        }
        lines.join("\n")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Text,
    Number,
    Bool,
    Temporal,
    Json,
    Null,
    Bytes,
}

impl Tone {
    fn of(v: &Value, kind: TypeKind) -> Tone {
        match v {
            Value::Null => Tone::Null,
            Value::Int(_) | Value::UInt(_) | Value::Float(_) => Tone::Number,
            Value::Bool(_) => Tone::Bool,
            Value::Bytes(_) => Tone::Bytes,
            Value::Text(_) if kind.is_numeric() => Tone::Number,
            Value::Text(_) if kind.is_temporal() => Tone::Temporal,
            Value::Text(_) => match kind {
                TypeKind::Bool => Tone::Bool,
                TypeKind::Json => Tone::Json,
                _ => Tone::Text,
            },
        }
    }
}

struct Palette {
    base: Style,
    muted: Style,
    border: Style,
    header: Style,
    type_hint: Style,
    sort: Style,
    hint: Style,
    thumb: Style,
    special: Style,
    error: Style,
    success: Style,
    gutter_current: Style,
    current_row: Color,
    zebra: Option<Color>,
    cursor: Color,
    selection: Color,
    search: Color,
    search_fg: Color,
    text: Style,
    number: Style,
    boolean: Style,
    temporal: Style,
    json: Style,
    null: Style,
}

impl Palette {
    fn new(t: &Theme, focused: bool) -> Palette {
        let fg = |c: Color| Style::new().fg(c);
        Palette {
            base: Style::new().fg(t.fg).bg(t.bg),
            muted: fg(t.muted),
            border: fg(t.border),
            header: fg(t.header).add_modifier(Modifier::BOLD),
            type_hint: fg(t.muted).add_modifier(Modifier::ITALIC),
            sort: fg(t.accent).add_modifier(Modifier::BOLD),
            hint: fg(t.accent),
            thumb: fg(if focused { t.accent } else { t.muted }),
            special: fg(t.muted),
            error: fg(t.error),
            success: fg(t.success),
            gutter_current: fg(t.fg).add_modifier(Modifier::BOLD),
            current_row: t.highlight,
            zebra: blend(t.highlight, t.bg, 0.4),
            cursor: if focused { t.selection } else { blend(t.selection, t.bg, 0.5).unwrap_or(t.highlight) },
            selection: t.selection,
            search: t.warning,
            search_fg: t.bg,
            text: fg(t.fg),
            number: fg(t.number),
            boolean: fg(t.boolean),
            temporal: fg(t.temporal),
            json: fg(t.json),
            null: fg(t.null).add_modifier(Modifier::ITALIC),
        }
    }

    fn tone(&self, tone: Tone) -> Style {
        match tone {
            Tone::Text => self.text,
            Tone::Number => self.number,
            Tone::Bool => self.boolean,
            Tone::Temporal => self.temporal,
            Tone::Json => self.json,
            Tone::Null => self.null,
            Tone::Bytes => self.muted,
        }
    }
}

fn blend(a: Color, b: Color, t: f32) -> Option<Color> {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let mix = |x: u8, y: u8| (f32::from(x) * t + f32::from(y) * (1.0 - t)).round() as u8;
            Some(Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2)))
        }
        _ => None,
    }
}

fn decimal_digits(mut n: u64) -> u16 {
    let mut d = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

/// Leaves two columns for the ` ▲` sort indicator so short headers never truncate when sorted.
fn header_width(name: &str) -> u16 {
    (text_width(name, AUTO_MAX_WIDTH as usize) as u16 + 2).clamp(MIN_WIDTH, AUTO_MAX_WIDTH)
}

/// Grid display text. Bytes are always a truncated hex preview so huge blobs stay cheap.
fn cell_text<'a>(v: &'a Value, null_text: &'a str) -> Cow<'a, str> {
    match v {
        Value::Null => Cow::Borrowed(null_text),
        Value::Bytes(b) => {
            let mut s = String::with_capacity(2 + BYTES_PREVIEW * 2 + 3);
            s.push_str("0x");
            for byte in b.iter().take(BYTES_PREVIEW) {
                let _ = write!(s, "{byte:02x}");
            }
            if b.len() > BYTES_PREVIEW {
                s.push('…');
            }
            Cow::Owned(s)
        }
        v => v.display(),
    }
}

/// (symbol, width, is_whitespace_marker) for one grapheme as the grid draws it.
fn glyph(g: &str) -> (&str, usize, bool) {
    match g {
        "\n" | "\r\n" | "\r" => ("↵", 1, true),
        "\t" => ("→", 1, true),
        _ if g.chars().any(char::is_control) => ("", 0, false),
        _ => (g, g.width(), false),
    }
}

/// Display width as drawn by the grid, saturating at `cap` (so huge values cost O(cap)).
fn text_width(s: &str, cap: usize) -> usize {
    let head = &s.as_bytes()[..s.len().min(cap)];
    if head.iter().all(|b| (0x20..0x7f).contains(b)) {
        return head.len();
    }
    let mut w = 0;
    for g in s.graphemes(true) {
        w += glyph(g).1;
        if w >= cap {
            return cap;
        }
    }
    w
}

/// Writes `text` into the first line of `at`, truncating with `…`. Right alignment only applies when it fits.
fn draw_text(buf: &mut Buffer, at: Rect, text: &str, style: Style, special: Style, right: bool) {
    let (x, y) = (at.x, at.y);
    let limit = at.width as usize;
    if limit == 0 {
        return;
    }
    let mut total = 0;
    let mut truncated = false;
    for g in text.graphemes(true) {
        let w = glyph(g).1;
        if total + w > limit {
            truncated = true;
            break;
        }
        total += w;
    }
    let avail = if truncated { limit - 1 } else { limit };
    let mut cx = x + if right && !truncated { (limit - total) as u16 } else { 0 };
    let mut used = 0;
    for g in text.graphemes(true) {
        let (sym, w, is_special) = glyph(g);
        if w == 0 {
            continue;
        }
        if used + w > avail {
            break;
        }
        buf[(cx, y)].set_symbol(sym).set_style(if is_special { special } else { style });
        for i in 1..w as u16 {
            // Hidden behind the wide glyph; the terminal diff skips it.
            buf[(cx + i, y)].set_symbol(" ");
        }
        cx += w as u16;
        used += w;
    }
    if truncated {
        buf[(cx, y)].set_symbol("…").set_style(style);
    }
}

fn draw_centered(buf: &mut Buffer, area: Rect, text: &str, style: Style) {
    if area.is_empty() {
        return;
    }
    let w = (text_width(text, area.width as usize) as u16).min(area.width);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + area.height / 2;
    draw_text(buf, Rect::new(x, y, w, 1), text, style, style, false);
}

fn contains_ci(hay: &str, needle_lower: &str) -> bool {
    if needle_lower.is_ascii() {
        let n = needle_lower.as_bytes();
        hay.as_bytes().windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
    } else {
        hay.to_lowercase().contains(needle_lower)
    }
}

fn value_matches(v: &Value, needle_lower: &str) -> bool {
    !v.is_null() && contains_ci(&v.display(), needle_lower)
}

fn tsv_field(s: &str) -> Cow<'_, str> {
    if s.contains(['\t', '\n', '\r', '"']) {
        Cow::Owned(format!("\"{}\"", s.replace('"', "\"\"")))
    } else {
        Cow::Borrowed(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str) -> Column {
        Column::new(name, ty)
    }

    fn text(s: &str) -> Value {
        Value::Text(s.into())
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn press(g: &mut GridState, codes: &[KeyCode]) {
        for &c in codes {
            g.handle_key(key(c));
        }
    }

    fn draw(g: &mut GridState, w: u16, h: u16) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        g.render(Rect::new(0, 0, w, h), &mut buf, &Theme::default(), true);
        buf
    }

    fn line(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    fn find_all(buf: &Buffer, y: u16, needle: &str) -> Vec<u16> {
        let n = needle.chars().count() as u16;
        (0..buf.area.width.saturating_sub(n - 1))
            .filter(|&x| (x..x + n).map(|x| buf[(x, y)].symbol()).collect::<String>() == needle)
            .collect()
    }

    fn find(buf: &Buffer, y: u16, needle: &str) -> Option<u16> {
        find_all(buf, y, needle).first().copied()
    }

    fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
        MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }
    }

    fn people() -> GridState {
        let mut g = GridState::new();
        g.set_data(
            vec![col("id", "int4"), col("name", "text"), col("active", "bool"), col("meta", "jsonb")],
            vec![
                vec![Value::Int(1), text("O'Brien"), Value::Bool(true), text(r#"{"a": 1}"#)],
                vec![Value::Int(2), text("tab\there"), Value::Bool(false), Value::Null],
                vec![Value::Int(3), text("back\\slash \"q\""), Value::Null, text("[1,2]")],
            ],
        );
        g
    }

    // Digits of different magnitudes must share a right edge so place values line up.
    #[test]
    fn numbers_are_right_aligned_text_left_aligned() {
        let mut g = GridState::new();
        g.set_data(
            vec![col("amount", "numeric"), col("label", "text")],
            vec![vec![text("1"), text("ab")], vec![text("12345.5"), text("abcdef")]],
        );
        let buf = draw(&mut g, 60, 6);
        let theme = Theme::default();
        let one = *find_all(&buf, 2, "1").last().unwrap();
        let big_end = find(&buf, 3, "12345.5").unwrap() + 6;
        assert_eq!(one, big_end, "right edges differ:\n{}\n{}", line(&buf, 2), line(&buf, 3));
        assert_eq!(buf[(one, 2)].fg, theme.number);
        assert_eq!(find(&buf, 2, "ab"), find(&buf, 3, "abcdef"));
    }

    // NULL must be distinguishable from the string 'NULL'.
    #[test]
    fn null_is_italic_and_muted_unlike_the_string_null() {
        let mut g = GridState::new();
        g.set_data(vec![col("a", "text"), col("b", "text")], vec![vec![Value::Null, text("NULL")]]);
        let buf = draw(&mut g, 40, 4);
        let theme = Theme::default();
        let hits = find_all(&buf, 2, "NULL");
        let [null_x, str_x] = hits[..] else { panic!("{}", line(&buf, 2)) };
        assert!(buf[(null_x, 2)].modifier.contains(Modifier::ITALIC));
        assert_eq!(buf[(null_x, 2)].fg, theme.null);
        assert_eq!(buf[(str_x, 2)].symbol(), "N");
        assert!(!buf[(str_x, 2)].modifier.contains(Modifier::ITALIC));
        assert_eq!(buf[(str_x, 2)].fg, theme.fg);
    }

    #[test]
    fn custom_null_text_is_used() {
        let mut g = GridState::new();
        g.null_text = "∅".into();
        g.set_data(vec![col("a", "text")], vec![vec![Value::Null]]);
        let buf = draw(&mut g, 20, 3);
        assert!(line(&buf, 2).contains('∅'));
    }

    // Wide glyphs occupy two cells; separators must still form straight vertical lines.
    #[test]
    fn cjk_and_emoji_keep_column_separators_aligned() {
        let mut g = GridState::new();
        g.set_data(
            vec![col("name", "text"), col("n", "int")],
            vec![
                vec![text("日本語"), Value::Int(1)],
                vec![text("ab"), Value::Int(2)],
                vec![text("😀x"), Value::Int(3)],
            ],
        );
        assert_eq!(g.column_width(0), Some(6));
        let buf = draw(&mut g, 40, 6);
        let seps: Vec<Vec<u16>> =
            (0..5).filter(|&y| y != 1).map(|y| (0..40).filter(|&x| buf[(x, y)].symbol() == "│").collect()).collect();
        assert!(seps[0].len() >= 3, "{seps:?}");
        assert!(seps.iter().all(|s| *s == seps[0]), "{seps:?}");
        assert_eq!(find(&buf, 2, "日").map(|x| buf[(x + 2, 2)].symbol().to_string()), Some("本".into()));
    }

    #[test]
    fn overlong_values_truncate_with_ellipsis_on_wide_boundaries() {
        let mut g = GridState::new();
        g.set_data(vec![col("t", "text")], vec![vec![text("short")], vec![text(&"漢".repeat(40))]]);
        assert_eq!(g.column_width(0), Some(AUTO_MAX_WIDTH));
        let buf = draw(&mut g, 80, 5);
        let row = line(&buf, 3);
        assert!(row.contains('…'), "{row}");
        let seps: Vec<u16> = (0..80).filter(|&x| buf[(x, 3)].symbol() == "│").collect();
        let header_seps: Vec<u16> = (0..80).filter(|&x| buf[(x, 0)].symbol() == "│").collect();
        assert_eq!(seps, header_seps);
    }

    #[test]
    fn control_characters_render_as_muted_markers() {
        let mut g = GridState::new();
        g.set_data(vec![col("t", "text")], vec![vec![text("a\nb\tc")]]);
        let buf = draw(&mut g, 30, 3);
        let x = find(&buf, 2, "a↵b→c").unwrap();
        assert_eq!(buf[(x + 1, 2)].fg, Theme::default().muted);
        assert_eq!(buf[(x, 2)].fg, Theme::default().fg);
    }

    #[test]
    fn bytes_render_as_truncated_hex() {
        let mut g = GridState::new();
        g.set_data(vec![col("b", "bytea")], vec![vec![Value::Bytes(vec![0xab; 100])]]);
        let buf = draw(&mut g, 100, 3);
        let row = line(&buf, 2);
        assert!(row.contains("0xabab"), "{row}");
        assert!(row.contains('…'));
    }

    // The cursor column must never end up scrolled off-screen, in either direction.
    #[test]
    fn horizontal_scroll_keeps_cursor_column_visible() {
        let mut g = GridState::new();
        let cols: Vec<_> = (0..12).map(|i| col(&format!("column_{i:02}"), "text")).collect();
        let row: Row = (0..12).map(|i| text(&format!("value_{i:02}"))).collect();
        g.set_data(cols, vec![row; 5]);
        let buf = draw(&mut g, 50, 8);
        assert!(!line(&buf, 1).contains('◂'));
        assert!(line(&buf, 1).contains('▸'));

        press(&mut g, &[KeyCode::End]);
        let buf = draw(&mut g, 50, 8);
        assert!(line(&buf, 0).contains("column_11"), "{}", line(&buf, 0));
        assert!(line(&buf, 1).contains('◂'));
        assert!(!line(&buf, 0).contains("column_00"));

        press(&mut g, &[KeyCode::Char('h'); 6]);
        let buf = draw(&mut g, 50, 8);
        assert!(line(&buf, 0).contains("column_05"), "{}", line(&buf, 0));

        press(&mut g, &[KeyCode::Char('0')]);
        let buf = draw(&mut g, 50, 8);
        assert!(line(&buf, 0).contains("column_00"));
        assert!(!line(&buf, 1).contains('◂'));
    }

    #[test]
    fn vertical_scroll_follows_cursor_and_gutter_shows_absolute_numbers() {
        let mut g = GridState::new();
        g.row_offset_label = 1000;
        g.set_data(vec![col("n", "int")], (0..100).map(|i| vec![Value::Int(i)]).collect());
        let buf = draw(&mut g, 30, 12);
        assert!(line(&buf, 2).contains("1001"));
        assert_eq!(g.visible_row_range(), 0..10);

        press(&mut g, &[KeyCode::Char('G')]);
        let buf = draw(&mut g, 30, 12);
        assert_eq!(g.visible_row_range(), 90..100);
        assert!(line(&buf, 11).contains("1100"), "{}", line(&buf, 11));
        assert_eq!(buf[(10, 11)].bg, Theme::default().selection);

        press(&mut g, &[KeyCode::Char('g')]);
        draw(&mut g, 30, 12);
        press(&mut g, &[KeyCode::PageDown]);
        draw(&mut g, 30, 12);
        assert_eq!(g.selected_cell(), Some((10, 0)));
        assert_eq!(g.visible_row_range(), 1..11);

        g.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(g.selected_cell(), Some((15, 0)));
    }

    #[test]
    fn wheel_scrolls_without_moving_cursor() {
        let mut g = GridState::new();
        g.set_data(vec![col("n", "int")], (0..100).map(|i| vec![Value::Int(i)]).collect());
        let area = Rect::new(0, 0, 30, 12);
        draw(&mut g, 30, 12);
        g.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 5), area);
        draw(&mut g, 30, 12);
        assert_eq!(g.visible_row_range(), 3..13);
        assert_eq!(g.selected_cell(), Some((0, 0)));
    }

    // Copies must round-trip through the target format: quotes/tabs/newlines escaped, NULL kept distinct.
    #[test]
    fn shift_selection_copies_as_tsv() {
        let mut g = people();
        g.handle_key(shift(KeyCode::Right));
        g.handle_key(shift(KeyCode::Down));
        assert_eq!(g.selection_range(), Some((0..2, 0..2)));
        assert_eq!(g.copy_cells_tsv(true), "id\tname\n1\tO'Brien\n2\t\"tab\there\"");

        press(&mut g, &[KeyCode::Char('j')]);
        assert_eq!(g.selection_range(), Some((2..3, 1..2)), "plain move drops a shift selection");
        assert_eq!(g.copy_cells_tsv(false), "back\\slash \"q\"");
    }

    #[test]
    fn visual_row_mode_selects_whole_rows() {
        let mut g = people();
        press(&mut g, &[KeyCode::Char('l'), KeyCode::Char('V'), KeyCode::Char('j')]);
        assert_eq!(g.selection_range(), Some((0..2, 0..4)));
        assert_eq!(g.copy_rows_tsv(false).lines().count(), 2);
        assert_eq!(g.handle_key(key(KeyCode::Char('D'))), GridEvent::DeleteRows(0..2));
        assert_eq!(g.handle_key(key(KeyCode::Esc)), GridEvent::Handled);
        assert_eq!(g.handle_key(key(KeyCode::Esc)), GridEvent::Unhandled);
    }

    #[test]
    fn search_is_case_insensitive_and_wraps() {
        let mut g = GridState::new();
        let rows = ["alpha", "Foo bar", "gamma", "delta", "xFOOx", "omega"].iter().map(|s| vec![text(s)]).collect();
        g.set_data(vec![col("s", "text")], rows);
        g.set_search(Some("foo".into()));
        press(&mut g, &[KeyCode::Char('n')]);
        assert_eq!(g.selected_cell(), Some((1, 0)));
        press(&mut g, &[KeyCode::Char('n')]);
        assert_eq!(g.selected_cell(), Some((4, 0)));
        press(&mut g, &[KeyCode::Char('n')]);
        assert_eq!(g.selected_cell(), Some((1, 0)), "wraps to the top");
        press(&mut g, &[KeyCode::Char('N')]);
        assert_eq!(g.selected_cell(), Some((4, 0)), "wraps backwards");

        let buf = draw(&mut g, 30, 10);
        assert_eq!(buf[(5, 3)].bg, Theme::default().warning, "non-cursor match highlighted");
        assert_ne!(buf[(5, 4)].bg, Theme::default().warning);

        g.set_search(Some("zzz".into()));
        assert_eq!(g.handle_key(key(KeyCode::Char('n'))), GridEvent::NoMatch);
        g.set_search(None);
        assert_eq!(g.handle_key(key(KeyCode::Char('n'))), GridEvent::Unhandled);
    }

    // Streamed batches arrive after the first render; late long values must widen columns, but never past the cap.
    #[test]
    fn push_rows_grows_widths_within_cap_and_respects_manual_resize() {
        let mut g = GridState::new();
        g.set_data(vec![col("a", "text"), col("b", "text")], vec![vec![text("x"), text("y")]]);
        assert_eq!(g.column_width(0), Some(MIN_WIDTH));
        g.push_rows(vec![vec![text("0123456789"), text("y")]]);
        assert_eq!(g.column_width(0), Some(10));
        g.push_rows(vec![vec![text(&"z".repeat(500)), text("y")]]);
        assert_eq!(g.column_width(0), Some(AUTO_MAX_WIDTH));
        assert_eq!(g.row_count(), 3);

        press(&mut g, &[KeyCode::Char('l'), KeyCode::Char('>'), KeyCode::Char('>')]);
        assert_eq!(g.column_width(1), Some(MIN_WIDTH + 2 * RESIZE_STEP));
        g.push_rows(vec![vec![text("x"), text("a much longer value")]]);
        assert_eq!(g.column_width(1), Some(MIN_WIDTH + 2 * RESIZE_STEP), "user width wins");

        press(&mut g, &[KeyCode::Char('h'), KeyCode::Char('=')]);
        assert_eq!(g.column_width(0), Some(FIT_MAX_WIDTH), "auto-fit sees full content up to 120");
    }

    #[test]
    fn same_columns_keep_view_state_on_refresh() {
        let mut g = people();
        press(&mut g, &[KeyCode::Char('j'), KeyCode::Char('l'), KeyCode::Char('>')]);
        let w = g.column_width(1);
        let (cols, rows) = (g.columns().to_vec(), g.rows().to_vec());
        g.set_data(cols, rows);
        assert_eq!(g.selected_cell(), Some((1, 1)));
        assert_eq!(g.column_width(1), w);
        g.set_data(vec![col("other", "text")], vec![vec![text("x")]]);
        assert_eq!(g.selected_cell(), Some((0, 0)));
    }

    #[test]
    fn mouse_header_click_requests_sort_and_double_click_opens() {
        let mut g = people();
        let area = Rect::new(0, 0, 80, 8);
        let buf = draw(&mut g, 80, 8);
        let name_x = find(&buf, 0, "name").unwrap();
        assert_eq!(
            g.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), name_x, 0), area),
            GridEvent::SortRequested(1)
        );

        g.set_sort(1, false);
        let buf = draw(&mut g, 80, 8);
        assert!(line(&buf, 0).contains('▼'));

        let click = mouse(MouseEventKind::Down(MouseButton::Left), name_x, 4);
        assert_eq!(g.handle_mouse(click, area), GridEvent::Handled);
        assert_eq!(g.selected_cell(), Some((2, 1)));
        assert_eq!(g.handle_mouse(click, area), GridEvent::OpenCell(2, 1));
    }

    #[test]
    fn mouse_drag_selects_rectangle() {
        let mut g = people();
        let area = Rect::new(0, 0, 80, 8);
        let buf = draw(&mut g, 80, 8);
        let id_x = find(&buf, 0, "id").unwrap();
        let active_x = find(&buf, 0, "active").unwrap();
        g.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), id_x, 2), area);
        g.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), active_x, 3), area);
        g.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), active_x, 3), area);
        assert_eq!(g.selection_range(), Some((0..2, 0..3)));
    }

    #[test]
    fn edit_markers_render() {
        let mut g = people();
        g.mark_edited(0, 1);
        g.mark_deleted_row(1);
        g.mark_new_row(2);
        press(&mut g, &[KeyCode::Char('l'), KeyCode::Char('l')]);
        let buf = draw(&mut g, 80, 8);
        let theme = Theme::default();
        let edited = find(&buf, 2, "O'Brien").unwrap();
        assert_eq!(buf[(edited, 2)].fg, theme.warning);
        assert!(buf[(edited, 2)].modifier.contains(Modifier::UNDERLINED));
        let deleted = find(&buf, 3, "tab").unwrap();
        assert_eq!(buf[(deleted, 3)].fg, theme.error);
        assert!(buf[(deleted, 3)].modifier.contains(Modifier::CROSSED_OUT));
        assert_eq!(buf[(0, 4)].symbol(), "+");
        assert_eq!(buf[(0, 4)].fg, theme.success);
        g.clear_marks();
        let buf = draw(&mut g, 80, 8);
        assert_eq!(buf[(0, 4)].symbol(), " ");
    }

    #[test]
    fn keys_emit_app_events() {
        let mut g = people();
        assert_eq!(g.handle_key(key(KeyCode::Char('/'))), GridEvent::StartSearch);
        assert_eq!(g.handle_key(key(KeyCode::Char('y'))), GridEvent::Copy(CopyKind::Cells));
        assert_eq!(g.handle_key(key(KeyCode::Char('Y'))), GridEvent::Copy(CopyKind::Rows));
        g.handle_key(key(KeyCode::Tab));
        assert_eq!(g.handle_key(key(KeyCode::Enter)), GridEvent::OpenCell(0, 1));
        assert_eq!(g.handle_key(key(KeyCode::Char('s'))), GridEvent::SortRequested(1));
        assert_eq!(g.handle_key(key(KeyCode::F(2))), GridEvent::EditCell(0, 1));
        assert_eq!(g.handle_key(key(KeyCode::Char('f'))), GridEvent::FilterRequested(1));
        assert_eq!(g.handle_key(key(KeyCode::Char('o'))), GridEvent::InsertRow);
        assert_eq!(g.handle_key(key(KeyCode::Delete)), GridEvent::DeleteRows(0..1));
        assert_eq!(g.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), GridEvent::Unhandled);
        assert_eq!(g.handle_key(key(KeyCode::Char('q'))), GridEvent::Unhandled);
        press(&mut g, &[KeyCode::Char('$'), KeyCode::Tab]);
        assert_eq!(g.selected_cell(), Some((1, 0)), "Tab wraps to the next row");
        g.handle_key(key(KeyCode::BackTab));
        assert_eq!(g.selected_cell(), Some((0, 3)));
    }

    #[test]
    fn empty_states_show_message() {
        let mut g = GridState::new();
        let buf = draw(&mut g, 30, 5);
        assert!(line(&buf, 2).contains("No rows"));
        assert_eq!(g.handle_key(key(KeyCode::Char('j'))), GridEvent::Unhandled);

        g.set_empty_message("Empty table");
        g.set_data(vec![col("a", "text")], vec![]);
        let buf = draw(&mut g, 30, 7);
        assert!(line(&buf, 0).contains('a'));
        assert!((2..7).any(|y| line(&buf, y).contains("Empty table")));
        assert_eq!(g.selected_value(), None);
        assert_eq!(g.handle_key(key(KeyCode::Enter)), GridEvent::Unhandled);
    }

    #[test]
    fn show_types_adds_type_line() {
        let mut g = GridState::new();
        g.show_types = true;
        let p = people();
        g.set_data(p.columns().to_vec(), p.rows().to_vec());
        let buf = draw(&mut g, 80, 8);
        assert!(line(&buf, 1).contains("int4"));
        assert!(line(&buf, 2).contains('─'));
        assert!(line(&buf, 3).contains("O'Brien"));
    }

    #[test]
    fn large_result_renders_one_frame_fast() {
        let mut g = GridState::new();
        let cols: Vec<_> = (0..8).map(|i| col(&format!("c{i}"), if i % 2 == 0 { "int" } else { "text" })).collect();
        let rows: Vec<Row> = (0..200_000i64)
            .map(|r| (0..8).map(|c| if c % 2 == 0 { Value::Int(r * c) } else { text("some text value") }).collect())
            .collect();
        g.set_data(cols, rows);
        g.set_search(Some("text".into()));
        press(&mut g, &[KeyCode::Char('G')]);
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 50));
        let theme = Theme::default();
        let start = Instant::now();
        g.render(Rect::new(0, 0, 160, 50), &mut buf, &theme, true);
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50), "render took {elapsed:?}");
        assert!(line(&buf, 49).contains("200000"));
    }

    #[test]
    fn widths_keep_growing_after_the_sample_budget() {
        let mut g = GridState::new();
        g.set_data(vec![col("n", "int8")], Vec::new());
        for batch in 0..200 {
            let rows = (0..256).map(|i| vec![Value::Int(batch * 256 + i)]).collect();
            g.push_rows(rows);
        }
        assert!(g.column_width(0).unwrap() >= 5, "51199 must fit without truncation");
    }
}
