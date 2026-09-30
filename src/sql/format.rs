use std::borrow::Cow;

use crate::db::Backend;
use crate::sql::keywords;
use crate::sql::lexer::{TokenKind, tokenize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatOptions {
    /// Spaces per indentation level.
    pub indent: usize,
    /// `Some(true)` upper-cases keywords, `Some(false)` lower-cases them, `None` keeps them.
    pub uppercase_keywords: Option<bool>,
    /// Lists (select list, GROUP BY, …) longer than this go one item per line.
    pub max_line: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions { indent: 4, uppercase_keywords: None, max_line: 80 }
    }
}

/// A non-whitespace token with layout facts about its surroundings.
struct FTok<'a> {
    kind: TokenKind,
    text: Cow<'a, str>,
    up: String,
    start: usize,
    /// Preceded by a line break (or the start of input).
    nl_before: bool,
    /// Followed by a line break (or the end of input).
    nl_after: bool,
}

impl FTok<'_> {
    fn is_comment(&self) -> bool {
        matches!(self.kind, TokenKind::LineComment | TokenKind::BlockComment)
    }

    fn is_word(&self) -> bool {
        matches!(self.kind, TokenKind::Keyword | TokenKind::DataType | TokenKind::Builtin | TokenKind::Ident)
    }

    fn kw(&self, w: &str) -> bool {
        self.kind == TokenKind::Keyword && self.up == w
    }
}

fn lex(sql: &str, backend: Backend) -> Vec<FTok<'_>> {
    let toks = tokenize(sql, backend);
    let mut out: Vec<FTok> = Vec::with_capacity(toks.len());
    let mut nl = true;
    for t in &toks {
        if t.kind == TokenKind::Whitespace {
            if t.text(sql).contains('\n') {
                nl = true;
                if let Some(last) = out.last_mut() {
                    last.nl_after = true;
                }
            }
            continue;
        }
        let text = t.text(sql);
        let up = if t.is_word() { text.to_ascii_uppercase() } else { String::new() };
        if let Some(last) = out.last_mut()
            && last.kind == TokenKind::LineComment
        {
            last.nl_after = true;
        }
        out.push(FTok { kind: t.kind, text: Cow::Borrowed(text), up, start: t.start, nl_before: nl, nl_after: false });
        nl = false;
    }
    if let Some(last) = out.last_mut() {
        last.nl_after = true;
    }
    out
}

/// Words that name a relation next, where changing case could change meaning (MySQL table names).
fn names_follow(up: &str) -> bool {
    matches!(up, "FROM" | "JOIN" | "INTO" | "UPDATE" | "TABLE" | "REFERENCES" | "STRAIGHT_JOIN") || up.ends_with("JOIN")
}

fn apply_case(toks: &mut [FTok], upper: bool) {
    let sig: Vec<usize> = (0..toks.len()).filter(|&i| !toks[i].is_comment()).collect();
    for (n, &i) in sig.iter().enumerate() {
        if toks[i].kind != TokenKind::Keyword {
            continue;
        }
        let prev = n.checked_sub(1).map(|p| &toks[sig[p]]);
        let next = sig.get(n + 1).map(|&x| &toks[x]);
        let alias = prev.is_some_and(|p| p.up == "AS")
            && !matches!(toks[i].up.as_str(), "SELECT" | "WITH" | "VALUES" | "TABLE" | "NOT" | "MATERIALIZED" | "IDENTITY" | "ENUM" | "RANGE");
        let is_name = alias
            || prev.is_some_and(|p| p.kind == TokenKind::Dot || names_follow(&p.up))
            || next.is_some_and(|x| x.kind == TokenKind::Dot);
        if is_name {
            continue;
        }
        let t = &mut toks[i];
        t.text = Cow::Owned(if upper { t.text.to_uppercase() } else { t.text.to_lowercase() });
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Clause {
    None,
    /// Comma-separated items: SELECT list, GROUP/ORDER BY, SET, VALUES, RETURNING.
    List,
    /// AND/OR break lines: WHERE, HAVING, join ON.
    Cond,
    Join,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FrameKind {
    /// Statement root or a parenthesized subquery.
    Query,
    /// Function call, IN list, expression group.
    Inline,
    /// `CREATE TABLE t (` column definitions, one per line.
    Columns,
}

struct Frame {
    kind: FrameKind,
    /// Level of clause keywords in this query.
    base: usize,
    /// Level of the line holding the `(`; the `)` goes back there.
    open_level: usize,
    clause: Clause,
    multi: bool,
    /// Token index where the current list's first item starts.
    list_start: usize,
    between: bool,
}

impl Frame {
    fn new(kind: FrameKind, base: usize, open_level: usize) -> Self {
        Frame { kind, base, open_level, clause: Clause::None, multi: false, list_start: usize::MAX, between: false }
    }
}

struct Printer<'a, 'o> {
    src: &'a str,
    toks: Vec<FTok<'a>>,
    close: Vec<Option<usize>>,
    backend: Backend,
    opts: &'o FormatOptions,
    out: String,
    line_level: usize,
    /// 0 = none, 1 = newline, 2 = blank line.
    pending: u8,
    pending_level: usize,
    prev: Option<usize>,
    frames: Vec<Frame>,
    /// (level of the CASE line, frame depth).
    cases: Vec<(usize, usize)>,
    verb: String,
    stmt_start: bool,
}

/// Pretty-prints SQL: major clauses on their own lines, long lists one item per line, AND/OR broken
/// out of WHERE/ON, subqueries and CASE blocks indented. Only whitespace and keyword case change.
pub fn format_sql(sql: &str, backend: Backend, opts: &FormatOptions) -> String {
    let mut toks = lex(sql, backend);
    if let Some(upper) = opts.uppercase_keywords {
        apply_case(&mut toks, upper);
    }
    let mut close = vec![None; toks.len()];
    let mut stack = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            TokenKind::LParen => stack.push(i),
            TokenKind::RParen => {
                if let Some(o) = stack.pop() {
                    close[o] = Some(i);
                }
            }
            TokenKind::Semicolon => stack.clear(),
            _ => {}
        }
    }
    let mut p = Printer {
        src: sql,
        toks,
        close,
        backend,
        opts,
        out: String::with_capacity(sql.len() + sql.len() / 4),
        line_level: 0,
        pending: 0,
        pending_level: 0,
        prev: None,
        frames: vec![Frame::new(FrameKind::Query, 0, 0)],
        cases: Vec::new(),
        verb: String::new(),
        stmt_start: true,
    };
    p.run();
    let trimmed = p.out.trim_end().len();
    p.out.truncate(trimmed);
    p.out
}

impl<'a> Printer<'a, '_> {
    fn newline(&mut self, level: usize) {
        self.pending = self.pending.max(1);
        self.pending_level = level;
    }

    fn blank(&mut self) {
        self.pending = 2;
        self.pending_level = 0;
    }

    /// Writes the pending line break(s) and the new line's indentation.
    fn write_break(&mut self) {
        let trimmed = self.out.trim_end_matches([' ', '\t']).len();
        self.out.truncate(trimmed);
        for _ in 0..self.pending {
            self.out.push('\n');
        }
        self.line_level = self.pending_level;
        self.out.extend(std::iter::repeat_n(' ', self.line_level * self.opts.indent));
        self.pending = 0;
    }

    fn emit(&mut self, i: usize) {
        if self.pending > 0 && !self.out.is_empty() {
            self.write_break();
        } else {
            self.pending = 0;
            if let Some(p) = self.prev
                && self.space_between(p, i)
            {
                self.out.push(' ');
            }
        }
        let t = &self.toks[i];
        self.out.push_str(&t.text);
        self.prev = Some(i);
    }

    fn prev_sig(&self, i: usize) -> Option<usize> {
        (0..i).rev().find(|&j| !self.toks[j].is_comment())
    }

    fn next_sig(&self, i: usize) -> Option<usize> {
        (i + 1..self.toks.len()).find(|&j| !self.toks[j].is_comment())
    }

    fn up(&self, i: Option<usize>) -> &str {
        i.map_or("", |i| self.toks[i].up.as_str())
    }

    fn is_unary(&self, i: usize) -> bool {
        let t = &self.toks[i];
        if t.kind != TokenKind::Operator || !matches!(&*t.text, "-" | "+" | "~") {
            return false;
        }
        match self.prev_sig(i).map(|p| &self.toks[p]) {
            None => true,
            Some(p) => {
                matches!(p.kind, TokenKind::Operator | TokenKind::LParen | TokenKind::Comma | TokenKind::LBracket)
                    || (p.kind == TokenKind::Keyword && !matches!(p.up.as_str(), "NULL" | "TRUE" | "FALSE" | "END"))
            }
        }
    }

    /// A relation name right after INTO/TABLE/REFERENCES…, where a following `(` is a column list.
    fn is_relation_name(&self, i: usize) -> bool {
        let mut k = i;
        while let Some(d) = self.prev_sig(k).filter(|&d| self.toks[d].kind == TokenKind::Dot) {
            match self.prev_sig(d) {
                Some(n) => k = n,
                None => break,
            }
        }
        let before = self.prev_sig(k);
        matches!(self.up(before), "INTO" | "TABLE" | "REFERENCES" | "VIEW" | "EXISTS")
            || (self.up(before) == "ON" && self.verb == "CREATE")
    }

    fn glue_call(&self, a: usize) -> bool {
        let t = &self.toks[a];
        match t.kind {
            TokenKind::Ident | TokenKind::QuotedIdent | TokenKind::Builtin | TokenKind::DataType => !self.is_relation_name(a),
            TokenKind::Keyword => {
                matches!(t.up.as_str(), "CAST" | "CONVERT" | "EXTRACT" | "SUBSTRING" | "POSITION" | "OVERLAY" | "TRIM" | "ROW" | "CHAR")
                    || (keywords::functions(self.backend).contains(&t.up.as_str()) && !self.is_relation_name(a))
            }
            _ => false,
        }
    }

    fn space_between(&self, a: usize, b: usize) -> bool {
        let (ta, tb) = (&self.toks[a], &self.toks[b]);
        if tb.is_comment() || ta.is_comment() {
            return true;
        }
        let glue = match (ta.kind, tb.kind) {
            (_, TokenKind::Comma | TokenKind::Semicolon | TokenKind::RParen | TokenKind::RBracket | TokenKind::Dot) => true,
            (TokenKind::LParen | TokenKind::LBracket | TokenKind::Dot | TokenKind::Backslash, _) => true,
            (_, TokenKind::Backslash) => true,
            (_, TokenKind::LParen) => self.glue_call(a),
            (TokenKind::Ident | TokenKind::QuotedIdent | TokenKind::RParen | TokenKind::RBracket, TokenKind::LBracket) => true,
            (TokenKind::Keyword | TokenKind::Builtin | TokenKind::DataType, TokenKind::LBracket) => true,
            _ => {
                (ta.kind == TokenKind::Operator && &*ta.text == "::")
                    || (tb.kind == TokenKind::Operator && &*tb.text == "::")
                    || (self.is_unary(a) && tb.kind != TokenKind::Operator)
            }
        };
        let merges_into_comment = (ta.text.ends_with('-') && tb.text.starts_with('-'))
            || (ta.text.ends_with('/') && tb.text.starts_with('*'))
            || (ta.text.ends_with('*') && tb.text.starts_with('/'));
        !glue || merges_into_comment
    }

    fn frame(&self) -> &Frame {
        self.frames.last().expect("root frame")
    }

    fn frame_mut(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("root frame")
    }

    fn reset_statement(&mut self) {
        self.frames.truncate(1);
        self.frames[0] = Frame::new(FrameKind::Query, 0, 0);
        self.cases.clear();
        self.verb.clear();
        self.stmt_start = true;
    }

    fn run(&mut self) {
        let mut i = 0;
        while i < self.toks.len() {
            i = self.step(i);
        }
    }

    fn step(&mut self, i: usize) -> usize {
        let kind = self.toks[i].kind;
        if self.toks[i].is_comment() {
            self.comment(i);
            return i + 1;
        }
        if self.stmt_start {
            if kind == TokenKind::Backslash {
                return self.verbatim_line(i);
            }
            self.verb = self.toks[i].up.clone();
            self.stmt_start = false;
        }
        if self.frame().kind == FrameKind::Query {
            self.mark_list_item(i);
        }
        match kind {
            TokenKind::Semicolon => {
                self.emit(i);
                self.reset_statement();
                self.blank();
                return i + 1;
            }
            TokenKind::Backslash => {
                let go = self.toks.get(i + 1).is_some_and(|n| n.start == self.toks[i].start + 1 && matches!(&*n.text, "G" | "g"));
                if go {
                    self.emit(i);
                    self.emit(i + 1);
                    self.reset_statement();
                    self.blank();
                    return i + 2;
                }
                self.emit(i);
                return i + 1;
            }
            TokenKind::LParen => {
                self.open_paren(i);
                return i + 1;
            }
            TokenKind::RParen => {
                self.close_paren(i);
                return i + 1;
            }
            _ => {}
        }

        let depth = self.frames.len();
        if self.toks[i].kind == TokenKind::Keyword {
            match self.toks[i].up.as_str() {
                "CASE" => {
                    self.emit(i);
                    self.cases.push((self.line_level, depth));
                    return i + 1;
                }
                "WHEN" | "ELSE" if self.cases.last().is_some_and(|c| c.1 == depth) => {
                    let lvl = self.cases.last().map_or(0, |c| c.0) + 1;
                    self.newline(lvl);
                    self.emit(i);
                    return i + 1;
                }
                "END" if self.cases.last().is_some_and(|c| c.1 == depth) => {
                    let lvl = self.cases.pop().map_or(0, |c| c.0);
                    self.newline(lvl);
                    self.emit(i);
                    return i + 1;
                }
                _ => {}
            }
        }

        match self.frame().kind {
            FrameKind::Query => self.query_token(i),
            FrameKind::Columns => {
                self.emit(i);
                if kind == TokenKind::Comma {
                    let lvl = self.frame().open_level + 1;
                    self.newline(lvl);
                }
            }
            FrameKind::Inline => self.emit(i),
        }
        i + 1
    }

    fn comment(&mut self, i: usize) {
        let (nl_before, nl_after, line) = {
            let t = &self.toks[i];
            (t.nl_before, t.nl_after, t.kind == TokenKind::LineComment)
        };
        if nl_before && !self.out.is_empty() {
            let lvl = if self.pending > 0 { self.pending_level } else { self.line_level };
            self.newline(lvl);
            self.emit(i);
        } else if self.out.is_empty() {
            self.emit(i);
        } else {
            if !self.out.ends_with(['(', ' ']) {
                self.out.push(' ');
            }
            self.out.push_str(&self.toks[i].text);
            self.prev = Some(i);
        }
        if line || (nl_before && nl_after) {
            let lvl = if self.pending > 0 { self.pending_level } else { self.line_level };
            self.newline(lvl);
        }
    }

    /// psql-style backslash command: copied through the end of its line.
    fn verbatim_line(&mut self, i: usize) -> usize {
        let start = self.toks[i].start;
        let end = self.src[start..].find('\n').map_or(self.src.len(), |n| start + n);
        if self.pending > 0 && !self.out.is_empty() {
            self.write_break();
        }
        self.out.push_str(self.src[start..end].trim_end());
        let mut j = i;
        while j < self.toks.len() && self.toks[j].start < end {
            j += 1;
        }
        self.prev = j.checked_sub(1);
        self.reset_statement();
        self.newline(0);
        j
    }

    fn open_paren(&mut self, i: usize) {
        let next = self.next_sig(i);
        let subquery = matches!(self.up(next), "SELECT" | "WITH");
        let columns = !subquery
            && self.frames.len() == 1
            && self.verb == "CREATE"
            && self.prev_sig(i).is_some_and(|p| {
                (self.toks[p].is_word() || self.toks[p].kind == TokenKind::QuotedIdent) && self.is_relation_name(p)
            })
            && self.table_before(i);
        self.emit(i);
        let lvl = self.line_level;
        if subquery {
            self.frames.push(Frame::new(FrameKind::Query, lvl + 1, lvl));
            self.newline(lvl + 1);
        } else if columns {
            self.frames.push(Frame::new(FrameKind::Columns, lvl + 1, lvl));
            self.newline(lvl + 1);
        } else {
            self.frames.push(Frame::new(FrameKind::Inline, lvl, lvl));
        }
    }

    fn table_before(&self, i: usize) -> bool {
        let mut k = self.prev_sig(i);
        while let Some(j) = k {
            let t = &self.toks[j];
            if t.kw("TABLE") {
                return true;
            }
            if !(t.is_word() || t.kind == TokenKind::QuotedIdent || t.kind == TokenKind::Dot) {
                return false;
            }
            k = self.prev_sig(j);
        }
        false
    }

    fn close_paren(&mut self, i: usize) {
        if self.frames.len() > 1 {
            let f = self.frames.pop().expect("non-root frame");
            let depth = self.frames.len();
            self.cases.retain(|c| c.1 <= depth);
            if f.kind != FrameKind::Inline {
                self.newline(f.open_level);
            }
        }
        self.emit(i);
    }

    /// Breaks before the first item of a multi-line list.
    fn mark_list_item(&mut self, i: usize) {
        let f = self.frame();
        if f.list_start == i && f.multi {
            let lvl = f.base + 1;
            self.newline(lvl);
        }
    }

    fn query_token(&mut self, i: usize) {
        let t = &self.toks[i];
        let kind = t.kind;
        let base = self.frame().base;
        if kind == TokenKind::Comma {
            self.emit(i);
            let f = self.frame();
            if f.clause == Clause::List && f.multi {
                self.newline(base + 1);
            }
            return;
        }
        if kind != TokenKind::Keyword {
            self.emit(i);
            return;
        }
        let up = t.up.clone();
        let in_case = self.cases.last().is_some_and(|c| c.1 == self.frames.len());
        match up.as_str() {
            "AND" | "OR" if self.frame().clause == Clause::Cond && !in_case => {
                if up == "AND" && self.frame().between {
                    self.frame_mut().between = false;
                } else {
                    self.newline(base + 1);
                }
                self.emit(i);
                return;
            }
            "BETWEEN" => {
                self.frame_mut().between = true;
                self.emit(i);
                return;
            }
            _ => {}
        }
        if let Some((clause, newline, list_from)) = self.clause_start(i) {
            if newline {
                self.newline(base);
            }
            let multi = clause == Clause::List && self.list_is_long(list_from, base);
            let f = self.frame_mut();
            f.clause = clause;
            f.between = false;
            f.multi = multi;
            f.list_start = list_from;
        }
        self.emit(i);
    }

    fn is_join_start(&self, i: usize) -> bool {
        let t = &self.toks[i];
        if t.kind != TokenKind::Keyword {
            return false;
        }
        const MODS: [&str; 7] = ["LEFT", "RIGHT", "FULL", "INNER", "CROSS", "NATURAL", "OUTER"];
        if t.up == "JOIN" || t.up == "STRAIGHT_JOIN" {
            return !MODS.contains(&self.up(self.prev_sig(i)));
        }
        if !MODS.contains(&t.up.as_str()) || MODS.contains(&self.up(self.prev_sig(i))) {
            return false;
        }
        let mut k = self.next_sig(i);
        while let Some(j) = k {
            match self.toks[j].up.as_str() {
                "JOIN" => return true,
                w if MODS.contains(&w) => k = self.next_sig(j),
                _ => return false,
            }
        }
        false
    }

    /// Whether token `i` starts a clause in the current query: (kind, break before, first list item).
    fn clause_start(&self, i: usize) -> Option<(Clause, bool, usize)> {
        let up = self.toks[i].up.as_str();
        let prev = self.up(self.prev_sig(i));
        let after = |n: usize| -> usize {
            let mut k = Some(i);
            for _ in 0..n {
                k = k.and_then(|k| self.next_sig(k));
            }
            k.and_then(|k| self.next_sig(k)).unwrap_or(usize::MAX)
        };
        let first = self.prev_sig(i).is_none_or(|p| {
            self.toks[p].kind == TokenKind::Semicolon || (self.toks[p].kind == TokenKind::LParen && self.frames.len() > 1)
        });
        if self.is_join_start(i) {
            return Some((Clause::Join, true, usize::MAX));
        }
        Some(match up {
            "SELECT" => {
                let mut k = after(0);
                while matches!(self.up(Some(k).filter(|&k| k < self.toks.len())), "DISTINCT" | "ALL") {
                    k = self.next_sig(k).unwrap_or(usize::MAX);
                    if self.up(Some(k).filter(|&k| k < self.toks.len())) == "ON" {
                        let paren = self.next_sig(k).filter(|&p| self.toks[p].kind == TokenKind::LParen);
                        k = paren.and_then(|p| self.close[p]).and_then(|c| self.next_sig(c)).unwrap_or(usize::MAX);
                    }
                }
                (Clause::List, !first, k)
            }
            "FROM" => (Clause::Other, prev != "DELETE" && !first, usize::MAX),
            "WHERE" | "HAVING" => (Clause::Cond, true, usize::MAX),
            "GROUP" | "ORDER" if self.up(self.next_sig(i)) == "BY" => (Clause::List, true, after(1)),
            "LIMIT" | "OFFSET" | "FETCH" | "WINDOW" => (Clause::Other, true, usize::MAX),
            "UNION" | "INTERSECT" | "EXCEPT" | "MINUS" => (Clause::Other, true, usize::MAX),
            "VALUES" if prev != "DEFAULT" && !first => (Clause::List, true, after(0)),
            "SET" if !first && (self.verb == "UPDATE" || prev == "UPDATE" || self.verb == "WITH") => {
                (Clause::List, true, after(0))
            }
            "RETURNING" => (Clause::List, true, after(0)),
            "ON" if self.frame().clause == Clause::Join => (Clause::Cond, false, usize::MAX),
            "ON" if matches!(self.up(self.next_sig(i)), "CONFLICT" | "DUPLICATE") => (Clause::Other, true, usize::MAX),
            "INSERT" | "UPDATE" | "DELETE" | "MERGE"
                if self.verb == "WITH" && self.prev_sig(i).is_some_and(|p| self.toks[p].kind == TokenKind::RParen) =>
            {
                (Clause::Other, true, usize::MAX)
            }
            _ => return None,
        })
    }

    /// Lookahead from the first list item: long, or holding CASE, a subquery or a line comment.
    fn list_is_long(&self, from: usize, base: usize) -> bool {
        let mut len = base * self.opts.indent + 8;
        let mut depth = 0usize;
        let mut j = from;
        while j < self.toks.len() {
            let t = &self.toks[j];
            if t.kind == TokenKind::LineComment || (t.kind == TokenKind::BlockComment && t.nl_before) {
                return true;
            }
            match t.kind {
                TokenKind::LParen => {
                    if matches!(self.up(self.next_sig(j)), "SELECT" | "WITH") {
                        return true;
                    }
                    depth += 1;
                }
                TokenKind::RParen => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                TokenKind::Semicolon | TokenKind::Backslash => break,
                TokenKind::Keyword => {
                    if t.up == "CASE" {
                        return true;
                    }
                    if depth == 0 && self.ends_list(j) {
                        break;
                    }
                }
                _ => {}
            }
            let space = j > from && self.space_between(j - 1, j);
            len += t.text.chars().count() + usize::from(space);
            j += 1;
        }
        len > self.opts.max_line
    }

    fn ends_list(&self, j: usize) -> bool {
        matches!(
            self.toks[j].up.as_str(),
            "FROM" | "WHERE" | "GROUP" | "ORDER" | "HAVING" | "LIMIT" | "OFFSET" | "FETCH" | "UNION" | "INTERSECT"
                | "EXCEPT" | "MINUS" | "WINDOW" | "INTO" | "RETURNING" | "VALUES" | "SET" | "SELECT"
        ) || self.is_join_start(j)
            || (self.toks[j].up == "ON" && matches!(self.up(self.next_sig(j)), "CONFLICT" | "DUPLICATE"))
    }
}

/// Collapses SQL onto one line. Line comments become block comments so nothing after them is swallowed.
pub fn compact_sql(sql: &str, backend: Backend) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut space = false;
    for t in tokenize(sql, backend) {
        let text = t.text(sql);
        let piece: Cow<str> = match t.kind {
            TokenKind::Whitespace => {
                space = !out.is_empty();
                continue;
            }
            TokenKind::LineComment => {
                let body = text.strip_prefix("--").or_else(|| text.strip_prefix('#')).unwrap_or(text).trim();
                if body.is_empty() {
                    continue;
                }
                Cow::Owned(format!("/* {} */", body.replace("*/", "* /").replace("/*", "/ *")))
            }
            TokenKind::BlockComment if text.contains(['\n', '\r']) => {
                Cow::Owned(text.split_whitespace().collect::<Vec<_>>().join(" "))
            }
            _ => Cow::Borrowed(text),
        };
        if space {
            out.push(' ');
            space = false;
        }
        out.push_str(&piece);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(sql: &str, b: Backend) -> String {
        format_sql(sql, b, &FormatOptions::default())
    }

    fn upper(sql: &str) -> String {
        format_sql(sql, Backend::Postgres, &FormatOptions { uppercase_keywords: Some(true), ..FormatOptions::default() })
    }

    /// Non-whitespace tokens: formatting may only change the whitespace between them.
    fn sig(sql: &str, b: Backend) -> Vec<(TokenKind, String)> {
        tokenize(sql, b)
            .into_iter()
            .filter(|t| t.kind != TokenKind::Whitespace)
            .map(|t| (t.kind, t.text(sql).to_string()))
            .collect()
    }

    const CORPUS: &[(&str, Backend)] = &[
        ("select a, b from t where x = 1 and y between 1 and 5 or z is null order by a desc limit 10", Backend::Postgres),
        ("select u.id, count(*) as n from users u left join orders o on o.user_id = u.id and o.total > 0 group by u.id having count(*) > 1", Backend::Postgres),
        ("with recent as (select * from orders where created_at > now() - interval '1 day'), top as (select user_id from recent) select * from top", Backend::Postgres),
        ("select case when a = 1 then 'one' when a = 2 then 'two' else 'many' end as label, b from t", Backend::Postgres),
        ("select * from (select id, (select max(x) from y where y.id = t.id) m from t) s where s.m in (select 1 union all select 2)", Backend::Postgres),
        ("insert into t (a, b) values (1, 'x'), (2, 'y') on conflict (a) do update set b = excluded.b returning *", Backend::Postgres),
        ("update t set a = 1, b = -2 where id = $1; delete from t where id in (1, 2, 3);", Backend::Postgres),
        ("create table if not exists t (id serial primary key, name text not null, price numeric(10, 2) default 0)", Backend::Postgres),
        ("select x::int, arr[1], data->>'k', -x, 1 - -1, (a).b from t -- trailing\n-- own line\nwhere /* inline */ true", Backend::Postgres),
        ("select `weird col`, \"str\" from `t` # mysql comment\nwhere a <> 'it''s' limit 1, 2\\G select 1", Backend::MySql),
        ("select [col x], 'a''b' from [my table] where x glob 'a*'; pragma table_info(t)", Backend::Sqlite),
        ("\\x on\nselect $fn$ select 1; $fn$ as body, e'\\n' from t", Backend::Postgres),
        ("select very_long_column_name_number_one, very_long_column_name_number_two, very_long_column_name_number_three from t", Backend::Postgres),
        ("select a from t where exists (select 1 from u where u.a = t.a and u.b = 2) and not (x or y)", Backend::Postgres),
        ("select count(distinct a) filter (where b) over (partition by c order by d) from t", Backend::Postgres),
        ("SELECT a FROM t\n\n-- comment between\n\nSELECT b FROM u;", Backend::Postgres),
        ("/* header */ select 1; select 2 /* tail */", Backend::Sqlite),
        ("explain analyze select * from t cross join u natural join v full outer join w using (id)", Backend::Postgres),
        ("select case when a then case when b then 1 else 2 end else 3 end from t where case when x and y then 1 end = 1", Backend::Postgres),
        ("select ( -- c\n a), (/* d */ b) from t where (select 1) = 1", Backend::Postgres),
        ("select a, /* c */ b from t", Backend::MySql),
        ("select (a from t; select a) from t where ((x", Backend::Sqlite),
        ("select 1 union select 2 order by 1", Backend::Sqlite),
        ("", Backend::Postgres),
        ("-- only a comment", Backend::Postgres),
    ];

    #[test]
    fn formatting_is_idempotent() {
        let up = FormatOptions { uppercase_keywords: Some(true), ..FormatOptions::default() };
        for (sql, b) in CORPUS {
            let once = fmt(sql, *b);
            assert_eq!(fmt(&once, *b), once, "not idempotent for {sql:?}:\n{once}");
            let once = format_sql(sql, *b, &up);
            assert_eq!(format_sql(&once, *b, &up), once, "not idempotent (upper) for {sql:?}:\n{once}");
        }
    }

    #[test]
    fn formatting_only_changes_whitespace() {
        for (sql, b) in CORPUS {
            assert_eq!(sig(&fmt(sql, *b), *b), sig(sql, *b), "tokens changed for {sql:?}");
        }
    }

    #[test]
    fn major_clauses_start_lines_and_conditions_break_on_and_or() {
        assert_eq!(
            fmt(CORPUS[0].0, Backend::Postgres),
            "select a, b\nfrom t\nwhere x = 1\n    and y between 1 and 5\n    or z is null\norder by a desc\nlimit 10"
        );
    }

    #[test]
    fn joins_keep_on_and_break_its_conditions() {
        assert_eq!(
            fmt("select * from a left outer join b on a.id = b.a_id and b.x > 0 join c using (id)", Backend::Postgres),
            "select *\nfrom a\nleft outer join b on a.id = b.a_id\n    and b.x > 0\njoin c using (id)"
        );
    }

    #[test]
    fn subqueries_are_indented_blocks() {
        assert_eq!(
            fmt("select * from (select id from t where a = 1) s where s.id in (select id from u)", Backend::Postgres),
            "select *\nfrom (\n    select id\n    from t\n    where a = 1\n) s\nwhere s.id in (\n    select id\n    from u\n)"
        );
        assert_eq!(fmt("with x as (select 1) select * from x", Backend::Postgres), "with x as (\n    select 1\n)\nselect *\nfrom x");
    }

    #[test]
    fn long_select_lists_go_one_item_per_line() {
        assert_eq!(
            fmt(CORPUS[12].0, Backend::Postgres),
            "select\n    very_long_column_name_number_one,\n    very_long_column_name_number_two,\n    very_long_column_name_number_three\nfrom t"
        );
        assert_eq!(fmt("select a, b from t", Backend::Postgres), "select a, b\nfrom t", "short lists stay inline");
    }

    #[test]
    fn case_blocks_are_laid_out() {
        assert_eq!(
            upper("select case when a = 1 then 'one' else 'many' end as label from t"),
            "SELECT\n    CASE\n        WHEN a = 1 THEN 'one'\n        ELSE 'many'\n    END AS label\nFROM t"
        );
    }

    #[test]
    fn comments_strings_and_quoted_identifiers_survive() {
        let sql = "select \"Mixed Col\", 'a  --  b' /* keep  me */ from t -- tail\n-- own line\nwhere x = $$ a;\n b $$";
        let out = fmt(sql, Backend::Postgres);
        for piece in ["\"Mixed Col\"", "'a  --  b'", "/* keep  me */", "-- tail", "-- own line", "$$ a;\n b $$"] {
            assert!(out.contains(piece), "{piece:?} lost in:\n{out}");
        }
        assert!(out.contains("-- tail\n"), "a line comment must still end its line:\n{out}");
    }

    #[test]
    fn each_backends_quoting_is_preserved() {
        assert_eq!(fmt("select `a b`, \"s\" from `t` # c\nwhere x = 1", Backend::MySql), "select `a b`, \"s\"\nfrom `t` # c\nwhere x = 1");
        assert_eq!(fmt("select [a b] from [t] where y = 'q'", Backend::Sqlite), "select [a b]\nfrom [t]\nwhere y = 'q'");
        assert_eq!(fmt("select \"a b\" from \"T\"", Backend::Postgres), "select \"a b\"\nfrom \"T\"");
    }

    #[test]
    fn multiple_statements_are_separated_by_a_blank_line() {
        assert_eq!(fmt("select 1;select 2 from t;", Backend::Postgres), "select 1;\n\nselect 2\nfrom t;");
        assert_eq!(fmt("select 1\\G select 2", Backend::MySql), "select 1\\G\n\nselect 2");
        assert_eq!(fmt("\\x auto\nselect 1", Backend::Postgres), "\\x auto\nselect 1");
    }

    #[test]
    fn uppercase_keywords_leaves_identifiers_and_table_names() {
        assert_eq!(upper("select user, name from user where value = 1"), "SELECT USER, name\nFROM user\nWHERE VALUE = 1");
        let lower = FormatOptions { uppercase_keywords: Some(false), ..FormatOptions::default() };
        assert_eq!(format_sql("SELECT A FROM T", Backend::Postgres, &lower), "select A\nfrom T");
    }

    #[test]
    fn spacing_never_creates_comments_or_merges_operators() {
        let sql = "select 1 - -1, a/ *b, - -x from t";
        let out = fmt(sql, Backend::Postgres);
        assert_eq!(sig(&out, Backend::Postgres), sig(sql, Backend::Postgres), "{out}");
    }

    #[test]
    fn insert_update_and_create_table_layout() {
        assert_eq!(
            fmt("insert into t (a, b) values (1, 2) returning id", Backend::Postgres),
            "insert into t (a, b)\nvalues (1, 2)\nreturning id"
        );
        assert_eq!(fmt("update t set a = 1, b = 2 where id = 3", Backend::Postgres), "update t\nset a = 1, b = 2\nwhere id = 3");
        assert_eq!(
            fmt("create table t (id int primary key, name text)", Backend::Postgres),
            "create table t (\n    id int primary key,\n    name text\n)"
        );
    }

    #[test]
    fn compact_collapses_to_one_line_and_neutralizes_line_comments() {
        let sql = "select a, -- first */ col\n  b\nfrom t # mysql\nwhere x = 'multi\nline'";
        let out = compact_sql(sql, Backend::MySql);
        assert_eq!(out, "select a, /* first * / col */ b from t /* mysql */ where x = 'multi\nline'");
        assert_eq!(sig(&out, Backend::MySql).iter().filter(|t| t.0 == TokenKind::BlockComment).count(), 2);
        assert_eq!(compact_sql("  select /* a\n b */ 1  ", Backend::Postgres), "select /* a b */ 1");
    }}
