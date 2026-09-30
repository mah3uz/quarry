use super::Backend;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RelKind {
    Table,
    View,
    MaterializedView,
    ForeignTable,
    PartitionedTable,
    SystemTable,
}

impl RelKind {
    pub fn label(self) -> &'static str {
        match self {
            RelKind::Table => "table",
            RelKind::View => "view",
            RelKind::MaterializedView => "materialized view",
            RelKind::ForeignTable => "foreign table",
            RelKind::PartitionedTable => "partitioned table",
            RelKind::SystemTable => "system table",
        }
    }

    pub fn is_view(self) -> bool {
        matches!(self, RelKind::View | RelKind::MaterializedView)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
    /// Auto-increment / identity / serial.
    pub auto: bool,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Relation {
    pub schema: String,
    pub name: String,
    pub kind: RelKind,
    /// Empty when loaded via `list_relations` on PostgreSQL and SQLite (lazy).
    pub columns: Vec<ColumnInfo>,
    pub comment: Option<String>,
    pub row_estimate: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionKind {
    Function,
    Aggregate,
    Window,
    Procedure,
    Trigger,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionInfo {
    pub schema: String,
    pub name: String,
    /// Argument signature as displayed, e.g. `a integer, b text`.
    pub args: String,
    pub return_type: String,
    pub kind: FunctionKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForeignKey {
    pub name: String,
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,
    pub ref_schema: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SchemaInfo {
    pub name: String,
    pub relations: Vec<Relation>,
    pub functions: Vec<FunctionInfo>,
    /// User-defined types / domains / enums.
    pub types: Vec<String>,
}

/// Metadata snapshot for completion and the sidebar.
/// MySQL: a "schema" is a database; only the current database's relations carry columns.
/// SQLite: schemas are attached databases (`main`, `temp`, …).
#[derive(Clone, Debug, PartialEq)]
pub struct Catalog {
    pub backend: Backend,
    pub databases: Vec<String>,
    pub current_database: Option<String>,
    /// Unqualified names resolve against these, in order (pg search_path; mysql current db; sqlite main).
    pub search_path: Vec<String>,
    pub schemas: Vec<SchemaInfo>,
    pub foreign_keys: Vec<ForeignKey>,
    pub users: Vec<String>,
}

impl Catalog {
    pub fn empty(backend: Backend) -> Self {
        Catalog {
            backend,
            databases: Vec::new(),
            current_database: None,
            search_path: Vec::new(),
            schemas: Vec::new(),
            foreign_keys: Vec::new(),
            users: Vec::new(),
        }
    }

    pub fn schema(&self, name: &str) -> Option<&SchemaInfo> {
        self.schemas.iter().find(|s| s.name == name)
            .or_else(|| self.schemas.iter().find(|s| s.name.eq_ignore_ascii_case(name)))
    }

    /// Resolves a possibly-unqualified relation name using the search path.
    pub fn find_relation(&self, schema: Option<&str>, name: &str) -> Option<&Relation> {
        fn find_in<'a>(s: &'a SchemaInfo, name: &str) -> Option<&'a Relation> {
            s.relations.iter().find(|r| r.name == name)
                .or_else(|| s.relations.iter().find(|r| r.name.eq_ignore_ascii_case(name)))
        }
        match schema {
            Some(s) => self.schema(s).and_then(|s| find_in(s, name)),
            None => self
                .search_path
                .iter()
                .filter_map(|s| self.schema(s))
                .find_map(|s| find_in(s, name))
                .or_else(|| self.schemas.iter().find_map(|s| find_in(s, name))),
        }
    }

    pub fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.schemas.iter().flat_map(|s| s.relations.iter())
    }

    pub fn functions(&self) -> impl Iterator<Item = &FunctionInfo> {
        self.schemas.iter().flat_map(|s| s.functions.iter())
    }

    pub fn is_on_search_path(&self, schema: &str) -> bool {
        self.search_path.iter().any(|s| s == schema)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    /// btree / hash / gin …
    pub method: Option<String>,
    /// Full definition where the server provides one (pg `pg_get_indexdef`, sqlite `sql`).
    pub definition: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConstraintInfo {
    pub name: String,
    /// PRIMARY KEY / UNIQUE / CHECK / FOREIGN KEY / EXCLUDE
    pub kind: String,
    pub definition: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TriggerInfo {
    pub name: String,
    /// e.g. `BEFORE INSERT`
    pub event: String,
    pub definition: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableDetails {
    pub schema: String,
    pub name: String,
    pub kind: RelKind,
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKey>,
    /// FKs in other tables pointing at this one.
    pub referenced_by: Vec<ForeignKey>,
    pub constraints: Vec<ConstraintInfo>,
    pub triggers: Vec<TriggerInfo>,
    pub row_estimate: Option<i64>,
    pub size_bytes: Option<i64>,
    pub comment: Option<String>,
    /// View definition (views only).
    pub view_definition: Option<String>,
}

impl TableDetails {
    pub fn primary_key(&self) -> Vec<&str> {
        self.columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect()
    }
}
