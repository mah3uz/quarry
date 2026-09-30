use nu_ansi_term::Style;
use reedline::StyledText;

use super::style::Palette;
use crate::db::Backend;
use crate::sql::lexer::{Token, TokenKind, tokenize};

pub fn highlight(line: &str, cursor: usize, backend: Backend, p: &Palette) -> StyledText {
    let mut out = StyledText::new();
    let trimmed = line.trim_start();
    let lead = line.len() - trimmed.len();
    if is_command_line(trimmed, backend) {
        let cmd_end = trimmed.find(char::is_whitespace).map(|i| lead + i).unwrap_or(line.len());
        out.push((Style::new(), line[..lead].to_string()));
        out.push((p.fg(p.theme.accent2).bold(), line[lead..cmd_end].to_string()));
        out.push((p.fg(p.theme.string), line[cmd_end..].to_string()));
        return out;
    }

    let tokens = tokenize(line, backend);
    let brackets = matching_brackets(&tokens, cursor);
    for (i, t) in tokens.iter().enumerate() {
        let text = t.text(line);
        let mut style = token_style(line, &tokens, i, backend, p);
        if brackets.is_some_and(|(a, b)| a == i || b == i) {
            style = style.bold().underline();
        }
        out.push((style, text.to_string()));
    }
    out
}

fn is_command_line(trimmed: &str, backend: Backend) -> bool {
    trimmed.starts_with('\\') || (backend == Backend::Sqlite && trimmed.starts_with('.') && trimmed.len() > 1)
}

fn token_style(line: &str, tokens: &[Token], i: usize, backend: Backend, p: &Palette) -> Style {
    let t = &tokens[i];
    let th = &p.theme;
    let followed_by_paren = tokens[i + 1..]
        .iter()
        .find(|t| t.kind != TokenKind::Whitespace)
        .is_some_and(|n| n.kind == TokenKind::LParen);
    match t.kind {
        TokenKind::Keyword if followed_by_paren && is_function_name(t.text(line), backend) => p.fg(th.function),
        TokenKind::Keyword => p.fg(th.keyword).bold(),
        TokenKind::DataType => p.fg(th.datatype),
        TokenKind::Builtin => p.fg(th.function),
        TokenKind::Ident if followed_by_paren => p.fg(th.function),
        TokenKind::Ident => p.fg(th.identifier),
        TokenKind::QuotedIdent => p.fg(th.quoted_ident),
        TokenKind::String if t.unterminated => p.fg(th.string).underline(),
        TokenKind::String => p.fg(th.string),
        TokenKind::Number => p.fg(th.number),
        TokenKind::LineComment | TokenKind::BlockComment => p.fg(th.comment).italic(),
        TokenKind::Operator => p.fg(th.operator),
        TokenKind::Parameter | TokenKind::Variable => p.fg(th.parameter),
        TokenKind::Comma | TokenKind::Dot | TokenKind::Semicolon | TokenKind::LParen | TokenKind::RParen
        | TokenKind::LBracket | TokenKind::RBracket => p.fg(th.punctuation),
        TokenKind::Backslash => p.fg(th.accent2).bold(),
        TokenKind::Whitespace | TokenKind::Unknown => Style::new(),
    }
}

/// Keywords that double as functions (`LEFT(`, `REPLACE(`, `IF(`); clause keywords like `IN (` stay keywords.
fn is_function_name(word: &str, backend: Backend) -> bool {
    let up = word.to_ascii_uppercase();
    crate::sql::keywords::functions(backend).binary_search(&up.as_str()).is_ok()
}

/// Token indices of the bracket adjacent to the cursor and its partner.
fn matching_brackets(tokens: &[Token], cursor: usize) -> Option<(usize, usize)> {
    let at = tokens.iter().position(|t| {
        matches!(t.kind, TokenKind::LParen | TokenKind::RParen) && (t.start == cursor || t.end == cursor)
    })?;
    let forward = tokens[at].kind == TokenKind::LParen;
    let mut depth = 0i32;
    let range: Box<dyn Iterator<Item = usize>> =
        if forward { Box::new(at..tokens.len()) } else { Box::new((0..=at).rev()) };
    for j in range {
        match tokens[j].kind {
            TokenKind::LParen => depth += if forward { 1 } else { -1 },
            TokenKind::RParen => depth += if forward { -1 } else { 1 },
            _ => continue,
        }
        if depth == 0 {
            return Some((at, j));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ColorDepth, Theme};

    fn palette() -> Palette {
        Palette::new(Theme::default(), ColorDepth::TrueColor)
    }

    #[test]
    fn highlighting_preserves_text_exactly() {
        let src = "SELECT count(*), 'x' -- c\nFROM t WHERE id = $1";
        let st = highlight(src, 0, Backend::Postgres, &palette());
        let joined: String = st.buffer.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(joined, src);
    }

    #[test]
    fn keywords_and_strings_get_theme_colors() {
        let p = palette();
        let st = highlight("select 'a'", 0, Backend::MySql, &p);
        assert_eq!(st.buffer[0].0, p.fg(p.theme.keyword).bold());
        assert_eq!(st.buffer[2].0, p.fg(p.theme.string));
    }

    #[test]
    fn brackets_under_cursor_are_emphasised() {
        let st = highlight("f((a))", 1, Backend::Sqlite, &palette());
        let underlined: Vec<&str> =
            st.buffer.iter().filter(|(s, _)| s.is_underline).map(|(_, t)| t.as_str()).collect();
        assert_eq!(underlined, vec!["(", ")"]);
    }

    #[test]
    fn special_commands_render_as_command() {
        let p = palette();
        let st = highlight("\\dt users", 0, Backend::Postgres, &p);
        assert_eq!(st.buffer[1].1, "\\dt");
        assert_eq!(st.buffer[1].0, p.fg(p.theme.accent2).bold());
    }
}
