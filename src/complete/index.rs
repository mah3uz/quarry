use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use crate::db::{Backend, Catalog};
use crate::sql::keywords;

/// A catalog object with its lower-cased name, addressed by (schema index, item index).
pub(crate) struct Entry {
    pub schema: usize,
    pub idx: usize,
    pub key: String,
    pub visible: bool,
}

/// Lower-cased names precomputed once per catalog snapshot so completion never re-lowercases.
pub(crate) struct Index {
    pub catalog: Arc<Catalog>,
    pub backend: Backend,
    pub rels: Vec<Entry>,
    pub funcs: Vec<Entry>,
    pub types: Vec<Entry>,
    pub schemas: Vec<String>,
    pub keywords: Vec<(&'static str, String)>,
    pub datatypes: Vec<(&'static str, String)>,
    pub builtins: Vec<(&'static str, String)>,
    /// Unique column names across the catalog: (schema, relation, column, key). Only for non-smart mode.
    columns: OnceLock<Vec<(usize, usize, usize, String)>>,
}

/// Multi-word keywords offered alongside single keywords.
const COMPOUND_KEYWORDS: &[&str] = &[
    "GROUP BY", "ORDER BY", "PARTITION BY", "LEFT JOIN", "RIGHT JOIN", "INNER JOIN", "CROSS JOIN",
    "FULL OUTER JOIN", "LEFT OUTER JOIN", "UNION ALL", "IS NULL", "IS NOT NULL", "NOT NULL", "PRIMARY KEY",
    "INSERT INTO", "DELETE FROM",
];

fn lowered(words: impl IntoIterator<Item = &'static str>) -> Vec<(&'static str, String)> {
    words.into_iter().map(|w| (w, w.to_lowercase())).collect()
}

impl Index {
    pub fn build(catalog: Arc<Catalog>, backend: Backend) -> Index {
        let visible = |name: &str| catalog.search_path.is_empty() || catalog.is_on_search_path(name);
        let mut rels = Vec::new();
        let mut funcs = Vec::new();
        let mut types = Vec::new();
        for (si, s) in catalog.schemas.iter().enumerate() {
            let vis = visible(&s.name);
            rels.extend(s.relations.iter().enumerate().map(|(i, r)| Entry { schema: si, idx: i, key: r.name.to_lowercase(), visible: vis }));
            funcs.extend(s.functions.iter().enumerate().map(|(i, f)| Entry { schema: si, idx: i, key: f.name.to_lowercase(), visible: vis }));
            types.extend(s.types.iter().enumerate().map(|(i, t)| Entry { schema: si, idx: i, key: t.to_lowercase(), visible: vis }));
        }
        let schemas = catalog.schemas.iter().map(|s| s.name.to_lowercase()).collect();
        let keywords = lowered(keywords::keywords(backend).iter().copied().chain(COMPOUND_KEYWORDS.iter().copied()));
        Index {
            rels,
            funcs,
            types,
            schemas,
            keywords,
            datatypes: lowered(keywords::datatypes(backend).iter().copied()),
            builtins: lowered(keywords::functions(backend).iter().copied()),
            columns: OnceLock::new(),
            catalog,
            backend,
        }
    }

    pub fn columns(&self) -> &[(usize, usize, usize, String)] {
        self.columns.get_or_init(|| {
            let mut seen = HashSet::new();
            let mut out = Vec::new();
            for (si, s) in self.catalog.schemas.iter().enumerate() {
                for (ri, r) in s.relations.iter().enumerate() {
                    for (ci, c) in r.columns.iter().enumerate() {
                        if seen.insert(c.name.as_str()) {
                            out.push((si, ri, ci, c.name.to_lowercase()));
                        }
                    }
                }
            }
            out
        })
    }

    pub fn schema_index(&self, name: &str) -> Option<usize> {
        let lower = name.to_lowercase();
        self.catalog.schemas.iter().position(|s| s.name == name).or_else(|| self.schemas.iter().position(|s| *s == lower))
    }
}
