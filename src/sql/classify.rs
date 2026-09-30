use crate::db::Backend;
use crate::sql::lexer::{Token, TokenKind, tokenize};

/// Significant (non-trivia) tokens of a statement with upper-cased word text.
struct Sig {
    kind: TokenKind,
    word: Option<String>,
    depth: i32,
}

fn significant(sql: &str, backend: Backend) -> Vec<Sig> {
    let mut depth = 0;
    tokenize(sql, backend)
        .into_iter()
        .filter(|t: &Token| !t.is_trivia())
        .map(|t| {
            if t.kind == TokenKind::RParen {
                depth -= 1;
            }
            let s = Sig {
                kind: t.kind,
                word: t.is_word().then(|| t.text(sql).to_ascii_uppercase()),
                depth,
            };
            if t.kind == TokenKind::LParen {
                depth += 1;
            }
            s
        })
        .collect()
}

pub fn first_keyword(sql: &str, backend: Backend) -> Option<String> {
    significant(sql, backend).into_iter().find_map(|s| s.word)
}

/// The keyword that determines what a statement does, looking through a leading `WITH` CTE list
/// and `EXPLAIN [ANALYZE]`.
pub fn main_verb(sql: &str, backend: Backend) -> Option<String> {
    let sig = significant(sql, backend);
    let first = sig.iter().find_map(|s| s.word.clone())?;
    if first == "WITH" {
        return sig
            .iter()
            .skip(1)
            .filter(|s| s.depth == 0)
            .filter_map(|s| s.word.as_deref())
            .find(|w| matches!(*w, "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "VALUES" | "TABLE"))
            .map(str::to_string);
    }
    Some(first)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destructive {
    /// The config keyword that matched, e.g. `drop`, `unconditional_delete`.
    pub rule: String,
    pub reason: String,
}

/// Checks a single statement against destructive-warning rules.
/// Rules: any statement verb in lowercase (`drop`, `truncate`, `alter`, `delete`, `update`, `shutdown`,
/// `grant`, `revoke`, `rename`…) plus `unconditional_update` / `unconditional_delete` (no top-level WHERE).
pub fn destructive(sql: &str, backend: Backend, rules: &[String]) -> Option<Destructive> {
    let sig = significant(sql, backend);
    let words: Vec<&str> = sig.iter().filter_map(|s| s.word.as_deref()).collect();
    let first = *words.first()?;
    let has = |r: &str| rules.iter().any(|x| x.eq_ignore_ascii_case(r));

    // data-modifying verbs can hide behind a WITH clause
    let verbs: Vec<(usize, &str)> = sig
        .iter()
        .enumerate()
        .filter(|(_, s)| s.depth == 0)
        .filter_map(|(i, s)| s.word.as_deref().map(|w| (i, w)))
        .filter(|(_, w)| matches!(*w, "DELETE" | "UPDATE"))
        .collect();

    for &(idx, verb) in &verbs {
        let is_stmt_verb = idx == 0
            || first == "WITH"
            || (first == "EXPLAIN" && words.contains(&"ANALYZE"));
        if !is_stmt_verb {
            continue;
        }
        // skip `ON DELETE`/`ON UPDATE` and `FOR UPDATE`
        let prev = sig[..idx].iter().rev().find_map(|s| s.word.as_deref());
        if matches!(prev, Some("ON" | "FOR" | "KEY")) {
            continue;
        }
        let where_follows = sig[idx..].iter().any(|s| s.depth == 0 && s.word.as_deref() == Some("WHERE"));
        let lower = verb.to_ascii_lowercase();
        if has(&lower) {
            return Some(Destructive { rule: lower.clone(), reason: format!("{verb} statement") });
        }
        if !where_follows && has(&format!("unconditional_{lower}")) {
            return Some(Destructive {
                rule: format!("unconditional_{lower}"),
                reason: format!("{verb} without a WHERE clause affects every row"),
            });
        }
    }

    let lower = first.to_ascii_lowercase();
    if !matches!(first, "DELETE" | "UPDATE" | "WITH") && has(&lower) {
        let object = words.get(1).map(|w| format!(" {w}")).unwrap_or_default();
        return Some(Destructive { rule: lower, reason: format!("{first}{object} statement") });
    }
    None
}

/// Conservative client-side read-only check used by `--readonly` (the server session is also put
/// into read-only mode where the backend supports it).
pub fn is_read_only(sql: &str, backend: Backend) -> bool {
    let sig = significant(sql, backend);
    let words: Vec<&str> = sig.iter().filter_map(|s| s.word.as_deref()).collect();
    let Some(&first) = words.first() else {
        return true;
    };
    let writes = |ws: &[&str]| {
        ws.iter().any(|w| {
            matches!(
                *w,
                "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "INTO" | "CREATE" | "DROP" | "ALTER" | "TRUNCATE" | "REPLACE"
                    | "GRANT" | "REVOKE" | "COPY" | "CALL" | "LOCK"
            )
        })
    };
    match first {
        "SELECT" | "VALUES" | "TABLE" | "WITH" => !writes(&words),
        "SHOW" | "DESCRIBE" | "DESC" | "HELP" => true,
        "EXPLAIN" => !words.contains(&"ANALYZE") || !writes(&words),
        "PRAGMA" => backend == Backend::Sqlite && !sig.iter().any(|s| s.kind == TokenKind::Operator),
        "SET" | "RESET" | "USE" | "BEGIN" | "START" | "COMMIT" | "ROLLBACK" | "END" | "SAVEPOINT" | "RELEASE" => {
            !words.iter().any(|w| matches!(*w, "PASSWORD" | "GLOBAL" | "PERSIST"))
        }
        _ => false,
    }
}

/// Statements after which the cached catalog (completion, sidebar) is stale.
pub fn changes_schema(sql: &str, backend: Backend) -> bool {
    matches!(
        first_keyword(sql, backend).as_deref(),
        Some("CREATE" | "ALTER" | "DROP" | "RENAME" | "ATTACH" | "DETACH" | "IMPORT" | "COMMENT" | "USE")
    )
}

/// `USE db` (MySQL) → target database name.
pub fn use_database(sql: &str, backend: Backend) -> Option<String> {
    let toks: Vec<Token> = tokenize(sql, backend).into_iter().filter(|t| !t.is_trivia()).collect();
    if toks.len() < 2 || !toks[0].text(sql).eq_ignore_ascii_case("use") {
        return None;
    }
    let t = toks[1];
    match t.kind {
        TokenKind::QuotedIdent => Some(crate::sql::lexer::unquote_ident(t.text(sql))),
        _ if t.is_word() => Some(t.text(sql).to_string()),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxEffect {
    Begin,
    End,
    None,
}

pub fn transaction_effect(sql: &str, backend: Backend) -> TxEffect {
    let sig = significant(sql, backend);
    let words: Vec<&str> = sig.iter().filter_map(|s| s.word.as_deref()).take(2).collect();
    match words.as_slice() {
        ["BEGIN", ..] | ["START", "TRANSACTION"] => TxEffect::Begin,
        ["COMMIT", ..] | ["END", ..] | ["ABORT", ..] => TxEffect::End,
        ["ROLLBACK", next] if *next != "TO" => TxEffect::End,
        ["ROLLBACK"] => TxEffect::End,
        _ => TxEffect::None,
    }
}

/// Statements that normally return rows (used to decide whether to describe result columns).
pub fn returns_rows(sql: &str, backend: Backend) -> bool {
    let sig = significant(sql, backend);
    let words: Vec<&str> = sig.iter().filter_map(|s| s.word.as_deref()).collect();
    match words.first().copied() {
        Some("SELECT" | "VALUES" | "TABLE" | "SHOW" | "EXPLAIN" | "DESCRIBE" | "DESC" | "PRAGMA") => true,
        Some("WITH") => true,
        Some(_) => words.contains(&"RETURNING"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Vec<String> {
        ["drop", "truncate", "shutdown", "unconditional_update", "unconditional_delete"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn where_less_delete_is_destructive_but_filtered_delete_is_not() {
        let r = rules();
        assert!(destructive("DELETE FROM users", Backend::Postgres, &r).is_some());
        assert!(destructive("delete from users where id = 1", Backend::Postgres, &r).is_none());
        assert!(destructive("update t set a = (select 1 where true)", Backend::Postgres, &r).is_some());
    }

    #[test]
    fn cte_delete_is_detected() {
        let r = rules();
        assert!(destructive("WITH x AS (SELECT 1) DELETE FROM t", Backend::Postgres, &r).is_some());
    }

    #[test]
    fn foreign_key_actions_are_not_statements() {
        let r = rules();
        let sql = "create table t (a int references b on delete cascade on update cascade)";
        assert!(destructive(sql, Backend::Postgres, &r).is_none());
        assert!(destructive("select * from t for update", Backend::Postgres, &r).is_none());
    }

    #[test]
    fn drop_reports_object() {
        let d = destructive("drop table users", Backend::MySql, &rules()).unwrap();
        assert_eq!(d.rule, "drop");
        assert!(d.reason.contains("TABLE"));
    }

    #[test]
    fn read_only_detection() {
        assert!(is_read_only("select * from t", Backend::Postgres));
        assert!(!is_read_only("select * into x from t", Backend::Postgres));
        assert!(!is_read_only("with d as (delete from t returning *) select * from d", Backend::Postgres));
        assert!(is_read_only("explain select 1", Backend::MySql));
        assert!(!is_read_only("explain analyze delete from t", Backend::Postgres));
        assert!(is_read_only("pragma table_info(t)", Backend::Sqlite));
        assert!(!is_read_only("pragma journal_mode = wal", Backend::Sqlite));
        assert!(!is_read_only("insert into t values (1)", Backend::Sqlite));
    }

    #[test]
    fn tx_and_use() {
        assert_eq!(transaction_effect("begin", Backend::Postgres), TxEffect::Begin);
        assert_eq!(transaction_effect("rollback to savepoint a", Backend::Postgres), TxEffect::None);
        assert_eq!(transaction_effect("rollback", Backend::Postgres), TxEffect::End);
        assert_eq!(use_database("use `my db`", Backend::MySql).as_deref(), Some("my db"));
    }

    #[test]
    fn main_verb_through_cte() {
        assert_eq!(main_verb("with a as (select 1) update t set x=1", Backend::Postgres).as_deref(), Some("UPDATE"));
    }
}
