use crate::db::Backend;
use crate::sql::keywords;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Whitespace,
    LineComment,
    BlockComment,
    Keyword,
    DataType,
    Builtin,
    Ident,
    QuotedIdent,
    String,
    Number,
    Parameter,
    Variable,
    Operator,
    Comma,
    Dot,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semicolon,
    Backslash,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
    /// String, quoted identifier or block comment that reached end of input unclosed.
    pub unterminated: bool,
}

impl Token {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }

    pub fn is_trivia(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Whitespace | TokenKind::LineComment | TokenKind::BlockComment
        )
    }

    pub fn is_word(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Keyword | TokenKind::DataType | TokenKind::Builtin | TokenKind::Ident
        )
    }
}

pub fn tokenize(src: &str, backend: Backend) -> Vec<Token> {
    Lexer { src, bytes: src.as_bytes(), pos: 0, backend }.run()
}

/// Unquotes an identifier token's text: `"a""b"` -> `a"b`, `` `x` `` -> `x`, `[x]` -> `x`.
pub fn unquote_ident(text: &str) -> String {
    let b = text.as_bytes();
    if b.len() >= 2 {
        let (open, close) = (b[0], b[b.len() - 1]);
        let inner = &text[1..text.len() - 1];
        match (open, close) {
            (b'"', b'"') => return inner.replace("\"\"", "\""),
            (b'`', b'`') => return inner.replace("``", "`"),
            (b'[', b']') => return inner.to_string(),
            _ => {}
        }
    }
    text.to_string()
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    backend: Backend,
}

impl<'a> Lexer<'a> {
    fn peek(&self, off: usize) -> u8 {
        *self.bytes.get(self.pos + off).unwrap_or(&0)
    }

    fn run(mut self) -> Vec<Token> {
        let mut out = Vec::with_capacity(self.bytes.len() / 3 + 1);
        while self.pos < self.bytes.len() {
            let start = self.pos;
            let (kind, unterminated) = self.next_kind();
            debug_assert!(self.pos > start);
            out.push(Token { kind, start, end: self.pos, unterminated });
        }
        out
    }

    fn next_kind(&mut self) -> (TokenKind, bool) {
        let c = self.peek(0);
        let my = self.backend == Backend::MySql;
        let pg = self.backend == Backend::Postgres;
        match c {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c => {
                while matches!(self.peek(0), b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c) {
                    self.pos += 1;
                }
                (TokenKind::Whitespace, false)
            }
            b'-' if self.peek(1) == b'-' => {
                self.skip_line();
                (TokenKind::LineComment, false)
            }
            b'#' if my => {
                self.skip_line();
                (TokenKind::LineComment, false)
            }
            b'/' if self.peek(1) == b'*' => {
                let closed = self.block_comment(pg);
                (TokenKind::BlockComment, !closed)
            }
            b'\'' => {
                let bs = my;
                (TokenKind::String, !self.quoted(b'\'', bs))
            }
            b'"' => {
                if my {
                    (TokenKind::String, !self.quoted(b'"', true))
                } else {
                    (TokenKind::QuotedIdent, !self.quoted(b'"', false))
                }
            }
            b'`' if !pg => (TokenKind::QuotedIdent, !self.quoted(b'`', false)),
            b'[' if self.backend == Backend::Sqlite => {
                self.pos += 1;
                while self.pos < self.bytes.len() && self.peek(0) != b']' {
                    self.pos += 1;
                }
                let closed = self.pos < self.bytes.len();
                if closed {
                    self.pos += 1;
                }
                (TokenKind::QuotedIdent, !closed)
            }
            b'[' => {
                self.pos += 1;
                (TokenKind::LBracket, false)
            }
            b']' => {
                self.pos += 1;
                (TokenKind::RBracket, false)
            }
            b'$' if pg => self.dollar(),
            b'?' if !pg => {
                self.pos += 1;
                while self.peek(0).is_ascii_digit() {
                    self.pos += 1;
                }
                (TokenKind::Parameter, false)
            }
            b':' if self.backend == Backend::Sqlite && is_ident_start(self.peek(1)) => {
                self.pos += 1;
                self.ident_tail();
                (TokenKind::Parameter, false)
            }
            b'@' if my => {
                self.pos += 1;
                if self.peek(0) == b'@' {
                    self.pos += 1;
                }
                match self.peek(0) {
                    b'\'' | b'"' | b'`' => {
                        let q = self.peek(0);
                        self.quoted(q, q != b'`');
                    }
                    _ => {
                        while is_ident_char(self.peek(0)) || self.peek(0) == b'.' {
                            self.pos += 1;
                        }
                    }
                }
                (TokenKind::Variable, false)
            }
            b'@' | b'$' if self.backend == Backend::Sqlite && is_ident_start(self.peek(1)) => {
                self.pos += 1;
                self.ident_tail();
                (TokenKind::Parameter, false)
            }
            b'0'..=b'9' => {
                self.number();
                (TokenKind::Number, false)
            }
            b'.' if self.peek(1).is_ascii_digit() => {
                self.number();
                (TokenKind::Number, false)
            }
            b'.' => {
                self.pos += 1;
                (TokenKind::Dot, false)
            }
            b',' => {
                self.pos += 1;
                (TokenKind::Comma, false)
            }
            b'(' => {
                self.pos += 1;
                (TokenKind::LParen, false)
            }
            b')' => {
                self.pos += 1;
                (TokenKind::RParen, false)
            }
            b';' => {
                self.pos += 1;
                (TokenKind::Semicolon, false)
            }
            b'\\' => {
                self.pos += 1;
                (TokenKind::Backslash, false)
            }
            _ if is_ident_start(c) => self.word(),
            _ if is_op_char(c) => {
                if pg && c == b':' && self.peek(1) == b':' {
                    self.pos += 2;
                    return (TokenKind::Operator, false);
                }
                let start = self.pos;
                while is_op_char(self.peek(0)) {
                    let comment_start = (self.peek(0) == b'-' && self.peek(1) == b'-')
                        || (self.peek(0) == b'/' && self.peek(1) == b'*');
                    if comment_start && self.pos > start {
                        break;
                    }
                    self.pos += 1;
                }
                (TokenKind::Operator, false)
            }
            _ => {
                let ch = self.src[self.pos..].chars().next().unwrap();
                self.pos += ch.len_utf8();
                (TokenKind::Unknown, false)
            }
        }
    }

    fn skip_line(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
            self.pos += 1;
        }
    }

    fn block_comment(&mut self, nested: bool) -> bool {
        self.pos += 2;
        let mut depth = 1;
        while self.pos < self.bytes.len() {
            if self.peek(0) == b'*' && self.peek(1) == b'/' {
                self.pos += 2;
                depth -= 1;
                if depth == 0 {
                    return true;
                }
            } else if nested && self.peek(0) == b'/' && self.peek(1) == b'*' {
                self.pos += 2;
                depth += 1;
            } else {
                self.pos += 1;
            }
        }
        false
    }

    /// Consumes a quoted run starting at the opening quote. Doubled quotes escape;
    /// backslash escapes when `backslash` is set. Returns whether it was closed.
    fn quoted(&mut self, q: u8, backslash: bool) -> bool {
        self.pos += 1;
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if backslash && c == b'\\' {
                self.pos = (self.pos + 2).min(self.bytes.len());
                continue;
            }
            self.pos += 1;
            if c == q {
                if self.peek(0) == q {
                    self.pos += 1;
                    continue;
                }
                return true;
            }
        }
        false
    }

    fn dollar(&mut self) -> (TokenKind, bool) {
        // $1 positional parameter
        if self.peek(1).is_ascii_digit() {
            self.pos += 1;
            while self.peek(0).is_ascii_digit() {
                self.pos += 1;
            }
            return (TokenKind::Parameter, false);
        }
        // $tag$ ... $tag$
        let mut end = self.pos + 1;
        while end < self.bytes.len() && is_ident_char(self.bytes[end]) && self.bytes[end] != b'$' {
            end += 1;
        }
        if end < self.bytes.len() && self.bytes[end] == b'$' {
            let tag = &self.src[self.pos..=end];
            self.pos = end + 1;
            return match self.src[self.pos..].find(tag) {
                Some(i) => {
                    self.pos += i + tag.len();
                    (TokenKind::String, false)
                }
                None => {
                    self.pos = self.bytes.len();
                    (TokenKind::String, true)
                }
            };
        }
        self.pos += 1;
        (TokenKind::Operator, false)
    }

    fn number(&mut self) {
        if self.peek(0) == b'0' && matches!(self.peek(1), b'x' | b'X') && self.peek(2).is_ascii_hexdigit() {
            self.pos += 2;
            while self.peek(0).is_ascii_hexdigit() || self.peek(0) == b'_' {
                self.pos += 1;
            }
            return;
        }
        while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
            self.pos += 1;
        }
        if self.peek(0) == b'.' && self.peek(1) != b'.' {
            self.pos += 1;
            while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
                self.pos += 1;
            }
        }
        if matches!(self.peek(0), b'e' | b'E')
            && (self.peek(1).is_ascii_digit()
                || (matches!(self.peek(1), b'+' | b'-') && self.peek(2).is_ascii_digit()))
        {
            self.pos += 2;
            while self.peek(0).is_ascii_digit() {
                self.pos += 1;
            }
        }
    }

    fn ident_tail(&mut self) {
        while self.pos < self.bytes.len() && is_ident_char(self.bytes[self.pos]) {
            self.pos += 1;
        }
    }

    fn word(&mut self) -> (TokenKind, bool) {
        let start = self.pos;
        let c = self.peek(0);
        // prefixed string literals: E'..' B'..' X'..' N'..' U&'..' _utf8'..'
        if self.peek(1) == b'\'' && matches!(c, b'e' | b'E' | b'b' | b'B' | b'x' | b'X' | b'n' | b'N') {
            self.pos += 1;
            let bs = matches!(c, b'e' | b'E') && self.backend == Backend::Postgres
                || self.backend == Backend::MySql;
            return (TokenKind::String, !self.quoted(b'\'', bs));
        }
        if matches!(c, b'u' | b'U') && self.peek(1) == b'&' && matches!(self.peek(2), b'\'' | b'"') {
            let q = self.peek(2);
            self.pos += 2;
            let closed = self.quoted(q, false);
            let kind = if q == b'\'' { TokenKind::String } else { TokenKind::QuotedIdent };
            return (kind, !closed);
        }
        self.pos += utf8_len(c);
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if is_ident_char(b) {
                self.pos += utf8_len(b);
            } else {
                break;
            }
        }
        self.pos = self.pos.min(self.bytes.len());
        let text = &self.src[start..self.pos];
        (keywords::classify(text, self.backend), false)
    }
}

fn utf8_len(b: u8) -> usize {
    match b {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

pub fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

pub fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

fn is_op_char(b: u8) -> bool {
    matches!(
        b,
        b'+' | b'-' | b'*' | b'/' | b'<' | b'>' | b'=' | b'~' | b'!' | b'@' | b'#' | b'%' | b'^' | b'&' | b'|' | b'`' | b'?' | b':'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str, b: Backend) -> Vec<(TokenKind, &str)> {
        tokenize(src, b)
            .into_iter()
            .filter(|t| t.kind != TokenKind::Whitespace)
            .map(|t| (t.kind, &src[t.start..t.end]))
            .collect()
    }

    #[test]
    fn basic_select() {
        let k = kinds("SELECT a, b FROM t WHERE x = 'it''s';", Backend::Postgres);
        assert_eq!(k[0], (TokenKind::Keyword, "SELECT"));
        assert_eq!(k[1], (TokenKind::Ident, "a"));
        assert_eq!(k[2].0, TokenKind::Comma);
        assert!(k.contains(&(TokenKind::String, "'it''s'")));
        assert_eq!(k.last().unwrap().0, TokenKind::Semicolon);
    }

    #[test]
    fn dollar_quotes_hide_semicolons() {
        let src = "select $fn$ a; b $fn$, $1";
        let k = kinds(src, Backend::Postgres);
        assert_eq!(k[1], (TokenKind::String, "$fn$ a; b $fn$"));
        assert_eq!(k[3], (TokenKind::Parameter, "$1"));
    }

    #[test]
    fn mysql_specifics() {
        let k = kinds("select `a b`, \"str\\\"x\", @v # c\n", Backend::MySql);
        assert_eq!(k[1], (TokenKind::QuotedIdent, "`a b`"));
        assert_eq!(k[3], (TokenKind::String, "\"str\\\"x\""));
        assert_eq!(k[5], (TokenKind::Variable, "@v"));
        assert_eq!(k[6].0, TokenKind::LineComment);
    }

    #[test]
    fn unterminated_string_is_flagged() {
        let t = tokenize("select 'abc", Backend::Sqlite);
        assert!(t.last().unwrap().unterminated);
    }

    #[test]
    fn pg_cast_and_nested_comment() {
        let k = kinds("x::int /* a /* b */ c */ y", Backend::Postgres);
        assert_eq!(k[1], (TokenKind::Operator, "::"));
        assert_eq!(k[2].0, TokenKind::DataType);
        assert_eq!(k[3], (TokenKind::BlockComment, "/* a /* b */ c */"));
        assert_eq!(k[4], (TokenKind::Ident, "y"));
    }

    #[test]
    fn e_string_backslash() {
        let k = kinds(r"select E'a\'b', 1.5e3", Backend::Postgres);
        assert_eq!(k[1], (TokenKind::String, r"E'a\'b'"));
        assert_eq!(k[3], (TokenKind::Number, "1.5e3"));
    }

    #[test]
    fn unicode_identifiers() {
        let k = kinds("select naïve, 名前 from t", Backend::Sqlite);
        assert_eq!(k[1], (TokenKind::Ident, "naïve"));
        assert_eq!(k[3], (TokenKind::Ident, "名前"));
    }

    #[test]
    fn operators_do_not_eat_comments() {
        let k = kinds("a <>-- c\n b", Backend::Postgres);
        assert_eq!(k[1], (TokenKind::Operator, "<>"));
        assert_eq!(k[2].0, TokenKind::LineComment);
    }
}
