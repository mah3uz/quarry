//! Vim-style modal editing for [`Editor`]: normal, insert, visual and visual-line modes with the
//! common motions, operators (`d`, `c`, `y`, `>`, `<`) and counts. Insert mode is the ordinary editor.

use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VimMode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
}

impl VimMode {
    pub fn label(self) -> &'static str {
        match self {
            VimMode::Normal => "NORMAL",
            VimMode::Insert => "INSERT",
            VimMode::Visual => "VISUAL",
            VimMode::VisualLine => "V-LINE",
        }
    }
}

#[derive(Debug, Default)]
pub struct Vim {
    pub mode: VimMode,
    count: Option<usize>,
    /// Operator waiting for a motion: d, c, y, >, <.
    op: Option<char>,
    /// First key of a two-key command: g (gg), r (replace), i/a (text object after an operator).
    prefix: Option<char>,
    /// The row visual-line mode started on.
    line_anchor: usize,
}

impl Vim {
    fn reset(&mut self) {
        self.count = None;
        self.op = None;
        self.prefix = None;
    }

    fn pending(&self) -> bool {
        self.count.is_some() || self.op.is_some() || self.prefix.is_some()
    }
}

/// Where a motion lands and how an operator treats the span up to it.
#[derive(Clone, Copy)]
struct Target {
    pos: Pos,
    linewise: bool,
    /// The character at `pos` belongs to the span (e, $).
    inclusive: bool,
}

impl Editor {
    pub fn set_vim(&mut self, on: bool) {
        self.vim = on.then(Vim::default);
    }

    pub fn vim_mode(&self) -> Option<VimMode> {
        self.vim.as_ref().map(|v| v.mode)
    }

    /// Runs `key` through vim; `None` means insert mode, where the ordinary editor handles it.
    pub(super) fn vim_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
        let mut v = self.vim.take()?;
        let ev = if v.mode == VimMode::Insert {
            if key.code == KeyCode::Esc {
                v.mode = VimMode::Normal;
                self.cursor = self.left_in_line(self.cursor);
                self.anchor = None;
                self.clamp_normal();
                Some(EditorEvent::Moved)
            } else {
                None
            }
        } else {
            Some(self.dispatch(&mut v, key))
        };
        self.vim = Some(v);
        ev
    }

    /// The normal-mode cursor sits on a character, never after the last one.
    fn clamp_normal(&mut self) {
        let len = self.lines[self.cursor.row].len();
        if len > 0 && self.cursor.col >= len {
            self.cursor = self.left_of(Pos { row: self.cursor.row, col: len });
        }
        self.follow_cursor = true;
    }

    fn left_in_line(&self, p: Pos) -> Pos {
        if p.col == 0 { p } else { self.left_of(p) }
    }

    /// One character right on the same line; `allow_end` permits the position after the last character.
    fn right_in_line(&self, p: Pos, allow_end: bool) -> Pos {
        let len = self.lines[p.row].len();
        if p.col >= len {
            return p;
        }
        let next = self.right_of(p);
        if !allow_end && next.col >= len { p } else { next }
    }

    fn first_non_blank(&self, row: usize) -> Pos {
        let line = &self.lines[row];
        Pos { row, col: line.len() - line.trim_start().len() }
    }

    fn word_end(&self, p: Pos) -> Pos {
        let mut q = self.right_of(p);
        while self.char_at(q).is_none_or(char::is_whitespace) {
            let n = self.right_of(q);
            if n == q {
                return q;
            }
            q = n;
        }
        let class = char_class(self.char_at(q).unwrap_or(' '));
        loop {
            let n = self.right_of(q);
            if n.row != q.row || self.char_at(n).is_none_or(|c| char_class(c) != class) {
                return q;
            }
            q = n;
        }
    }

    fn vertical_target(&self, delta: isize) -> Pos {
        let Pos { row, col } = self.cursor;
        let want = self.want_col.unwrap_or_else(|| display_col(&self.lines[row], col));
        let target = row.saturating_add_signed(delta).min(self.lines.len() - 1);
        Pos { row: target, col: col_at_display(&self.lines[target], want) }
    }

    /// Rows `first..=last` with the newline that separates them from the rest, so deleting the span
    /// removes the lines entirely.
    fn line_span(&self, first: usize, last: usize) -> (Pos, Pos) {
        if last + 1 < self.lines.len() {
            (Pos { row: first, col: 0 }, Pos { row: last + 1, col: 0 })
        } else if first > 0 {
            (Pos { row: first - 1, col: self.lines[first - 1].len() }, Pos { row: last, col: self.lines[last].len() })
        } else {
            (Pos { row: 0, col: 0 }, Pos { row: last, col: self.lines[last].len() })
        }
    }

    /// `count` is the typed count, if any (G and gg use it as a line number).
    fn motion(&self, c: char, count: Option<usize>) -> Option<Target> {
        let n = count.unwrap_or(1).max(1);
        let chars = |pos| Some(Target { pos, linewise: false, inclusive: false });
        let lines = |pos| Some(Target { pos, linewise: true, inclusive: false });
        let mut p = self.cursor;
        let last_row = self.lines.len() - 1;
        match c {
            'h' => (0..n).for_each(|_| p = self.left_in_line(p)),
            'l' => (0..n).for_each(|_| p = self.right_in_line(p, false)),
            'j' => return lines(self.vertical_target(n as isize)),
            'k' => return lines(self.vertical_target(-(n as isize))),
            'w' => (0..n).for_each(|_| p = self.word_right(p)),
            'b' => (0..n).for_each(|_| p = self.word_left(p)),
            'e' => {
                (0..n).for_each(|_| p = self.word_end(p));
                return Some(Target { pos: p, linewise: false, inclusive: true });
            }
            '0' => p.col = 0,
            '^' => p = self.first_non_blank(p.row),
            '$' => {
                let row = (p.row + n - 1).min(last_row);
                let len = self.lines[row].len();
                return Some(Target { pos: self.left_in_line(Pos { row, col: len }), linewise: false, inclusive: len > 0 });
            }
            'G' => return lines(self.first_non_blank(count.map_or(last_row, |n| n.saturating_sub(1).min(last_row)))),
            _ => return None,
        }
        chars(p)
    }

    /// Start and end (exclusive) of what an operator covers from the cursor to `t`.
    fn span(&self, t: Target) -> (Pos, Pos) {
        let (a, b) = (self.cursor.min(t.pos), self.cursor.max(t.pos));
        if t.linewise {
            return self.line_span(a.row, b.row);
        }
        (a, if t.inclusive { self.right_in_line(b, true) } else { b })
    }

    fn operate(&mut self, v: &mut Vim, op: char, start: Pos, end: Pos, linewise: bool) -> EditorEvent {
        v.reset();
        let first_row = start.row + usize::from(linewise && start.col > 0);
        let mut text = self.text_between(start, end);
        if linewise {
            text = text.trim_start_matches('\n').to_string();
            if !text.ends_with('\n') {
                text.push('\n');
            }
        }
        match op {
            'y' => {
                self.copy(text);
                self.cursor = if linewise { Pos { row: first_row, col: self.cursor.col } } else { start };
                self.clamp_normal();
                EditorEvent::Moved
            }
            '>' | '<' => {
                let last_row = if end.col == 0 && end.row > first_row { end.row - 1 } else { end.row };
                self.anchor = Some(Pos { row: first_row, col: 0 });
                self.cursor = Pos { row: last_row, col: self.lines[last_row].len() };
                let ev = self.shift_lines(op == '>');
                self.anchor = None;
                self.cursor = self.first_non_blank(first_row);
                ev
            }
            'c' if linewise => {
                self.copy(text);
                let last_row = if end.col == 0 && end.row > first_row { end.row - 1 } else { end.row };
                let indent: String = self.lines[first_row].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                let mut tx = self.begin();
                let line_end = Pos { row: last_row, col: self.lines[last_row].len() };
                self.cursor = self.tx_replace(&mut tx, Pos { row: first_row, col: 0 }, line_end, &indent);
                self.commit(tx, StepKind::Other);
                v.mode = VimMode::Insert;
                EditorEvent::Changed
            }
            _ => {
                self.copy(text);
                let mut tx = self.begin();
                self.cursor = self.tx_replace(&mut tx, start, end, "");
                self.anchor = None;
                self.commit(tx, StepKind::Other);
                if op == 'c' {
                    v.mode = VimMode::Insert;
                } else {
                    if linewise {
                        self.cursor = self.first_non_blank(first_row.min(self.lines.len() - 1));
                    }
                    self.clamp_normal();
                }
                EditorEvent::Changed
            }
        }
    }

    /// Text ending in a newline was yanked by lines and goes in as whole lines.
    fn paste(&mut self, before: bool, count: usize) -> EditorEvent {
        let text = normalize_newlines(&self.paste_source());
        if text.is_empty() {
            return EditorEvent::Moved;
        }
        let mut tx = self.begin();
        let row = self.cursor.row;
        if let Some(body) = text.strip_suffix('\n') {
            let block = vec![body; count].join("\n");
            if before {
                let at = Pos { row, col: 0 };
                self.tx_replace(&mut tx, at, at, &format!("{block}\n"));
                self.cursor = self.first_non_blank(row);
            } else {
                let at = Pos { row, col: self.lines[row].len() };
                self.tx_replace(&mut tx, at, at, &format!("\n{block}"));
                self.cursor = self.first_non_blank(row + 1);
            }
        } else {
            let at = if before { self.cursor } else { self.right_in_line(self.cursor, true) };
            let after = self.tx_replace(&mut tx, at, at, &text.repeat(count));
            self.cursor = self.left_in_line(after);
        }
        self.commit(tx, StepKind::Other);
        self.clamp_normal();
        EditorEvent::Changed
    }

    /// Visual-line keeps whole lines selected whichever way the cursor moves.
    fn cover_lines(&mut self, anchor_row: usize) {
        let row = self.cursor.row;
        if row >= anchor_row {
            self.anchor = Some(Pos { row: anchor_row, col: 0 });
            self.cursor = Pos { row, col: self.lines[row].len() };
        } else {
            self.anchor = Some(Pos { row: anchor_row, col: self.lines[anchor_row].len() });
            self.cursor = Pos { row, col: 0 };
        }
    }

    fn enter_insert(&mut self, v: &mut Vim) {
        v.mode = VimMode::Insert;
        v.reset();
        self.anchor = None;
        self.follow_cursor = true;
    }

    fn move_by_motion(&mut self, t: Target) -> EditorEvent {
        let vertical = t.linewise && t.pos.row != self.cursor.row;
        let want = self.want_col.or_else(|| Some(display_col(&self.lines[self.cursor.row], self.cursor.col)));
        self.move_to(t.pos, false);
        self.want_col = if vertical { want } else { None };
        self.clamp_normal();
        EditorEvent::Moved
    }

    fn dispatch(&mut self, v: &mut Vim, key: KeyEvent) -> EditorEvent {
        use EditorEvent::*;
        let edit = !self.read_only;
        let visual = matches!(v.mode, VimMode::Visual | VimMode::VisualLine);
        if key.code == KeyCode::Esc {
            if !v.pending() && !visual {
                return Unhandled;
            }
            v.reset();
            v.mode = VimMode::Normal;
            self.anchor = None;
            self.clamp_normal();
            return Moved;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Char('r') if edit => {
                    let ev = changed_if(self.redo());
                    self.clamp_normal();
                    ev
                }
                KeyCode::Char('d') | KeyCode::Char('u') => {
                    let half = (self.view_height / 2).max(1) as isize;
                    let p = self.vertical_target(if key.code == KeyCode::Char('d') { half } else { -half });
                    self.move_to(p, visual);
                    Moved
                }
                _ => Unhandled,
            };
        }
        let c = match key.code {
            KeyCode::Char(c) => c,
            KeyCode::Left | KeyCode::Backspace => 'h',
            KeyCode::Right => 'l',
            KeyCode::Down => 'j',
            KeyCode::Up => 'k',
            KeyCode::Home => '0',
            KeyCode::End => '$',
            _ => return Unhandled,
        };

        if let Some(pre) = v.prefix.take() {
            return self.second_key(v, pre, c, edit);
        }
        if c.is_ascii_digit() && (c != '0' || v.count.is_some()) {
            v.count = Some(v.count.unwrap_or(0) * 10 + c.to_digit(10).unwrap_or(0) as usize);
            return Moved;
        }
        if visual {
            return self.visual_key(v, c, edit);
        }

        let n = v.count.unwrap_or(1).max(1);
        if let Some(op) = v.op {
            if c == op {
                let last = (self.cursor.row + n - 1).min(self.lines.len() - 1);
                let (s, e) = self.line_span(self.cursor.row, last);
                return self.operate(v, op, s, e, true);
            }
            if matches!(c, 'i' | 'a' | 'g') {
                v.prefix = Some(c);
                return Moved;
            }
        }
        // as in vim, cw on a word changes to its end and keeps the blank after it
        let c = if v.op == Some('c') && c == 'w' && self.char_at(self.cursor).is_some_and(|ch| !ch.is_whitespace()) { 'e' } else { c };
        if let Some(t) = self.motion(c, v.count) {
            return match v.op {
                Some(op) => {
                    let (s, e) = self.span(t);
                    self.operate(v, op, s, e, t.linewise)
                }
                None => {
                    v.reset();
                    self.move_by_motion(t)
                }
            };
        }
        if v.op.is_some() {
            v.reset();
            return Moved;
        }
        v.count = None;
        match c {
            'g' | 'r' => {
                v.count = (n > 1).then_some(n);
                v.prefix = Some(c);
                Moved
            }
            'y' | 'd' | 'c' | '>' | '<' if edit || c == 'y' => {
                v.op = Some(c);
                v.count = (n > 1).then_some(n);
                Moved
            }
            'i' if edit => {
                self.enter_insert(v);
                Moved
            }
            'a' if edit => {
                self.cursor = self.right_in_line(self.cursor, true);
                self.enter_insert(v);
                Moved
            }
            'I' if edit => {
                self.cursor = self.first_non_blank(self.cursor.row);
                self.enter_insert(v);
                Moved
            }
            'A' if edit => {
                self.cursor.col = self.lines[self.cursor.row].len();
                self.enter_insert(v);
                Moved
            }
            'o' | 'O' if edit => {
                let row = self.cursor.row;
                let indent: String = self.lines[row].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                let mut tx = self.begin();
                if c == 'o' {
                    let end = Pos { row, col: self.lines[row].len() };
                    self.cursor = self.tx_replace(&mut tx, end, end, &format!("\n{indent}"));
                } else {
                    let at = Pos { row, col: 0 };
                    self.tx_replace(&mut tx, at, at, &format!("{indent}\n"));
                    self.cursor = Pos { row, col: indent.len() };
                }
                self.commit(tx, StepKind::Other);
                self.enter_insert(v);
                Changed
            }
            'x' | 's' if edit => {
                let end = (0..n).fold(self.cursor, |p, _| self.right_in_line(p, true));
                if end == self.cursor && c == 'x' {
                    return Moved;
                }
                self.operate(v, if c == 's' { 'c' } else { 'd' }, self.cursor, end, false)
            }
            'X' if edit => {
                let start = (0..n).fold(self.cursor, |p, _| self.left_in_line(p));
                if start == self.cursor {
                    return Moved;
                }
                self.operate(v, 'd', start, self.cursor, false)
            }
            'D' | 'C' if edit => {
                let end = Pos { row: self.cursor.row, col: self.lines[self.cursor.row].len() };
                self.operate(v, if c == 'D' { 'd' } else { 'c' }, self.cursor, end, false)
            }
            'S' if edit => {
                let (s, e) = self.line_span(self.cursor.row, self.cursor.row);
                self.operate(v, 'c', s, e, true)
            }
            'Y' => {
                let last = (self.cursor.row + n - 1).min(self.lines.len() - 1);
                let (s, e) = self.line_span(self.cursor.row, last);
                self.operate(v, 'y', s, e, true)
            }
            'p' | 'P' if edit => self.paste(c == 'P', n),
            'u' if edit => {
                let changed = (0..n).fold(false, |ch, _| self.undo() || ch);
                self.clamp_normal();
                changed_if(changed)
            }
            'J' if edit => self.join_lines(n),
            'v' => {
                v.mode = VimMode::Visual;
                self.anchor = Some(self.cursor);
                Moved
            }
            'V' => {
                v.mode = VimMode::VisualLine;
                v.line_anchor = self.cursor.row;
                self.cover_lines(v.line_anchor);
                Moved
            }
            _ => Moved,
        }
    }

    fn second_key(&mut self, v: &mut Vim, pre: char, c: char, edit: bool) -> EditorEvent {
        use EditorEvent::*;
        match pre {
            'r' if edit => {
                let n = v.count.take().unwrap_or(1);
                let end = (0..n).fold(self.cursor, |p, _| self.right_in_line(p, true));
                v.reset();
                if end == self.cursor {
                    return Moved;
                }
                let at = self.cursor;
                let chars = self.text_between(at, end).chars().count();
                let mut tx = self.begin();
                self.tx_replace(&mut tx, at, end, &c.to_string().repeat(chars));
                self.cursor = at;
                self.commit(tx, StepKind::Other);
                Changed
            }
            'g' if c == 'g' => {
                let row = v.count.take().map_or(0, |n| n.saturating_sub(1)).min(self.lines.len() - 1);
                let t = Target { pos: self.first_non_blank(row), linewise: true, inclusive: false };
                match v.op {
                    Some(op) => {
                        let (s, e) = self.span(t);
                        self.operate(v, op, s, e, true)
                    }
                    None => {
                        v.reset();
                        if matches!(v.mode, VimMode::Visual | VimMode::VisualLine) {
                            self.visual_move(v, t.pos);
                            return Moved;
                        }
                        self.move_by_motion(t)
                    }
                }
            }
            'i' | 'a' if c == 'w' && v.op.is_some() => {
                let op = v.op.unwrap_or('d');
                let (start, mut end) = self.word_object(self.cursor);
                if pre == 'a' {
                    while self.char_at(end).is_some_and(|ch| ch == ' ' || ch == '\t') {
                        end = self.right_in_line(end, true);
                    }
                }
                self.operate(v, op, start, end, false)
            }
            _ => {
                v.reset();
                Moved
            }
        }
    }

    /// The word (or run of punctuation or blanks) under `p`, end exclusive.
    fn word_object(&self, p: Pos) -> (Pos, Pos) {
        let line = &self.lines[p.row];
        let Some(c) = self.char_at(p) else { return (p, p) };
        let class = char_class(c);
        let mut start = p.col;
        for (i, ch) in line[..p.col].char_indices().rev() {
            if char_class(ch) != class {
                break;
            }
            start = i;
        }
        let end = line[p.col..].char_indices().find(|(_, ch)| char_class(*ch) != class).map_or(line.len(), |(i, _)| p.col + i);
        (Pos { row: p.row, col: start }, Pos { row: p.row, col: end })
    }

    fn join_lines(&mut self, n: usize) -> EditorEvent {
        let mut tx = self.begin();
        for _ in 0..n {
            let row = self.cursor.row;
            if row + 1 >= self.lines.len() {
                break;
            }
            let next = &self.lines[row + 1];
            let lead = next.len() - next.trim_start().len();
            let sep = if self.lines[row].ends_with(' ') || next.trim().is_empty() { "" } else { " " };
            let join = Pos { row, col: self.lines[row].len() };
            self.tx_replace(&mut tx, join, Pos { row: row + 1, col: lead }, sep);
            self.cursor = join;
        }
        self.commit(tx, StepKind::Other);
        self.clamp_normal();
        EditorEvent::Changed
    }

    fn visual_move(&mut self, v: &Vim, to: Pos) {
        let anchor = self.anchor.unwrap_or(self.cursor);
        self.cursor = to;
        self.follow_cursor = true;
        if v.mode == VimMode::VisualLine {
            self.cover_lines(v.line_anchor);
        } else {
            self.anchor = Some(anchor);
        }
    }

    fn visual_key(&mut self, v: &mut Vim, c: char, edit: bool) -> EditorEvent {
        use EditorEvent::*;
        let linewise = v.mode == VimMode::VisualLine;
        let op = match c {
            'd' | 'x' if edit => Some('d'),
            'c' | 's' if edit => Some('c'),
            'y' => Some('y'),
            '>' | '<' if edit => Some(c),
            _ => None,
        };
        if let Some(op) = op {
            let a = self.anchor.unwrap_or(self.cursor);
            let (lo, hi) = (a.min(self.cursor), a.max(self.cursor));
            let (s, e) = if linewise { self.line_span(lo.row, hi.row) } else { (lo, self.right_in_line(hi, true)) };
            v.mode = VimMode::Normal;
            self.anchor = None;
            self.cursor = lo;
            return self.operate(v, op, s, e, linewise);
        }
        match c {
            'v' | 'V' => {
                let target = if c == 'v' { VimMode::Visual } else { VimMode::VisualLine };
                if v.mode == target {
                    v.mode = VimMode::Normal;
                    self.anchor = None;
                    self.clamp_normal();
                } else {
                    let a = self.anchor.unwrap_or(self.cursor);
                    v.mode = target;
                    v.line_anchor = a.row;
                    if target == VimMode::VisualLine {
                        self.cover_lines(a.row);
                    } else {
                        self.anchor = Some(a);
                    }
                }
                Moved
            }
            'o' => {
                if let Some(a) = self.anchor {
                    self.anchor = Some(self.cursor);
                    self.cursor = a;
                    if linewise {
                        v.line_anchor = self.anchor.map_or(self.cursor.row, |a| a.row);
                    }
                }
                Moved
            }
            'g' => {
                v.prefix = Some('g');
                Moved
            }
            _ => {
                let count = v.count.take();
                if let Some(t) = self.motion(c, count) {
                    self.visual_move(v, t.pos);
                }
                Moved
            }
        }
    }
}
