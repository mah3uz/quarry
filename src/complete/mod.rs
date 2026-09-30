mod context;
mod index;
mod rank;
mod scope;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::db::catalog::{FunctionInfo, Relation};
use crate::db::{Backend, Catalog, quote_ident};

use context::{Analysis, Ctx, Scope, Stmt};
use index::Index;
use scope::TableRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SuggestionKind {
    Keyword,
    Table,
    View,
    Column,
    Schema,
    Database,
    Function,
    DataType,
    Alias,
    Join,
    JoinCondition,
    Special,
    Favorite,
    File,
    User,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// Text inserted in place of `replace_start..cursor`.
    pub text: String,
    /// Label shown in the menu (usually == text).
    pub display: String,
    pub kind: SuggestionKind,
    /// Right-hand detail: column type, function signature, table kind, command help…
    pub detail: Option<String>,
    /// Higher is better; items are returned sorted.
    pub score: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Completions {
    /// Byte offset in the input where the word being completed starts.
    pub replace_start: usize,
    pub items: Vec<Suggestion>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeywordCasing {
    Upper,
    Lower,
    /// Match the case the user typed.
    #[default]
    Auto,
}

#[derive(Clone, Debug)]
pub struct CompleteOptions {
    pub keyword_casing: KeywordCasing,
    /// false → keywords + every name, no context analysis (pgcli `smart_completion = False`).
    pub smart: bool,
    /// Suggest `JOIN b ON a.id = b.a_id` style completions from foreign keys.
    pub join_suggestions: bool,
    pub generate_aliases: bool,
    pub max_items: usize,
}

impl Default for CompleteOptions {
    fn default() -> Self {
        CompleteOptions { keyword_casing: KeywordCasing::Auto, smart: true, join_suggestions: true, generate_aliases: false, max_items: 200 }
    }
}

/// Extra, non-catalog candidates (special commands, favorite queries).
#[derive(Clone, Debug, Default)]
pub struct Extras {
    /// (command, description) e.g. ("\\dt", "List tables").
    pub specials: Vec<(String, String)>,
    pub favorites: Vec<String>,
    /// Output format names offered after `\T`, `\tableformat`, `.mode`.
    pub table_formats: Vec<String>,
    /// Theme names offered after `\theme`.
    pub themes: Vec<String>,
}

pub struct Completer {
    pub backend: Backend,
    pub catalog: Arc<Catalog>,
    pub options: CompleteOptions,
    pub extras: Extras,
    /// Lower-cased name index, rebuilt whenever `catalog` points at a different snapshot.
    index: Mutex<Option<Arc<Index>>>,
}

impl Completer {
    pub fn new(backend: Backend, catalog: Arc<Catalog>, options: CompleteOptions, extras: Extras) -> Self {
        Completer { backend, catalog, options, extras, index: Mutex::new(None) }
    }

    /// Context-aware completion for `text` with the cursor at byte offset `cursor`.
    /// `text` may contain several statements; only the one under the cursor matters.
    pub fn complete(&self, text: &str, cursor: usize) -> Completions {
        let mut cursor = cursor.min(text.len());
        while !text.is_char_boundary(cursor) {
            cursor -= 1;
        }
        let ix = self.index();
        if !self.options.smart {
            return self.complete_plain(text, cursor, &ix);
        }
        let Some(a) = context::analyze(text, cursor, self.backend) else {
            return Completions { replace_start: cursor, items: Vec::new() };
        };
        let pat = match a.ctx {
            Ctx::Files => a.word.prefix.rsplit('/').next().unwrap_or("").to_lowercase(),
            _ => a.word.prefix.to_lowercase(),
        };
        let mut run = Run::new(self, &ix, &a.stmt, pat, &a.word.prefix);
        run.quoted = a.word.quoted;
        run.gather(&a);
        Completions { replace_start: a.word.start, items: run.finish() }
    }

    fn index(&self) -> Arc<Index> {
        let mut guard = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ix) = guard.as_ref().filter(|ix| Arc::ptr_eq(&ix.catalog, &self.catalog) && ix.backend == self.backend) {
            return ix.clone();
        }
        let ix = Arc::new(Index::build(self.catalog.clone(), self.backend));
        *guard = Some(ix.clone());
        ix
    }

    /// `smart = false`: keywords and every catalog name matching the word before the cursor.
    fn complete_plain(&self, text: &str, cursor: usize, ix: &Index) -> Completions {
        let start = text[..cursor]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '$')
            .last()
            .map_or(cursor, |(i, _)| i);
        let prefix = &text[start..cursor];
        let empty = Stmt::empty();
        let mut run = Run::new(self, ix, &empty, prefix.to_lowercase(), prefix);
        run.all_keywords();
        run.datatypes(None);
        run.builtins();
        let cat = &*ix.catalog;
        for e in &ix.rels {
            let rel = &cat.schemas[e.schema].relations[e.idx];
            run.add(Cow::Borrowed(e.key.as_str()), rel_kind(rel), Item::Ident { name: &rel.name, detail: Some(rel.kind.label()) });
        }
        for e in &ix.funcs {
            run.add(Cow::Borrowed(e.key.as_str()), SuggestionKind::Function, Item::Func(&cat.schemas[e.schema].functions[e.idx]));
        }
        for (si, ri, ci, key) in ix.columns() {
            let col = &cat.schemas[*si].relations[*ri].columns[*ci];
            run.add(Cow::Borrowed(key.as_str()), SuggestionKind::Column, Item::Ident { name: &col.name, detail: None });
        }
        run.schemas();
        run.databases();
        Completions { replace_start: start, items: run.finish() }
    }
}

fn rel_kind(r: &Relation) -> SuggestionKind {
    if r.kind.is_view() { SuggestionKind::View } else { SuggestionKind::Table }
}

enum Item<'a> {
    Ident { name: &'a str, detail: Option<&'a str> },
    Owned { text: String, display: String, detail: Option<String> },
    Keyword(&'a str),
    Builtin(&'a str),
    Func(&'a FunctionInfo),
    Rel { rel: &'a Relation, alias: bool },
}

struct Cand<'a> {
    key: Cow<'a, str>,
    kind: SuggestionKind,
    item: Item<'a>,
}

/// One completion request: collects scored candidates, then renders the best.
struct Run<'a> {
    c: &'a Completer,
    ix: &'a Index,
    cat: &'a Catalog,
    st: &'a Stmt<'a>,
    pat: String,
    upper: bool,
    /// Completing a quoted identifier: keywords and builtins cannot match.
    quoted: bool,
    group: u8,
    scored: Vec<(i64, usize, Cand<'a>)>,
}

fn owned(text: String, detail: Option<String>) -> Item<'static> {
    Item::Owned { display: text.clone(), text, detail }
}

impl<'a> Run<'a> {
    fn new(c: &'a Completer, ix: &'a Index, st: &'a Stmt<'a>, pat: String, typed: &str) -> Self {
        let upper = match c.options.keyword_casing {
            KeywordCasing::Upper => true,
            KeywordCasing::Lower => false,
            KeywordCasing::Auto => typed.chars().rev().find(|c| c.is_alphabetic()).is_none_or(|c| !c.is_lowercase()),
        };
        Run { c, ix, cat: &ix.catalog, st, pat, upper, quoted: false, group: 0, scored: Vec::new() }
    }

    fn q(&self, name: &str) -> String {
        quote_ident(name, self.c.backend)
    }

    /// Following candidates rank below everything added so far (at equal match quality).
    fn next(&mut self) {
        self.group = self.group.saturating_add(1);
    }

    fn add(&mut self, key: Cow<'a, str>, kind: SuggestionKind, item: Item<'a>) {
        let q = rank::quality(&key, &self.pat);
        if q > 0 {
            let s = rank::score(q, self.group, key.len());
            let seq = self.scored.len();
            self.scored.push((s, seq, Cand { key, kind, item }));
        }
    }

    fn add_owned(&mut self, key: &str, kind: SuggestionKind, text: String, detail: Option<String>) {
        self.add(Cow::Owned(key.to_lowercase()), kind, owned(text, detail));
    }

    fn gather(&mut self, a: &Analysis) {
        let q = &a.word.qualifier;
        if !q.is_empty() && a.ctx != Ctx::Star {
            self.qualified(&a.ctx, q);
            return;
        }
        match &a.ctx {
            Ctx::None => {}
            Ctx::Start => self.start(),
            Ctx::Specials => {
                let lead = a.word.prefix.chars().next();
                if matches!(lead, Some('\\' | '.')) {
                    self.specials(|name| name.starts_with(lead.unwrap_or('\\')));
                } else {
                    self.start();
                }
            }
            Ctx::Keywords(list) => self.keyword_list(list),
            Ctx::AllKeywords => self.all_keywords(),
            Ctx::Following(list) => {
                self.keyword_list(list);
                self.next();
                self.all_keywords();
            }
            Ctx::Expr => {
                self.columns(Scope::Visible);
                self.next();
                self.aliases();
                self.next();
                self.functions(None);
                self.builtins();
                self.next();
                self.keyword_list(EXPR_KEYWORDS);
                self.next();
                self.all_keywords();
            }
            Ctx::Columns(scope) => self.columns(*scope),
            Ctx::ColumnsThenKeywords(list) => {
                self.columns(Scope::Visible);
                self.next();
                self.keyword_list(list);
            }
            Ctx::Relations { join, alias } => {
                if *join && self.c.options.join_suggestions {
                    self.joins();
                    self.next();
                }
                self.ctes();
                self.next();
                self.relations(None, *alias);
                self.next();
                self.schemas();
            }
            Ctx::JoinOn => {
                self.join_conditions();
                self.next();
                self.columns(Scope::Visible);
                self.next();
                self.aliases();
            }
            Ctx::DataTypes => self.datatypes(None),
            Ctx::Databases => self.databases(),
            Ctx::Schemas => self.schemas(),
            Ctx::Users => {
                for u in &self.cat.users {
                    self.add(Cow::Owned(u.to_lowercase()), SuggestionKind::User, Item::Ident { name: u, detail: None });
                }
            }
            Ctx::Functions => {
                self.functions(None);
                self.next();
                self.schemas();
            }
            Ctx::Favorites => self.plain(&self.c.extras.favorites, SuggestionKind::Favorite, "favorite"),
            Ctx::TableFormats => self.plain(&self.c.extras.table_formats, SuggestionKind::Keyword, "format"),
            Ctx::Themes => self.plain(&self.c.extras.themes, SuggestionKind::Keyword, "theme"),
            Ctx::Files => self.files(&a.word.prefix),
            Ctx::Star => self.star(q),
        }
    }

    fn plain(&mut self, list: &'a [String], kind: SuggestionKind, detail: &str) {
        for s in list {
            self.add(Cow::Owned(s.to_lowercase()), kind, owned(s.clone(), Some(detail.to_string())));
        }
    }

    fn qualified(&mut self, ctx: &Ctx, q: &[String]) {
        if let [schema, table] = q {
            if let Some(rel) = self.cat.find_relation(Some(schema), table) {
                self.relation_columns(rel);
            }
            return;
        }
        let q = &q[0];
        let schema = self.ix.schema_index(q);
        match ctx {
            Ctx::None | Ctx::Databases | Ctx::Schemas | Ctx::Users | Ctx::Favorites | Ctx::TableFormats | Ctx::Themes
            | Ctx::Files | Ctx::Start | Ctx::Specials | Ctx::Star => {}
            Ctx::Relations { alias, .. } => {
                if schema.is_some() {
                    self.relations(schema, *alias);
                }
            }
            Ctx::Functions => {
                if schema.is_some() {
                    self.functions(schema);
                }
            }
            Ctx::DataTypes => {
                if schema.is_some() {
                    self.user_types(schema);
                }
            }
            _ => {
                if let Some(ri) = self.st.find_ref(q) {
                    self.columns(Scope::Ref(ri));
                } else if schema.is_some() {
                    self.relations(schema, false);
                    self.next();
                    self.functions(schema);
                } else if let Some(rel) = self.cat.find_relation(None, q) {
                    self.relation_columns(rel);
                }
            }
        }
    }

    fn relation_columns(&mut self, rel: &'a Relation) {
        for c in &rel.columns {
            self.add(Cow::Owned(c.name.to_lowercase()), SuggestionKind::Column, Item::Ident { name: &c.name, detail: Some(&c.data_type) });
        }
    }

    fn start(&mut self) {
        self.keyword_list(start_keywords(self.c.backend));
        self.next();
        self.specials(|name| !name.starts_with(['\\', '.']));
    }

    fn specials(&mut self, keep: impl Fn(&str) -> bool) {
        for (name, desc) in &self.c.extras.specials {
            if keep(name) {
                self.add(Cow::Owned(name.to_lowercase()), SuggestionKind::Special, owned(name.clone(), Some(desc.clone())));
            }
        }
    }

    /// Curated lists rank in list order.
    fn keyword_list(&mut self, list: &'static [&'static str]) {
        if self.quoted {
            return;
        }
        for k in list {
            self.add(Cow::Owned(k.to_lowercase()), SuggestionKind::Keyword, Item::Keyword(k));
            self.group = self.group.saturating_add(1);
        }
    }

    fn all_keywords(&mut self) {
        if self.quoted {
            return;
        }
        let ix = self.ix;
        for (k, key) in &ix.keywords {
            self.add(Cow::Borrowed(key.as_str()), SuggestionKind::Keyword, Item::Keyword(k));
        }
    }

    fn builtins(&mut self) {
        if self.quoted {
            return;
        }
        let ix = self.ix;
        for (f, key) in &ix.builtins {
            self.add(Cow::Borrowed(key.as_str()), SuggestionKind::Function, Item::Builtin(f));
        }
    }

    fn datatypes(&mut self, schema: Option<usize>) {
        if schema.is_none() && !self.quoted {
            let ix = self.ix;
            for (t, key) in &ix.datatypes {
                self.add(Cow::Borrowed(key.as_str()), SuggestionKind::DataType, Item::Keyword(t));
            }
        }
        self.user_types(schema);
    }

    fn user_types(&mut self, schema: Option<usize>) {
        let (ix, cat) = (self.ix, self.cat);
        for e in ix.types.iter().filter(|e| schema.map_or(e.visible, |s| e.schema == s)) {
            let name = &cat.schemas[e.schema].types[e.idx];
            self.add(Cow::Borrowed(e.key.as_str()), SuggestionKind::DataType, Item::Ident { name, detail: Some("type") });
        }
    }

    fn functions(&mut self, schema: Option<usize>) {
        let (ix, cat) = (self.ix, self.cat);
        for e in ix.funcs.iter().filter(|e| schema.map_or(e.visible, |s| e.schema == s)) {
            self.add(Cow::Borrowed(e.key.as_str()), SuggestionKind::Function, Item::Func(&cat.schemas[e.schema].functions[e.idx]));
        }
    }

    fn relations(&mut self, schema: Option<usize>, alias: bool) {
        let (ix, cat) = (self.ix, self.cat);
        let alias = alias && self.c.options.generate_aliases;
        for e in ix.rels.iter().filter(|e| schema.map_or(e.visible, |s| e.schema == s)) {
            let rel = &cat.schemas[e.schema].relations[e.idx];
            self.add(Cow::Borrowed(e.key.as_str()), rel_kind(rel), Item::Rel { rel, alias });
        }
    }

    fn schemas(&mut self) {
        let (ix, cat) = (self.ix, self.cat);
        for (s, key) in cat.schemas.iter().zip(&ix.schemas) {
            self.add(Cow::Borrowed(key.as_str()), SuggestionKind::Schema, Item::Ident { name: &s.name, detail: Some("schema") });
        }
    }

    fn databases(&mut self) {
        for d in &self.cat.databases {
            self.add(Cow::Owned(d.to_lowercase()), SuggestionKind::Database, Item::Ident { name: d, detail: Some("database") });
        }
    }

    fn ctes(&mut self) {
        let st = self.st;
        for cte in &st.ctes {
            let text = self.q(&cte.name);
            self.add_owned(&cte.name, SuggestionKind::Table, text, Some("cte".into()));
        }
    }

    fn aliases(&mut self) {
        let st = self.st;
        for (_, r) in st.visible_refs() {
            let text = self.q(r.refname());
            self.add_owned(r.refname(), SuggestionKind::Alias, text, Some(r.name.clone()));
        }
    }

    fn columns(&mut self, scope: Scope) {
        let st = self.st;
        let refs: Vec<&TableRef> = match scope {
            Scope::Visible | Scope::Using => st.visible_refs().map(|(_, r)| r).collect(),
            Scope::Ref(i) => st.refs.get(i).into_iter().collect(),
        };
        let per_ref: Vec<(&TableRef, Vec<scope::Col>)> = refs.iter().map(|r| (*r, st.ref_columns(r, self.cat, 0))).collect();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for (_, cols) in &per_ref {
            for c in cols {
                *counts.entry(c.name.to_lowercase()).or_default() += 1;
            }
        }
        if scope == Scope::Using {
            for shared in [true, false] {
                for c in per_ref.iter().flat_map(|(_, cols)| cols) {
                    let key = c.name.to_lowercase();
                    if (counts[&key] > 1) == shared {
                        let text = self.q(&c.name);
                        self.add(Cow::Owned(key), SuggestionKind::Column, owned(text, c.data_type.map(str::to_string)));
                    }
                }
                self.next();
            }
            return;
        }
        let multi = per_ref.len() > 1;
        for (r, cols) in per_ref {
            for c in cols {
                let key = c.name.to_lowercase();
                let col = self.q(&c.name);
                let text = if multi && counts[&key] > 1 { format!("{}.{col}", self.q(r.refname())) } else { col };
                self.add(Cow::Owned(key), SuggestionKind::Column, owned(text, c.data_type.map(str::to_string)));
            }
        }
    }

    fn star(&mut self, q: &[String]) {
        let st = self.st;
        let refs: Vec<&TableRef> = match q.last() {
            Some(q) => st.find_ref(q).map(|i| &st.refs[i]).into_iter().collect(),
            None => st.local_refs().map(|(_, r)| r).collect(),
        };
        let qualify = !q.is_empty() || refs.len() > 1;
        let mut cols = Vec::new();
        for r in &refs {
            for c in st.ref_columns(r, self.cat, 0) {
                let col = self.q(&c.name);
                cols.push(if qualify { format!("{}.{col}", self.q(r.refname())) } else { col });
            }
        }
        if !cols.is_empty() {
            self.add(Cow::Borrowed(""), SuggestionKind::Column, owned(cols.join(", "), Some("expand *".into())));
        }
    }

    /// Catalog relation behind a reference, unless a CTE or derived table shadows it.
    fn catalog_rel(&self, r: &TableRef) -> Option<&'a Relation> {
        if r.body.is_some() || r.func || (r.schema.is_none() && self.st.find_cte(&r.name).is_some()) {
            return None;
        }
        self.cat.find_relation(r.schema.as_deref(), &r.name)
    }

    /// Refs visible at the cursor that appear before it, in statement order.
    fn refs_before(&self) -> Vec<&'a TableRef> {
        let st = self.st;
        let mut v: Vec<&TableRef> = st.visible_refs().map(|(_, r)| r).filter(|r| r.tok < st.cur).collect();
        v.sort_by_key(|r| r.tok);
        v
    }

    fn joins(&mut self) {
        let cat = self.cat;
        let taken: HashSet<String> = self.st.refs.iter().map(|r| r.refname().to_lowercase()).collect();
        for left in self.refs_before() {
            let Some(lrel) = self.catalog_rel(left) else { continue };
            for fk in &cat.foreign_keys {
                let (other_schema, other_table, pairs): (&str, &str, Vec<(&str, &str)>) =
                    if fk.table == lrel.name && fk.schema == lrel.schema {
                        let pairs = fk.ref_columns.iter().zip(&fk.columns).map(|(o, l)| (o.as_str(), l.as_str())).collect();
                        (&fk.ref_schema, &fk.ref_table, pairs)
                    } else if fk.ref_table == lrel.name && fk.ref_schema == lrel.schema {
                        let pairs = fk.columns.iter().zip(&fk.ref_columns).map(|(o, l)| (o.as_str(), l.as_str())).collect();
                        (&fk.schema, &fk.table, pairs)
                    } else {
                        continue;
                    };
                let needs_alias = self.c.options.generate_aliases || taken.contains(&other_table.to_lowercase());
                let table_text = if cat.search_path.is_empty() || cat.is_on_search_path(other_schema) {
                    self.q(other_table)
                } else {
                    format!("{}.{}", self.q(other_schema), self.q(other_table))
                };
                let oref = if needs_alias { self.q(&unique_alias(other_table, &taken)) } else { self.q(other_table) };
                let lref = self.q(left.refname());
                let cond = pairs
                    .iter()
                    .map(|(o, l)| format!("{oref}.{} = {lref}.{}", self.q(o), self.q(l)))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                let text = if needs_alias { format!("{table_text} {oref} ON {cond}") } else { format!("{table_text} ON {cond}") };
                self.add_owned(other_table, SuggestionKind::Join, text, Some("join".into()));
            }
        }
    }

    fn join_conditions(&mut self) {
        let cat = self.cat;
        let before = self.refs_before();
        let Some((right, lefts)) = before.split_last() else { return };
        let Some(rrel) = self.catalog_rel(right) else { return };
        let rref = self.q(right.refname());
        for left in lefts {
            let Some(lrel) = self.catalog_rel(left) else { continue };
            let lref = self.q(left.refname());
            for fk in &cat.foreign_keys {
                let is = |schema: &str, table: &str, rel: &Relation| rel.name == table && rel.schema == schema;
                let pairs: Vec<(&String, &String)> = if is(&fk.schema, &fk.table, rrel) && is(&fk.ref_schema, &fk.ref_table, lrel) {
                    fk.columns.iter().zip(&fk.ref_columns).collect()
                } else if is(&fk.schema, &fk.table, lrel) && is(&fk.ref_schema, &fk.ref_table, rrel) {
                    fk.ref_columns.iter().zip(&fk.columns).collect()
                } else {
                    continue;
                };
                let cond = pairs
                    .iter()
                    .map(|(r, l)| format!("{rref}.{} = {lref}.{}", self.q(r), self.q(l)))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                let key = cond.clone();
                self.add_owned(&key, SuggestionKind::JoinCondition, cond, Some("fk join".into()));
            }
        }
    }

    fn files(&mut self, arg: &str) {
        let (dir_part, file_prefix) = match arg.rfind('/') {
            Some(i) => (&arg[..=i], &arg[i + 1..]),
            None => ("", arg),
        };
        let dir = match dir_part.strip_prefix('~') {
            Some(rest) => match dirs::home_dir() {
                Some(h) => format!("{}{rest}", h.display()),
                None => return,
            },
            None if dir_part.is_empty() => ".".to_string(),
            None => dir_part.to_string(),
        };
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !file_prefix.starts_with('.') {
                continue;
            }
            let is_dir = std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir());
            let text = format!("{dir_part}{name}{}", if is_dir { "/" } else { "" });
            let detail = if is_dir { "directory" } else { "file" };
            self.add(Cow::Owned(name.to_lowercase()), SuggestionKind::File, owned(text, Some(detail.into())));
        }
    }

    fn render(&self, cand: Cand<'a>, score: i64, taken: &HashSet<String>) -> Suggestion {
        let case = |s: &str| if self.upper { s.to_uppercase() } else { s.to_lowercase() };
        let (text, display, detail) = match cand.item {
            Item::Ident { name, detail } => {
                let t = self.q(name);
                (t.clone(), t, detail.map(str::to_string))
            }
            Item::Owned { text, display, detail } => (text, display, detail),
            Item::Keyword(k) => {
                let t = case(k);
                (t.clone(), t, None)
            }
            Item::Builtin(f) => {
                let t = case(f);
                (t.clone(), format!("{t}()"), None)
            }
            Item::Func(f) => {
                let ret = (!f.return_type.is_empty()).then(|| f.return_type.clone());
                (self.q(&f.name), format!("{}({})", f.name, f.args), ret)
            }
            Item::Rel { rel, alias } => {
                let t = self.q(&rel.name);
                let t = if alias { format!("{t} {}", self.q(&unique_alias(&rel.name, taken))) } else { t };
                (t.clone(), t, Some(rel.kind.label().to_string()))
            }
        };
        Suggestion { text, display, kind: cand.kind, detail, score }
    }

    fn finish(mut self) -> Vec<Suggestion> {
        let max = self.c.options.max_items;
        let cmp = |a: &(i64, usize, Cand), b: &(i64, usize, Cand)| {
            b.0.cmp(&a.0).then_with(|| a.2.key.cmp(&b.2.key)).then_with(|| a.1.cmp(&b.1))
        };
        let keep = max.saturating_mul(2).saturating_add(32);
        if self.scored.len() > keep {
            self.scored.select_nth_unstable_by(keep, cmp);
            self.scored.truncate(keep);
        }
        self.scored.sort_by(cmp);
        let taken: HashSet<String> = self.st.refs.iter().map(|r| r.refname().to_lowercase()).collect();
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (score, _, cand) in std::mem::take(&mut self.scored) {
            if out.len() >= max {
                break;
            }
            let s = self.render(cand, score, &taken);
            if seen.insert(s.text.to_lowercase()) {
                out.push(s);
            }
        }
        out
    }
}

/// pgcli-style alias: upper-case letters of a CamelCase name, else initials of `_` segments; made unique.
fn unique_alias(table: &str, taken: &HashSet<String>) -> String {
    let caps: String = table.chars().filter(|c| c.is_uppercase()).collect();
    let base = if caps.is_empty() {
        table.split('_').filter_map(|s| s.chars().next()).collect::<String>()
    } else {
        caps
    }
    .to_lowercase();
    let base = if base.is_empty() { "t".to_string() } else { base };
    if !taken.contains(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}{n}")).find(|a| !taken.contains(a)).unwrap_or(base)
}

/// Keywords that can begin or continue an expression.
const EXPR_KEYWORDS: &[&str] = &[
    "DISTINCT", "CASE", "WHEN", "THEN", "ELSE", "END", "NOT", "NULL", "EXISTS", "TRUE", "FALSE", "CAST", "AND", "OR",
    "IN", "IS", "LIKE", "BETWEEN", "INTERVAL", "CURRENT_DATE", "CURRENT_TIMESTAMP",
];

fn start_keywords(backend: Backend) -> &'static [&'static str] {
    match backend {
        Backend::Postgres => &[
            "SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "CREATE", "ALTER", "DROP", "SHOW", "EXPLAIN", "BEGIN",
            "COMMIT", "ROLLBACK", "SAVEPOINT", "RELEASE", "START", "TRUNCATE", "GRANT", "REVOKE", "SET", "RESET",
            "VALUES", "TABLE", "COPY", "ANALYZE", "VACUUM", "CALL", "DO", "LISTEN", "NOTIFY", "UNLISTEN", "PREPARE",
            "EXECUTE", "DEALLOCATE", "DISCARD", "COMMENT", "REFRESH", "CLUSTER", "REINDEX", "LOCK", "MERGE",
            "CHECKPOINT",
        ],
        Backend::MySql => &[
            "SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "CREATE", "ALTER", "DROP", "SHOW", "EXPLAIN", "BEGIN",
            "COMMIT", "ROLLBACK", "SAVEPOINT", "RELEASE", "START", "TRUNCATE", "GRANT", "REVOKE", "SET", "VALUES",
            "TABLE", "DESCRIBE", "DESC", "USE", "SOURCE", "HELP", "REPLACE", "CALL", "LOCK", "UNLOCK", "ANALYZE",
            "OPTIMIZE", "REPAIR", "CHECK", "FLUSH", "KILL", "PREPARE", "EXECUTE", "DEALLOCATE", "LOAD", "RENAME",
            "HANDLER",
        ],
        Backend::Sqlite => &[
            "SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "CREATE", "ALTER", "DROP", "EXPLAIN", "BEGIN", "COMMIT",
            "ROLLBACK", "SAVEPOINT", "RELEASE", "END", "VALUES", "REPLACE", "PRAGMA", "ATTACH", "DETACH", "ANALYZE",
            "VACUUM", "REINDEX",
        ],
    }
}
