use crate::db::Backend;
use crate::sql::lexer::{Token, TokenKind, tokenize, unquote_ident};
use crate::sql::split::{Terminator, split};

use super::scope::{self, Cte, TableRef};

pub(crate) const ROOT: usize = usize::MAX;

/// A significant token of the statement under the cursor.
#[derive(Clone, Debug)]
pub(crate) struct Tok<'a> {
    pub kind: TokenKind,
    pub text: &'a str,
    /// Upper-cased text for words, empty otherwise.
    pub up: String,
    /// Index (into `Stmt::toks`) of the enclosing `(`, or `ROOT`.
    pub group: usize,
}

impl Tok<'_> {
    pub fn is_kw(&self, w: &str) -> bool {
        self.kind == TokenKind::Keyword && self.up == w
    }

    /// Identifier-like: plain or quoted identifier, or a word that is not a clause keyword.
    pub fn is_name(&self) -> bool {
        match self.kind {
            TokenKind::Ident | TokenKind::QuotedIdent | TokenKind::DataType | TokenKind::Builtin => true,
            TokenKind::Keyword => !is_structural(&self.up),
            _ => false,
        }
    }

    pub fn name(&self) -> String {
        if self.kind == TokenKind::QuotedIdent { unquote_ident(self.text) } else { self.text.to_string() }
    }
}

pub(crate) fn is_structural(up: &str) -> bool {
    matches!(
        up,
        "SELECT" | "FROM" | "WHERE" | "GROUP" | "ORDER" | "BY" | "HAVING" | "LIMIT" | "OFFSET" | "JOIN" | "INNER"
            | "LEFT" | "RIGHT" | "FULL" | "OUTER" | "CROSS" | "NATURAL" | "ON" | "USING" | "UNION" | "INTERSECT"
            | "EXCEPT" | "MINUS" | "AS" | "SET" | "VALUES" | "INTO" | "WINDOW" | "RETURNING" | "FETCH" | "FOR"
            | "WITH" | "AND" | "OR" | "NOT" | "IS" | "IN" | "LIKE" | "ILIKE" | "BETWEEN" | "CASE" | "WHEN" | "THEN"
            | "ELSE" | "END" | "DISTINCT" | "ALL" | "ANY" | "SOME" | "EXISTS" | "NULL" | "TRUE" | "FALSE" | "INSERT"
            | "UPDATE" | "DELETE" | "CREATE" | "ALTER" | "DROP" | "TABLE" | "VIEW" | "INDEX" | "LATERAL" | "ONLY"
            | "STRAIGHT_JOIN" | "PARTITION" | "OVER" | "FILTER" | "ASC" | "DESC" | "NULLS" | "PRIMARY" | "FOREIGN"
            | "REFERENCES" | "DEFAULT" | "CONSTRAINT" | "UNIQUE" | "CHECK" | "COLUMN" | "ADD" | "RENAME" | "TO"
            | "IF" | "USE" | "DATABASE" | "SCHEMA" | "FUNCTION" | "PROCEDURE" | "TRIGGER" | "CALL" | "GRANT"
            | "REVOKE" | "TRUNCATE" | "DESCRIBE" | "EXPLAIN" | "SHOW" | "TYPE" | "CAST" | "LOCK" | "RECURSIVE"
    )
}

/// What kind of candidates the cursor position calls for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ctx {
    None,
    /// Statement start: starting keywords (and plain-word special commands).
    Start,
    /// Special command names (`\dt`, `.tables`).
    Specials,
    Keywords(&'static [&'static str]),
    AllKeywords,
    /// After a complete term: likely next keywords first, then all keywords.
    Following(&'static [&'static str]),
    /// Expression position: columns, aliases, functions, keywords.
    Expr,
    Columns(Scope),
    ColumnsThenKeywords(&'static [&'static str]),
    Relations { join: bool, alias: bool },
    JoinOn,
    DataTypes,
    Databases,
    Schemas,
    Users,
    Functions,
    Favorites,
    TableFormats,
    Themes,
    Files,
    /// `SELECT *` / `SELECT t.*` expansion.
    Star,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Every table reference visible at the cursor.
    Visible,
    /// One reference (index into `Stmt::refs`).
    Ref(usize),
    /// `JOIN … USING (`: bare names, those shared by several tables first.
    Using,
}

/// The word under the cursor.
#[derive(Clone, Debug, Default)]
pub(crate) struct Word {
    /// Byte offset where replacement starts.
    pub start: usize,
    /// Unquoted text typed so far, used for matching.
    pub prefix: String,
    /// Qualifier parts before the word (`schema.` / `alias.` / `schema.table.`), unquoted.
    pub qualifier: Vec<String>,
    /// The word is a (partial) quoted identifier, so keywords cannot match.
    pub quoted: bool,
}

pub(crate) struct Stmt<'a> {
    pub toks: Vec<Tok<'a>>,
    /// Number of tokens before the cursor word.
    pub cur: usize,
    /// Groups enclosing the cursor, innermost first, ending with `ROOT`.
    pub chain: Vec<usize>,
    /// Matching `)` for each `(` token index.
    pub close: Vec<Option<usize>>,
    pub refs: Vec<TableRef>,
    pub ctes: Vec<Cte>,
}

impl<'a> Stmt<'a> {
    pub fn empty() -> Stmt<'static> {
        Stmt::new("", &[], 0)
    }

    fn new(src: &'a str, toks: &[Token], cur_tokens: usize) -> Self {
        let mut out: Vec<Tok<'a>> = Vec::with_capacity(toks.len());
        let mut close = vec![None; toks.len()];
        let mut stack: Vec<usize> = Vec::new();
        let mut chain = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            if i == cur_tokens {
                chain = stack.iter().rev().copied().chain([ROOT]).collect();
            }
            if t.kind == TokenKind::RParen
                && let Some(o) = stack.pop()
            {
                close[o] = Some(i);
            }
            let group = stack.last().copied().unwrap_or(ROOT);
            let text = t.text(src);
            let up = if t.is_word() { text.to_ascii_uppercase() } else { String::new() };
            out.push(Tok { kind: t.kind, text, up, group });
            if t.kind == TokenKind::LParen {
                stack.push(i);
            }
        }
        if cur_tokens >= toks.len() {
            chain = stack.iter().rev().copied().chain([ROOT]).collect();
        }
        let mut st = Stmt { toks: out, cur: cur_tokens, chain, close, refs: Vec::new(), ctes: Vec::new() };
        st.refs = scope::extract_refs(&st);
        st.ctes = scope::extract_ctes(&st);
        st
    }

    pub fn verb(&self) -> &str {
        self.toks.first().map(|t| t.up.as_str()).unwrap_or("")
    }

    fn up(&self, i: Option<usize>) -> &str {
        i.and_then(|i| self.toks.get(i)).map(|t| t.up.as_str()).unwrap_or("")
    }

    pub fn visible_refs(&self) -> impl Iterator<Item = (usize, &TableRef)> {
        self.refs.iter().enumerate().filter(|(_, r)| self.chain.contains(&r.group))
    }

    /// Refs of the innermost query at the cursor (what `SELECT *` expands to).
    pub fn local_refs(&self) -> impl Iterator<Item = (usize, &TableRef)> {
        let g = self.chain[0];
        self.refs.iter().enumerate().filter(move |(_, r)| r.group == g)
    }

    pub fn find_ref(&self, name: &str) -> Option<usize> {
        let by = |f: &dyn Fn(&TableRef) -> bool| {
            self.visible_refs().find(|(_, r)| f(r)).or_else(|| self.refs.iter().enumerate().find(|(_, r)| f(r)))
        };
        by(&|r| r.refname() == name)
            .or_else(|| by(&|r| r.refname().eq_ignore_ascii_case(name)))
            .map(|(i, _)| i)
    }

    /// Index of the first token of a dotted name ending at `end`.
    fn name_start(&self, end: usize) -> usize {
        let mut k = end;
        while k >= 2 && self.toks[k - 1].kind == TokenKind::Dot && self.toks[k - 2].is_name() {
            k -= 2;
        }
        k
    }
}

pub(crate) struct Analysis<'a> {
    pub word: Word,
    pub ctx: Ctx,
    pub stmt: Stmt<'a>,
}

/// Byte range of the statement containing `cursor`.
fn region(src: &str, all: &[Token], backend: Backend, cursor: usize) -> (usize, usize) {
    let (mut lo, mut hi) = (0, src.len());
    for s in split(src, backend, ";") {
        if s.terminator == Terminator::None {
            continue;
        }
        let mut k = all.partition_point(|t| t.start < s.end);
        while k < all.len() && all[k].is_trivia() {
            k += 1;
        }
        let Some(t) = all.get(k) else { break };
        let term_end = if t.kind == TokenKind::Backslash { all.get(k + 1).map_or(t.end, |n| n.end) } else { t.end };
        if term_end <= cursor {
            lo = term_end;
        } else {
            hi = t.start.max(cursor);
            break;
        }
    }
    (lo, hi)
}

fn inside_opaque(t: &Token, cursor: usize) -> bool {
    match t.kind {
        TokenKind::String | TokenKind::BlockComment => cursor < t.end || t.unterminated,
        TokenKind::LineComment => true,
        _ => false,
    }
}

/// Argument kind of a special command, keyed by its name (`\dt`, `.open`, `source`).
fn special_arg(cmd: &str) -> Option<Ctx> {
    let cmd = cmd.strip_suffix('+').unwrap_or(cmd);
    Some(match cmd {
        "\\c" | "\\connect" | "\\l" | "\\u" | "use" | "connect" | "\\r" => Ctx::Databases,
        "\\d" | "\\dt" | "\\dv" | "\\dm" | "\\de" | "\\dE" | "\\di" | "\\dp" | "\\z" | "\\sv" | ".schema"
        | ".indexes" | ".indices" | ".tables" | ".views" | ".dump" => Ctx::Relations { join: false, alias: false },
        "\\df" | "\\sf" => Ctx::Functions,
        "\\dn" => Ctx::Schemas,
        "\\du" | "\\dg" => Ctx::Users,
        "\\dT" => Ctx::DataTypes,
        "\\f" | "\\fd" => Ctx::Favorites,
        "\\T" | "\\tableformat" | ".mode" => Ctx::TableFormats,
        "\\theme" => Ctx::Themes,
        "\\i" | "\\ir" | "\\o" | "\\once" | "\\export" | "\\." | "source" | "tee" | ".read" | ".open" | ".output"
        | ".once" | ".load" | ".import" | ".backup" | ".restore" => Ctx::Files,
        _ => return None,
    })
}

pub(crate) fn analyze(src: &str, cursor: usize, backend: Backend) -> Option<Analysis<'_>> {
    let all = tokenize(src, backend);
    let (lo, hi) = region(src, &all, backend, cursor);
    let toks: Vec<Token> = all.into_iter().filter(|t| t.start >= lo && t.end <= hi).collect();
    let first = toks.iter().position(|t| !t.is_trivia());

    if let Some(f) = first
        && let Some(a) = special_command(src, &toks, f, cursor, backend)
    {
        return a;
    }

    let at = toks.iter().position(|t| t.start < cursor && cursor <= t.end);
    let mut word = Word { start: cursor, ..Word::default() };
    let mut word_end = cursor;
    let mut star = None;
    if let Some(ai) = at {
        let t = toks[ai];
        if inside_opaque(&t, cursor) {
            return None;
        }
        match t.kind {
            TokenKind::Keyword | TokenKind::DataType | TokenKind::Builtin | TokenKind::Ident => {
                word.start = t.start;
                word.prefix = src[t.start..cursor].to_string();
                word_end = t.end;
            }
            TokenKind::QuotedIdent => {
                word.start = t.start;
                word_end = t.end;
                word.quoted = true;
                word.prefix = if cursor == t.end && !t.unterminated {
                    unquote_ident(t.text(src))
                } else {
                    let q = &src[t.start..t.start + 1];
                    src[t.start + 1..cursor].replace(&format!("{q}{q}"), q)
                };
            }
            TokenKind::Operator if t.text(src) == "*" && cursor == t.end => {
                word.start = t.start;
                word_end = t.end;
                star = Some(t.start);
            }
            TokenKind::Number | TokenKind::Parameter | TokenKind::Variable => return None,
            _ => {}
        }
    }

    // qualifier: `a.` / `a.b.` immediately before the word
    let mut qual_start = word.start;
    let sig_before: Vec<&Token> = toks.iter().filter(|t| !t.is_trivia() && t.end <= word.start).collect();
    let mut k = sig_before.len();
    while word.qualifier.len() < 2 && k >= 2 {
        let (dot, name) = (sig_before[k - 1], sig_before[k - 2]);
        let adjacent = dot.end == qual_start && name.end == dot.start;
        let is_name = matches!(
            name.kind,
            TokenKind::Ident | TokenKind::QuotedIdent | TokenKind::Keyword | TokenKind::DataType | TokenKind::Builtin
        );
        if dot.kind != TokenKind::Dot || !adjacent || !is_name {
            break;
        }
        word.qualifier.insert(0, unquote_ident(name.text(src)));
        qual_start = name.start;
        k -= 2;
    }

    let sig: Vec<Token> =
        toks.iter().filter(|t| !t.is_trivia() && (t.end <= qual_start || t.start >= word_end)).copied().collect();
    let cur = sig.iter().take_while(|t| t.end <= qual_start).count();
    let stmt = Stmt::new(src, &sig, cur);

    if let Some(s) = star {
        if !star_position(&stmt) {
            return None;
        }
        word.start = if word.qualifier.is_empty() { s } else { qual_start };
        return Some(Analysis { word, ctx: Ctx::Star, stmt });
    }

    let ctx = if cur == 0 {
        if word.qualifier.is_empty() { Ctx::Start } else { Ctx::Expr }
    } else {
        ctx_at(&stmt, cur - 1, backend)
    };
    Some(Analysis { word, ctx, stmt })
}

/// Handles `\cmd`, sqlite `.cmd` and mysql plain-word commands (`source`, `tee`).
fn special_command<'a>(
    src: &'a str,
    toks: &[Token],
    f: usize,
    cursor: usize,
    backend: Backend,
) -> Option<Option<Analysis<'a>>> {
    let t0 = toks[f];
    let is_cmd = match t0.kind {
        TokenKind::Backslash => true,
        TokenKind::Dot => {
            backend == Backend::Sqlite
                && (cursor == t0.end || toks.get(f + 1).is_some_and(|n| n.start == t0.end && n.is_word()))
        }
        TokenKind::Ident | TokenKind::Keyword if backend == Backend::MySql => {
            matches!(t0.text(src).to_ascii_lowercase().as_str(), "source" | "tee" | "connect")
        }
        _ => false,
    };
    if !is_cmd || cursor < t0.start {
        return None;
    }
    let cmd_end = src[t0.start..].find(char::is_whitespace).map_or(src.len(), |i| t0.start + i);
    if cursor <= cmd_end {
        let word = Word { start: t0.start, prefix: src[t0.start..cursor].to_string(), ..Word::default() };
        let stmt = Stmt::new(src, &[], 0);
        return Some(Some(Analysis { word, ctx: Ctx::Specials, stmt }));
    }
    let cmd = &src[t0.start..cmd_end];
    let cmd_key = if t0.kind == TokenKind::Backslash { cmd.to_string() } else { cmd.to_ascii_lowercase() };
    let Some(kind) = special_arg(&cmd_key) else {
        return Some(None);
    };
    let before = &src[cmd_end..cursor];
    let arg_start = before.rfind(char::is_whitespace).map_or(cmd_end, |i| cmd_end + i + 1);
    if kind == Ctx::Files {
        let word = Word { start: arg_start, prefix: src[arg_start..cursor].to_string(), ..Word::default() };
        let stmt = Stmt::new(src, &[], 0);
        return Some(Some(Analysis { word, ctx: Ctx::Files, stmt }));
    }
    if !src[cmd_end..arg_start].trim().is_empty() {
        return Some(None);
    }
    let arg_toks: Vec<Token> = toks.iter().filter(|t| t.start >= cmd_end).copied().collect();
    Some(analyze_arg(src, &arg_toks, cursor).map(|mut a| {
        a.ctx = kind;
        a
    }))
}

/// Word/qualifier extraction for a special command argument; context is set by the caller.
fn analyze_arg<'a>(src: &'a str, toks: &[Token], cursor: usize) -> Option<Analysis<'a>> {
    let mut word = Word { start: cursor, ..Word::default() };
    if let Some(t) = toks.iter().find(|t| t.start < cursor && cursor <= t.end) {
        if inside_opaque(t, cursor) {
            return None;
        }
        match t.kind {
            TokenKind::Keyword | TokenKind::DataType | TokenKind::Builtin | TokenKind::Ident => {
                word.start = t.start;
                word.prefix = src[t.start..cursor].to_string();
            }
            TokenKind::QuotedIdent => {
                word.start = t.start;
                word.quoted = true;
                word.prefix = src[t.start + 1..cursor].trim_end_matches(['"', '`', ']']).to_string();
            }
            TokenKind::Dot | TokenKind::Whitespace => {}
            _ => return None,
        }
    }
    let sig: Vec<&Token> = toks.iter().filter(|t| !t.is_trivia() && t.end <= word.start).collect();
    if sig.len() >= 2 && sig[sig.len() - 1].kind == TokenKind::Dot && sig[sig.len() - 1].end == word.start {
        let n = sig[sig.len() - 2];
        word.qualifier.push(unquote_ident(n.text(src)));
    }
    Some(Analysis { word, ctx: Ctx::None, stmt: Stmt::new(src, &[], 0) })
}

/// Whether a `*` right before the cursor is a select-list star.
fn star_position(st: &Stmt) -> bool {
    let Some(p) = st.cur.checked_sub(1) else { return false };
    let t = &st.toks[p];
    match t.kind {
        TokenKind::Comma => comma_anchor(st, p).is_some_and(|a| matches!(st.toks[a].up.as_str(), "SELECT" | "DISTINCT")),
        _ => t.is_kw("SELECT") || t.is_kw("DISTINCT") || (t.is_kw("ALL") && st.up(p.checked_sub(1)) == "SELECT"),
    }
}

fn ctx_at(st: &Stmt, i: usize, backend: Backend) -> Ctx {
    let t = &st.toks[i];
    match t.kind {
        TokenKind::LParen => paren_ctx(st, i),
        TokenKind::Comma => comma_ctx(st, i, backend),
        TokenKind::Operator => match t.text {
            "::" => Ctx::DataTypes,
            "*" => {
                let p = st.toks.get(i.wrapping_sub(1));
                if p.is_none_or(|p| p.is_kw("SELECT") || p.is_kw("DISTINCT") || p.kind == TokenKind::Comma || p.kind == TokenKind::Dot) {
                    following(st, i)
                } else {
                    Ctx::Expr
                }
            }
            _ => Ctx::Expr,
        },
        TokenKind::RParen | TokenKind::Number | TokenKind::String | TokenKind::Parameter | TokenKind::Variable => {
            following(st, i)
        }
        TokenKind::Keyword => keyword_ctx(st, i, backend),
        TokenKind::Ident | TokenKind::QuotedIdent | TokenKind::DataType | TokenKind::Builtin => ident_ctx(st, i),
        _ => Ctx::None,
    }
}

const SUBQUERY_KW: &[&str] = &["SELECT", "WITH", "VALUES"];
const IS_KW: &[&str] = &["NULL", "NOT NULL", "NOT", "TRUE", "FALSE", "DISTINCT FROM", "NOT DISTINCT FROM"];
const ALTER_SUB_KW: &[&str] = &["COLUMN", "CONSTRAINT", "INDEX", "PRIMARY KEY", "FOREIGN KEY", "DEFAULT"];
const ADD_KW: &[&str] = &["COLUMN", "CONSTRAINT", "INDEX", "PRIMARY KEY", "FOREIGN KEY", "UNIQUE", "CHECK"];
const OBJECT_KW: &[&str] = &[
    "TABLE", "VIEW", "INDEX", "UNIQUE INDEX", "FUNCTION", "PROCEDURE", "TRIGGER", "SCHEMA", "DATABASE",
    "SEQUENCE", "TYPE", "EXTENSION", "MATERIALIZED VIEW", "ROLE", "USER", "DOMAIN", "EVENT", "TEMPORARY TABLE",
    "OR REPLACE", "VIRTUAL TABLE", "IF EXISTS", "IF NOT EXISTS",
];
const MYSQL_SHOW_KW: &[&str] = &[
    "TABLES", "FULL TABLES", "DATABASES", "SCHEMAS", "COLUMNS FROM", "FULL COLUMNS FROM", "INDEX FROM",
    "CREATE TABLE", "CREATE VIEW", "CREATE DATABASE", "CREATE PROCEDURE", "CREATE FUNCTION", "CREATE TRIGGER",
    "TABLE STATUS", "PROCESSLIST", "FULL PROCESSLIST", "VARIABLES", "GLOBAL VARIABLES", "SESSION VARIABLES",
    "STATUS", "GLOBAL STATUS", "GRANTS", "WARNINGS", "ERRORS", "ENGINES", "TRIGGERS", "EVENTS", "PLUGINS",
    "PRIVILEGES", "PROCEDURE STATUS", "FUNCTION STATUS", "MASTER STATUS", "REPLICA STATUS", "SLAVE STATUS",
    "BINARY LOGS", "CHARACTER SET", "COLLATION", "OPEN TABLES", "PROFILES",
];

fn paren_ctx(st: &Stmt, p: usize) -> Ctx {
    let Some(prev_i) = p.checked_sub(1) else { return Ctx::Keywords(SUBQUERY_KW) };
    let prev = &st.toks[prev_i];
    if prev.is_kw("EXISTS") {
        return Ctx::Keywords(SUBQUERY_KW);
    }
    if prev.is_kw("USING") {
        return Ctx::Columns(Scope::Using);
    }
    if prev.kind == TokenKind::Keyword
        && (matches!(prev.up.as_str(), "FROM" | "AS" | "LATERAL" | "UNION" | "INTERSECT" | "EXCEPT")
            || prev.up.ends_with("JOIN"))
    {
        return Ctx::Keywords(SUBQUERY_KW);
    }
    if prev.is_name() {
        let k = st.name_start(prev_i);
        let before = st.up(k.checked_sub(1));
        let named_ref = || st.refs.iter().position(|r| r.tok == k);
        match before {
            "INTO" | "REFERENCES" => return named_ref().map_or(Ctx::None, |r| Ctx::Columns(Scope::Ref(r))),
            "ON" if st.verb() == "CREATE" => return named_ref().map_or(Ctx::None, |r| Ctx::Columns(Scope::Ref(r))),
            _ if is_create_table_paren(st, p) => return Ctx::None,
            _ => {}
        }
    }
    Ctx::Expr
}

/// Whether the `(` at `p` opens a `CREATE TABLE name (...)` column list.
fn is_create_table_paren(st: &Stmt, p: usize) -> bool {
    if st.verb() != "CREATE" || p == 0 || !st.toks[p - 1].is_name() {
        return false;
    }
    let mut k = st.name_start(p - 1);
    while k > 0 && matches!(st.toks[k - 1].up.as_str(), "EXISTS" | "NOT" | "IF") {
        k -= 1;
    }
    k > 0 && st.toks[k - 1].is_kw("TABLE")
}

/// Walks back from the comma at `c` to the clause keyword (or opening paren) that owns the list.
fn comma_anchor(st: &Stmt, c: usize) -> Option<usize> {
    let g = st.toks[c].group;
    let mut j = c;
    while j > 0 {
        j -= 1;
        let t = &st.toks[j];
        if t.group == g {
            if t.kind == TokenKind::Keyword
                && matches!(
                    t.up.as_str(),
                    "SELECT" | "DISTINCT" | "FROM" | "BY" | "SET" | "VALUES" | "RETURNING" | "TABLE" | "USING" | "WITH"
                        | "UPDATE" | "INTO" | "WHERE" | "HAVING" | "ON" | "GRANT" | "REVOKE"
                )
            {
                return Some(j);
            }
        } else if j == g {
            return Some(j);
        }
    }
    None
}

fn comma_ctx(st: &Stmt, c: usize, backend: Backend) -> Ctx {
    let Some(a) = comma_anchor(st, c) else { return Ctx::AllKeywords };
    let t = &st.toks[a];
    if t.kind == TokenKind::LParen {
        return paren_ctx(st, a);
    }
    match t.up.as_str() {
        "WITH" => Ctx::None,
        "SELECT" | "DISTINCT" => Ctx::Expr,
        _ => match keyword_ctx(st, a, backend) {
            Ctx::Relations { alias, .. } => Ctx::Relations { join: false, alias },
            other => other,
        },
    }
}

fn keyword_ctx(st: &Stmt, i: usize, backend: Backend) -> Ctx {
    let w = st.toks[i].up.as_str();
    let prev = st.up(i.checked_sub(1));
    let verb = st.verb();
    let first = i == 0;
    let rel = Ctx::Relations { join: false, alias: false };
    match w {
        "ALL" | "DISTINCT" if matches!(prev, "UNION" | "INTERSECT" | "EXCEPT" | "MINUS") => Ctx::Keywords(&["SELECT"]),
        "OR" if prev == "CREATE" => Ctx::Keywords(&["REPLACE"]),
        "REPLACE" if prev == "OR" => Ctx::Keywords(OBJECT_KW),
        "REPLACE" if first => Ctx::Keywords(&["INTO"]),
        "SELECT" | "WHERE" | "HAVING" | "AND" | "OR" | "NOT" | "DISTINCT" | "WHEN" | "THEN" | "ELSE" | "CASE"
        | "BETWEEN" | "LIKE" | "ILIKE" | "RLIKE" | "REGEXP" | "GLOB" | "RETURNING" | "IN" | "ANY" | "SOME" | "ALL"
        | "XOR" | "DIV" | "MOD" | "ELSEIF" => Ctx::Expr,
        "BY" if matches!(prev, "ORDER" | "GROUP" | "PARTITION") => Ctx::Expr,
        "IS" => Ctx::Keywords(IS_KW),
        "ON" => {
            if matches!(verb, "CREATE" | "GRANT" | "REVOKE" | "DROP" | "COMMENT") {
                rel
            } else if (0..i).rev().any(|j| st.toks[j].group == st.toks[i].group && st.toks[j].up.ends_with("JOIN")) {
                Ctx::JoinOn
            } else {
                Ctx::Expr
            }
        }
        "USING" if verb == "DELETE" => rel,
        "FROM" => from_ctx(st, i),
        "UPDATE" if first => rel,
        "UPDATE" if prev == "DO" => Ctx::Keywords(&["SET"]),
        "INTO" | "REFERENCES" | "TRUNCATE" | "DESCRIBE" | "VACUUM" => rel,
        "DESC" | "COPY" | "LOCK" if first => rel,
        "ANALYZE" if prev == "EXPLAIN" => Ctx::Start,
        "ANALYZE" if first => rel,
        "TABLE" | "VIEW" => {
            if verb == "CREATE" {
                Ctx::None
            } else {
                rel
            }
        }
        "EXISTS" => match (prev, st.up(i.checked_sub(2))) {
            ("IF", _) => keyword_or_none(st, i.checked_sub(2), backend),
            ("NOT", "IF") => keyword_or_none(st, i.checked_sub(3), backend),
            _ => Ctx::None,
        },
        "IF" => Ctx::Keywords(&["EXISTS", "NOT EXISTS"]),
        "COLUMN" => {
            if matches!(prev, "ADD") {
                Ctx::None
            } else {
                Ctx::Columns(Scope::Visible)
            }
        }
        "DROP" | "ALTER" | "RENAME" | "MODIFY" | "CHANGE" if !first && verb == "ALTER" => {
            Ctx::ColumnsThenKeywords(ALTER_SUB_KW)
        }
        "ADD" if verb == "ALTER" => Ctx::Keywords(ADD_KW),
        "CREATE" | "ALTER" | "DROP" => Ctx::Keywords(OBJECT_KW),
        "USE" | "DATABASE" => {
            if verb == "CREATE" {
                Ctx::None
            } else {
                Ctx::Databases
            }
        }
        "SCHEMA" => {
            if verb == "CREATE" {
                Ctx::None
            } else {
                Ctx::Schemas
            }
        }
        "FUNCTION" | "PROCEDURE" => {
            if verb == "CREATE" {
                Ctx::None
            } else {
                Ctx::Functions
            }
        }
        "CALL" => Ctx::Functions,
        "TO" => {
            if verb == "GRANT" || st.toks[..i].iter().any(|t| t.is_kw("OWNER")) {
                Ctx::Users
            } else if verb == "SET" && st.toks[..i].iter().any(|t| t.up == "SEARCH_PATH") {
                Ctx::Schemas
            } else {
                Ctx::None
            }
        }
        "OWNER" => Ctx::Keywords(&["TO"]),
        "TYPE" => {
            if prev == "CREATE" {
                Ctx::None
            } else {
                Ctx::DataTypes
            }
        }
        "AS" => as_ctx(st, i),
        "ORDER" | "GROUP" | "PARTITION" => Ctx::Keywords(&["BY"]),
        "INSERT" => Ctx::Keywords(&["INTO"]),
        "DELETE" if first => Ctx::Keywords(&["FROM"]),
        "LEFT" | "RIGHT" | "FULL" => Ctx::Keywords(&["JOIN", "OUTER JOIN"]),
        "INNER" | "CROSS" | "OUTER" => Ctx::Keywords(&["JOIN"]),
        "NATURAL" => Ctx::Keywords(&["JOIN", "LEFT JOIN", "RIGHT JOIN", "FULL JOIN", "INNER JOIN"]),
        "UNION" | "INTERSECT" | "EXCEPT" | "MINUS" => Ctx::Keywords(&["SELECT", "ALL", "DISTINCT"]),
        "WITH" if first => Ctx::Keywords(&["RECURSIVE"]),
        "EXPLAIN" => Ctx::Start,
        "SHOW" if backend == Backend::MySql => Ctx::Keywords(MYSQL_SHOW_KW),
        "BEGIN" | "START" if first => Ctx::Keywords(&["TRANSACTION", "WORK"]),
        "VALUES" | "LIMIT" | "OFFSET" | "RECURSIVE" | "INDEX" | "TRIGGER" | "SEQUENCE" => Ctx::None,
        "SET" => {
            if first {
                Ctx::AllKeywords
            } else if st.toks[..i].iter().any(|t| t.is_kw("UPDATE")) {
                Ctx::Columns(Scope::Visible)
            } else {
                Ctx::AllKeywords
            }
        }
        _ if w.ends_with("JOIN") => Ctx::Relations { join: true, alias: true },
        _ => ident_ctx(st, i),
    }
}

fn keyword_or_none(st: &Stmt, i: Option<usize>, backend: Backend) -> Ctx {
    match i {
        Some(i) if st.toks[i].kind == TokenKind::Keyword => keyword_ctx(st, i, backend),
        _ => Ctx::None,
    }
}

fn from_ctx(st: &Stmt, i: usize) -> Ctx {
    let verb = st.verb();
    if verb == "REVOKE" {
        return Ctx::Users;
    }
    if verb == "SHOW" {
        return if st.toks[..i].iter().any(|t| t.up == "TABLES") {
            Ctx::Databases
        } else {
            Ctx::Relations { join: false, alias: false }
        };
    }
    let g = st.toks[i].group;
    if g != ROOT && g > 0 && matches!(st.toks[g - 1].up.as_str(), "EXTRACT" | "SUBSTRING" | "SUBSTR" | "TRIM" | "OVERLAY") {
        return Ctx::Expr;
    }
    if st.up(i.checked_sub(1)) == "DISTINCT" {
        return Ctx::Expr;
    }
    Ctx::Relations { join: false, alias: verb != "DELETE" }
}

fn as_ctx(st: &Stmt, i: usize) -> Ctx {
    let g = st.toks[i].group;
    if g != ROOT && g > 0 && matches!(st.toks[g - 1].up.as_str(), "CAST" | "TRY_CAST") {
        return Ctx::DataTypes;
    }
    if st.verb() == "CREATE" && g == ROOT {
        return Ctx::Keywords(&["SELECT", "WITH", "VALUES", "TABLE"]);
    }
    Ctx::None
}

fn ident_ctx(st: &Stmt, i: usize) -> Ctx {
    if !st.toks[i].is_name() {
        return following(st, i);
    }
    let g = st.toks[i].group;
    if g != ROOT && is_create_table_paren(st, g) && i > 0 {
        let p = &st.toks[i - 1];
        if i - 1 == g || (p.kind == TokenKind::Comma && p.group == g) {
            return Ctx::DataTypes;
        }
    }
    if st.verb() == "ALTER" {
        let p = st.up(i.checked_sub(1));
        let pp = st.up(i.checked_sub(2));
        let ppp = st.up(i.checked_sub(3));
        let prev_is_name = i > 0 && st.toks[i - 1].is_name();
        if matches!(p, "ADD" | "MODIFY")
            || (p == "COLUMN" && matches!(pp, "ADD" | "MODIFY"))
            || (prev_is_name && (pp == "CHANGE" || (pp == "COLUMN" && ppp == "CHANGE")))
        {
            return Ctx::DataTypes;
        }
    }
    following(st, i)
}

const AFTER_SELECT_ITEM: &[&str] = &["FROM", "AS", "INTO"];
const AFTER_TABLE: &[&str] = &[
    "WHERE", "JOIN", "LEFT JOIN", "INNER JOIN", "ON", "USING", "GROUP BY", "ORDER BY", "LIMIT", "AS", "RIGHT JOIN",
    "FULL OUTER JOIN", "CROSS JOIN", "HAVING", "OFFSET", "UNION", "SET",
];
const AFTER_INTO: &[&str] = &["VALUES", "SELECT", "DEFAULT VALUES", "ON CONFLICT", "RETURNING"];
const AFTER_ALTER_TABLE: &[&str] = &[
    "ADD", "DROP", "ALTER", "RENAME", "ADD COLUMN", "DROP COLUMN", "ALTER COLUMN", "RENAME COLUMN", "RENAME TO",
    "OWNER TO", "SET", "MODIFY", "CHANGE",
];
const AFTER_CONDITION: &[&str] = &[
    "AND", "OR", "ORDER BY", "GROUP BY", "LIMIT", "NOT", "IS", "IN", "LIKE", "ILIKE", "BETWEEN", "HAVING", "OFFSET",
    "UNION", "RETURNING",
];
const AFTER_ORDER_ITEM: &[&str] = &["ASC", "DESC", "LIMIT", "NULLS", "OFFSET", "HAVING", "ORDER BY", "UNION"];
const AFTER_ASSIGNMENT: &[&str] = &["WHERE", "FROM", "RETURNING"];

/// Keywords likely to follow a complete term, by the clause the term is in.
fn following(st: &Stmt, i: usize) -> Ctx {
    let g = st.toks[i].group;
    let clause = (0..=i).rev().map(|j| &st.toks[j]).filter(|t| t.group == g && t.kind == TokenKind::Keyword).find(|t| {
        matches!(t.up.as_str(), "SELECT" | "FROM" | "WHERE" | "HAVING" | "ON" | "BY" | "SET" | "UPDATE" | "INTO" | "VALUES" | "TABLE")
            || t.up.ends_with("JOIN")
    });
    match clause.map(|t| t.up.as_str()) {
        Some("SELECT") => Ctx::Following(AFTER_SELECT_ITEM),
        Some("INTO") => Ctx::Following(AFTER_INTO),
        Some("TABLE") if st.verb() == "ALTER" => Ctx::Following(AFTER_ALTER_TABLE),
        Some("FROM" | "UPDATE") => Ctx::Following(AFTER_TABLE),
        Some(w) if w.ends_with("JOIN") => Ctx::Following(AFTER_TABLE),
        Some("WHERE" | "HAVING" | "ON") => Ctx::Following(AFTER_CONDITION),
        Some("BY") => Ctx::Following(AFTER_ORDER_ITEM),
        Some("SET") => Ctx::Following(AFTER_ASSIGNMENT),
        _ => Ctx::AllKeywords,
    }
}
