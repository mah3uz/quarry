use std::borrow::Cow;

use crate::db::Catalog;
use crate::sql::lexer::TokenKind;

use super::context::{ROOT, Stmt, Tok};

/// A table reference in a FROM/JOIN/UPDATE/INTO/… clause.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TableRef {
    pub schema: Option<String>,
    pub name: String,
    pub alias: Option<String>,
    /// Paren group the reference lives in.
    pub group: usize,
    /// Token index of the reference's first token.
    pub tok: usize,
    /// Derived table `( … ) alias`: token indices of its parens.
    pub body: Option<(usize, usize)>,
    /// Table function (`generate_series(…) g`): columns unknown.
    pub func: bool,
}

impl TableRef {
    pub fn refname(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cte {
    pub name: String,
    pub columns: Option<Vec<String>>,
    pub body: Option<(usize, usize)>,
}

pub(crate) struct Col<'c> {
    pub name: Cow<'c, str>,
    pub data_type: Option<&'c str>,
}

fn at<'s, 'a>(st: &'s Stmt<'a>, i: usize) -> Option<&'s Tok<'a>> {
    st.toks.get(i)
}

fn is_kw(st: &Stmt, i: usize, w: &str) -> bool {
    at(st, i).is_some_and(|t| t.is_kw(w))
}

fn is_function_paren(st: &Stmt, g: usize) -> bool {
    g != ROOT && g > 0 && matches!(st.toks[g - 1].up.as_str(), "EXTRACT" | "SUBSTRING" | "SUBSTR" | "TRIM" | "OVERLAY")
}

pub(crate) fn extract_refs(st: &Stmt) -> Vec<TableRef> {
    let mut refs = Vec::new();
    let mut i = 0;
    let verb = st.verb().to_string();
    while i < st.toks.len() {
        let t = &st.toks[i];
        let w = t.up.as_str();
        let list = match w {
            _ if t.kind != TokenKind::Keyword => None,
            "FROM" if !is_function_paren(st, t.group) && st.up_at(i.wrapping_sub(1)) != "DISTINCT" => Some(true),
            "USING" if at(st, i + 1).is_some_and(|n| n.kind != TokenKind::LParen) => Some(true),
            "UPDATE" | "TABLE" | "TRUNCATE" => Some(true),
            "INTO" | "REFERENCES" | "DESCRIBE" => Some(false),
            "DESC" if i == 0 => Some(false),
            "ON" if verb == "CREATE" => Some(false),
            _ if w.ends_with("JOIN") => Some(true),
            _ => None,
        };
        i = match list {
            Some(list) => parse_list(st, i + 1, list, w.ends_with("JOIN") || w == "FROM" || w == "USING", &mut refs),
            None => i + 1,
        };
    }
    refs
}

fn parse_list(st: &Stmt, mut j: usize, list: bool, from_like: bool, refs: &mut Vec<TableRef>) -> usize {
    loop {
        while at(st, j).is_some_and(|t| {
            t.kind == TokenKind::Keyword && matches!(t.up.as_str(), "ONLY" | "LATERAL" | "IF" | "NOT" | "EXISTS" | "TABLE")
        }) {
            j += 1;
        }
        let Some(t) = at(st, j) else { break };
        let start = j;
        let group = t.group;
        if t.kind == TokenKind::LParen && from_like {
            let close = st.close[j];
            let body = (j, close.unwrap_or(st.toks.len()));
            j = close.map_or(st.toks.len(), |c| c + 1);
            let (alias, next) = parse_alias(st, j, true);
            j = next;
            if let Some(alias) = alias {
                refs.push(TableRef { schema: None, name: alias, alias: None, group, tok: start, body: Some(body), func: false });
            }
        } else if t.is_name() {
            let mut parts = vec![t.name()];
            j += 1;
            while at(st, j).is_some_and(|d| d.kind == TokenKind::Dot) && at(st, j + 1).is_some_and(Tok::is_name) {
                parts.push(st.toks[j + 1].name());
                j += 2;
            }
            let mut func = false;
            if from_like && at(st, j).is_some_and(|p| p.kind == TokenKind::LParen) {
                func = true;
                j = st.close[j].map_or(st.toks.len(), |c| c + 1);
            }
            let (alias, next) = parse_alias(st, j, from_like || list);
            j = next;
            let name = parts.pop().unwrap_or_default();
            let schema = parts.pop();
            refs.push(TableRef { schema, name, alias, group, tok: start, body: None, func });
        } else {
            break;
        }
        if list && at(st, j).is_some_and(|c| c.kind == TokenKind::Comma) {
            j += 1;
        } else {
            break;
        }
    }
    j
}

/// `[AS] alias [(col, …)]`; a bare alias only when `bare` is allowed.
fn parse_alias(st: &Stmt, j: usize, bare: bool) -> (Option<String>, usize) {
    let (alias, mut next) = if is_kw(st, j, "AS") {
        match at(st, j + 1) {
            Some(n) if n.is_name() => (Some(n.name()), j + 2),
            _ => (None, j + 1),
        }
    } else if bare && at(st, j).is_some_and(|t| t.is_name() && t.kind != TokenKind::Builtin) {
        (Some(st.toks[j].name()), j + 1)
    } else {
        (None, j)
    };
    if alias.is_some() && at(st, next).is_some_and(|p| p.kind == TokenKind::LParen) {
        next = st.close[next].map_or(st.toks.len(), |c| c + 1);
    }
    (alias, next)
}

pub(crate) fn extract_ctes(st: &Stmt) -> Vec<Cte> {
    let mut ctes = Vec::new();
    for i in 0..st.toks.len() {
        if !st.toks[i].is_kw("WITH") {
            continue;
        }
        let mut j = i + 1;
        if is_kw(st, j, "RECURSIVE") {
            j += 1;
        }
        while let Some(n) = at(st, j).filter(|n| n.is_name()) {
            let name = n.name();
            j += 1;
            let mut columns = None;
            if at(st, j).is_some_and(|p| p.kind == TokenKind::LParen) {
                let close = st.close[j].unwrap_or(st.toks.len());
                columns = Some(st.toks[j + 1..close].iter().filter(|t| t.is_name()).map(Tok::name).collect());
                j = close + 1;
            }
            if !is_kw(st, j, "AS") {
                break;
            }
            j += 1;
            while is_kw(st, j, "NOT") || at(st, j).is_some_and(|t| t.up == "MATERIALIZED") {
                j += 1;
            }
            if !at(st, j).is_some_and(|p| p.kind == TokenKind::LParen) {
                break;
            }
            let close = st.close[j];
            ctes.push(Cte { name, columns, body: Some((j, close.unwrap_or(st.toks.len()))) });
            j = close.map_or(st.toks.len(), |c| c + 1);
            if at(st, j).is_some_and(|c| c.kind == TokenKind::Comma) {
                j += 1;
            } else {
                break;
            }
        }
    }
    ctes
}

impl Stmt<'_> {
    pub(crate) fn up_at(&self, i: usize) -> &str {
        self.toks.get(i).map(|t| t.up.as_str()).unwrap_or("")
    }

    pub(crate) fn find_cte(&self, name: &str) -> Option<&Cte> {
        self.ctes.iter().find(|c| c.name == name).or_else(|| self.ctes.iter().find(|c| c.name.eq_ignore_ascii_case(name)))
    }

    /// Columns of a table reference: derived-table/CTE select lists first, then the catalog.
    pub(crate) fn ref_columns<'c>(&self, r: &TableRef, cat: &'c Catalog, depth: usize) -> Vec<Col<'c>> {
        if r.func || depth > 4 {
            return Vec::new();
        }
        if let Some((open, close)) = r.body {
            return self.select_list_columns(open, close, cat, depth + 1);
        }
        if r.schema.is_none()
            && let Some(cte) = self.find_cte(&r.name)
        {
            if let Some(cols) = &cte.columns {
                return cols.iter().map(|c| Col { name: Cow::Owned(c.clone()), data_type: None }).collect();
            }
            if let Some((open, close)) = cte.body {
                return self.select_list_columns(open, close, cat, depth + 1);
            }
        }
        cat.find_relation(r.schema.as_deref(), &r.name)
            .map(|rel| {
                rel.columns.iter().map(|c| Col { name: Cow::Borrowed(c.name.as_str()), data_type: Some(c.data_type.as_str()) }).collect()
            })
            .unwrap_or_default()
    }

    /// Output column names of the query inside the parens `open..close`.
    fn select_list_columns<'c>(&self, open: usize, close: usize, cat: &'c Catalog, depth: usize) -> Vec<Col<'c>> {
        let toks = &self.toks;
        let end = close.min(toks.len());
        let direct = |k: usize| toks[k].group == open;
        let Some(sel) = (open + 1..end).find(|&k| direct(k) && toks[k].is_kw("SELECT")) else {
            return Vec::new();
        };
        let mut k = sel + 1;
        while k < end && direct(k) && matches!(toks[k].up.as_str(), "DISTINCT" | "ALL") {
            k += 1;
            if k < end && toks[k].is_kw("ON") && toks.get(k + 1).is_some_and(|p| p.kind == TokenKind::LParen) {
                k = self.close[k + 1].map_or(end, |c| c + 1);
            }
        }
        let mut items: Vec<(usize, usize)> = Vec::new();
        let mut item_start = k;
        while k < end {
            let t = &toks[k];
            if direct(k) && t.kind == TokenKind::Keyword && matches!(t.up.as_str(), "FROM" | "INTO" | "WHERE" | "GROUP" | "ORDER" | "LIMIT" | "UNION" | "HAVING" | "WINDOW") {
                break;
            }
            if direct(k) && t.kind == TokenKind::Comma {
                items.push((item_start, k));
                item_start = k + 1;
            }
            k += 1;
        }
        items.push((item_start, k));

        let mut out = Vec::new();
        for (a, b) in items {
            if b <= a {
                continue;
            }
            let last = &toks[b - 1];
            let prev = (b - 1 > a).then(|| &toks[b - 2]);
            if last.kind == TokenKind::Operator && last.text == "*" {
                let qual = prev.filter(|p| p.kind == TokenKind::Dot).and_then(|_| (b >= a + 3).then(|| toks[b - 3].name()));
                for r in self.refs.iter().filter(|r| r.group == open) {
                    if qual.as_deref().is_none_or(|q| r.refname().eq_ignore_ascii_case(q)) {
                        out.extend(self.ref_columns(r, cat, depth));
                    }
                }
                continue;
            }
            if !last.is_name() {
                continue;
            }
            let named = match prev {
                None => true,
                Some(p) => {
                    p.is_kw("AS")
                        || p.kind == TokenKind::Dot
                        || matches!(p.kind, TokenKind::RParen | TokenKind::Number | TokenKind::String | TokenKind::QuotedIdent)
                        || (p.is_name() && p.kind != TokenKind::Builtin)
                }
            };
            if named {
                out.push(Col { name: Cow::Owned(last.name()), data_type: None });
            }
        }
        out
    }
}
