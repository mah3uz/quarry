use crate::db::Backend;
use crate::sql::lexer::{Token, TokenKind, tokenize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terminator {
    /// Input ended without a delimiter.
    None,
    Delimiter,
    /// `\G` — show this result vertically.
    Vertical,
    /// `\g` — plain "go".
    Go,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub text: String,
    /// Byte range of `text` inside the source.
    pub start: usize,
    pub end: usize,
    pub terminator: Terminator,
}

/// Splits `src` into statements. Semicolons inside strings, comments, dollar quotes,
/// trigger / routine bodies (`BEGIN … END`) and pg `BEGIN ATOMIC` blocks do not split.
/// With a custom `delimiter` (MySQL `DELIMITER //`), `;` is ordinary text.
pub fn split(src: &str, backend: Backend, delimiter: &str) -> Vec<Statement> {
    let tokens = tokenize(src, backend);
    let words_upper: Vec<Option<String>> = tokens
        .iter()
        .map(|t| t.is_word().then(|| t.text(src).to_ascii_uppercase()))
        .collect();
    let semicolon = delimiter.is_empty() || delimiter == ";";
    let mut out = Vec::new();
    let mut stmt_start: Option<usize> = None;
    let mut last_end = 0usize;
    let mut routine = RoutineState::default();
    let mut i = 0;

    while i < tokens.len() {
        let t = tokens[i];
        if t.is_trivia() {
            i += 1;
            continue;
        }
        let at_depth0 = routine.depth == 0;

        if at_depth0 {
            if let Some((term, next)) = terminator_at(src, &tokens, i, semicolon, delimiter) {
                if let Some(s) = stmt_start.take() {
                    push(&mut out, src, s, last_end, term);
                }
                routine = RoutineState::default();
                i = next;
                continue;
            }
        }

        if stmt_start.is_none() {
            stmt_start = Some(t.start);
        }
        routine.observe(i, &tokens, &words_upper, backend);
        last_end = t.end;
        i += 1;
    }
    if let Some(s) = stmt_start {
        push(&mut out, src, s, last_end, Terminator::None);
    }
    out
}

/// True when the buffer ends in a statement terminator (outside any string/comment/block),
/// i.e. a multi-line REPL should submit it.
pub fn ends_with_terminator(src: &str, backend: Backend, delimiter: &str) -> bool {
    !has_unterminated(src, backend)
        && split(src, backend, delimiter)
            .last()
            .is_some_and(|s| s.terminator != Terminator::None)
}

pub fn has_unterminated(src: &str, backend: Backend) -> bool {
    tokenize(src, backend).last().is_some_and(|t| t.unterminated)
}

fn push(out: &mut Vec<Statement>, src: &str, start: usize, end: usize, term: Terminator) {
    let text = src[start..end].trim();
    if text.is_empty() {
        return;
    }
    out.push(Statement { text: text.to_string(), start, end, terminator: term });
}

fn terminator_at(
    src: &str,
    tokens: &[Token],
    i: usize,
    semicolon: bool,
    delimiter: &str,
) -> Option<(Terminator, usize)> {
    let t = tokens[i];
    if t.kind == TokenKind::Backslash {
        if let Some(n) = tokens.get(i + 1) {
            if n.start == t.end {
                match n.text(src) {
                    "G" => return Some((Terminator::Vertical, i + 2)),
                    "g" => return Some((Terminator::Go, i + 2)),
                    _ => {}
                }
            }
        }
        return None;
    }
    if semicolon {
        return (t.kind == TokenKind::Semicolon).then_some((Terminator::Delimiter, i + 1));
    }
    if matches!(t.kind, TokenKind::String | TokenKind::QuotedIdent) {
        return None;
    }
    if src[t.start..].starts_with(delimiter) {
        let stop = t.start + delimiter.len();
        let mut j = i;
        while j < tokens.len() && tokens[j].start < stop {
            j += 1;
        }
        return Some((Terminator::Delimiter, j));
    }
    None
}

/// Tracks `BEGIN … END` nesting inside CREATE TRIGGER / FUNCTION / PROCEDURE / EVENT bodies.
#[derive(Default)]
struct RoutineState {
    /// Number of significant tokens seen in this statement.
    seen: usize,
    in_routine: bool,
    depth: usize,
}

impl RoutineState {
    fn observe(&mut self, i: usize, tokens: &[Token], words: &[Option<String>], backend: Backend) {
        self.seen += 1;
        let Some(w) = words[i].as_deref() else {
            return;
        };
        if !self.in_routine {
            if self.seen <= 12 && matches!(w, "TRIGGER" | "FUNCTION" | "PROCEDURE" | "EVENT") {
                self.in_routine = first_word(words, i) == Some("CREATE");
            }
            return;
        }
        let next = next_word(tokens, words, i);
        match w {
            "BEGIN" => {
                let opens = match backend {
                    Backend::Postgres => next == Some("ATOMIC"),
                    _ => !matches!(next, Some("TRANSACTION" | "WORK")),
                };
                if opens {
                    self.depth += 1;
                }
            }
            "CASE" if self.depth > 0 => {
                // `END CASE` closes a CASE statement: the CASE after END is not an opener
                if prev_word(tokens, words, i) != Some("END") {
                    self.depth += 1;
                }
            }
            "END" if self.depth > 0 => {
                if !matches!(next, Some("IF" | "LOOP" | "WHILE" | "REPEAT" | "FOR")) {
                    self.depth -= 1;
                }
            }
            _ => {}
        }
    }
}

fn first_word(words: &[Option<String>], upto: usize) -> Option<&str> {
    words[..upto].iter().flatten().next().map(String::as_str)
}

fn next_word<'a>(tokens: &[Token], words: &'a [Option<String>], i: usize) -> Option<&'a str> {
    (i + 1..tokens.len())
        .find(|&j| !tokens[j].is_trivia())
        .and_then(|j| words[j].as_deref())
}

fn prev_word<'a>(tokens: &[Token], words: &'a [Option<String>], i: usize) -> Option<&'a str> {
    (0..i)
        .rev()
        .find(|&j| !tokens[j].is_trivia())
        .and_then(|j| words[j].as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(src: &str, b: Backend) -> Vec<String> {
        split(src, b, ";").into_iter().map(|s| s.text).collect()
    }

    #[test]
    fn splits_simple() {
        assert_eq!(texts("select 1; select 2;", Backend::Postgres), vec!["select 1", "select 2"]);
    }

    #[test]
    fn ignores_semicolons_in_strings_and_comments() {
        let v = texts("select ';'; -- a;b\nselect /* ; */ 2", Backend::MySql);
        assert_eq!(v.len(), 2);
        assert_eq!(v[1], "select /* ; */ 2", "leading comments are not part of the statement");
    }

    #[test]
    fn comment_only_statements_are_dropped() {
        assert_eq!(texts("select 1; -- trailing", Backend::Sqlite), vec!["select 1"]);
    }

    #[test]
    fn sqlite_trigger_body() {
        let src = "CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE b SET x = CASE WHEN 1 THEN 2 END; DELETE FROM c; END; select 1";
        let v = texts(src, Backend::Sqlite);
        assert_eq!(v.len(), 2, "{v:?}");
        assert!(v[0].ends_with("END"));
    }

    #[test]
    fn mysql_procedure_without_delimiter() {
        let src = "CREATE PROCEDURE p() BEGIN IF 1 THEN SELECT 1; END IF; SELECT 2; END; CALL p()";
        let v = texts(src, Backend::MySql);
        assert_eq!(v.len(), 2, "{v:?}");
        assert_eq!(v[1], "CALL p()");
    }

    #[test]
    fn pg_begin_is_transaction_outside_routines() {
        assert_eq!(texts("BEGIN; select 1; COMMIT;", Backend::Postgres).len(), 3);
    }

    #[test]
    fn pg_begin_atomic() {
        let src = "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END; select 3";
        assert_eq!(texts(src, Backend::Postgres).len(), 2);
    }

    #[test]
    fn custom_delimiter() {
        let v: Vec<_> = split("select 1; select 2// select 3//", Backend::MySql, "//")
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_eq!(v, vec!["select 1; select 2", "select 3"]);
    }

    #[test]
    fn vertical_terminator() {
        let v = split("select 1\\G select 2", Backend::MySql, ";");
        assert_eq!(v[0].terminator, Terminator::Vertical);
        assert_eq!(v[0].text, "select 1");
        assert_eq!(v[1].terminator, Terminator::None);
    }

    #[test]
    fn completeness() {
        assert!(ends_with_terminator("select 1;", Backend::Postgres, ";"));
        assert!(ends_with_terminator("select 1;  -- done\n", Backend::Postgres, ";"));
        assert!(!ends_with_terminator("select ';", Backend::Postgres, ";"));
        assert!(!ends_with_terminator("select 1; select", Backend::Postgres, ";"));
        assert!(!ends_with_terminator("create trigger t after insert on a begin select 1;", Backend::Sqlite, ";"));
    }
}
