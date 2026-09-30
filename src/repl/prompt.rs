use std::borrow::Cow;
use std::time::Duration;

use reedline::{Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, PromptViMode};

use super::style::Palette;

#[derive(Clone, Debug, Default)]
pub struct PromptInfo {
    pub backend: String,
    pub user: String,
    pub host: String,
    pub port: Option<u16>,
    pub database: String,
    pub in_transaction: bool,
    pub readonly: bool,
    pub last_elapsed: Option<Duration>,
    pub last_ok: Option<bool>,
}

pub struct QPrompt {
    left: String,
    indicator: String,
    multiline: String,
}

impl QPrompt {
    /// `format == "auto"` draws the two-line themed prompt; anything else is expanded mycli-style.
    pub fn new(format: &str, continuation: &str, info: &PromptInfo, p: &Palette) -> Self {
        if format.trim().is_empty() || format == "auto" {
            let (left, indicator) = fancy(info, p);
            QPrompt { left, indicator, multiline: p.muted("  · ") }
        } else {
            let expanded = expand(format, info);
            QPrompt { left: p.accent(&expanded), indicator: String::new(), multiline: p.muted(continuation) }
        }
    }
}

fn fancy(info: &PromptInfo, p: &Palette) -> (String, String) {
    let th = &p.theme;
    let mut s = String::new();
    s.push_str(&p.muted("╭─ "));
    s.push_str(&p.paint(p.fg(th.accent).bold(), &info.backend));
    s.push(' ');
    let who = match (info.host.is_empty(), info.port) {
        (true, _) => info.user.clone(),
        (false, Some(port)) => format!("{}@{}:{}", info.user, info.host, port),
        (false, None) => format!("{}@{}", info.user, info.host),
    };
    if !who.is_empty() {
        s.push_str(&p.paint(p.fg(th.accent2), &who));
        s.push(' ');
    }
    if !info.database.is_empty() {
        s.push_str(&p.muted("▸ "));
        s.push_str(&p.paint(p.fg(th.fg).bold(), &info.database));
        s.push(' ');
    }
    if info.in_transaction {
        s.push_str(&p.paint(p.on(th.bg, th.warning).bold(), " TX "));
        s.push(' ');
    }
    if info.readonly {
        s.push_str(&p.paint(p.on(th.bg, th.info).bold(), " RO "));
        s.push(' ');
    }
    match (info.last_ok, info.last_elapsed) {
        (Some(true), Some(d)) => s.push_str(&p.muted(&format!(" ✓ {}", human_duration(d)))),
        (Some(false), Some(d)) => {
            s.push_str(&p.error(" ✗"));
            s.push_str(&p.muted(&format!(" {}", human_duration(d))));
        }
        _ => {}
    }
    s.push('\n');
    s.push_str(&p.muted("╰─"));
    let indicator = p.paint(p.fg(if info.in_transaction { th.warning } else { th.accent }).bold(), "❯ ");
    (s, indicator)
}

pub fn human_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        format!("{:.0} µs", ms * 1000.0)
    } else if ms < 1000.0 {
        format!("{ms:.1} ms")
    } else if ms < 60_000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else {
        let secs = d.as_secs();
        format!("{}m {:02}s", secs / 60, secs % 60)
    }
}

/// mycli/pgcli-compatible escapes: \u user, \h host, \p port, \d database, \t product, \n newline,
/// \T transaction marker, \x read-only marker, \D date-time, \R time, \\ backslash.
pub fn expand(format: &str, info: &PromptInfo) -> String {
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('u') => out.push_str(&info.user),
            Some('h') => out.push_str(&info.host),
            Some('p') => out.push_str(&info.port.map(|p| p.to_string()).unwrap_or_default()),
            Some('d') => out.push_str(&info.database),
            Some('t') => out.push_str(&info.backend),
            Some('n') => out.push('\n'),
            Some('T') => out.push_str(if info.in_transaction { "*" } else { "" }),
            Some('x') => out.push_str(if info.readonly { "(ro)" } else { "" }),
            Some('D') => out.push_str(&chrono::Local::now().format("%a %b %d %H:%M:%S %Y").to_string()),
            Some('R') => out.push_str(&chrono::Local::now().format("%H:%M:%S").to_string()),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

impl Prompt for QPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.left)
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, mode: PromptEditMode) -> Cow<'_, str> {
        match mode {
            PromptEditMode::Vi(PromptViMode::Normal) => Cow::Owned(format!("{}", self.indicator.replace('❯', "❮"))),
            _ => Cow::Borrowed(&self.indicator),
        }
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.multiline)
    }

    fn render_prompt_history_search_indicator(&self, search: PromptHistorySearch) -> Cow<'_, str> {
        let prefix = match search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!("({prefix}history search: {}) ", search.term))
    }

    fn get_prompt_color(&self) -> reedline::Color {
        reedline::Color::Default
    }

    fn get_indicator_color(&self) -> reedline::Color {
        reedline::Color::Default
    }

    fn get_prompt_multiline_color(&self) -> nu_ansi_term::Color {
        nu_ansi_term::Color::Default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> PromptInfo {
        PromptInfo {
            backend: "MySQL".into(),
            user: "root".into(),
            host: "db".into(),
            port: Some(3306),
            database: "shop".into(),
            in_transaction: true,
            readonly: false,
            ..Default::default()
        }
    }

    #[test]
    fn mycli_style_escapes() {
        assert_eq!(expand("\\t \\u@\\h:\\d\\T> ", &info()), "MySQL root@db:shop*> ");
        assert_eq!(expand("\\d\\n> ", &info()), "shop\n> ");
        assert_eq!(expand("a\\\\b\\q", &info()), "a\\b\\q");
    }

    #[test]
    fn durations_are_humane() {
        assert_eq!(human_duration(Duration::from_micros(250)), "250 µs");
        assert_eq!(human_duration(Duration::from_millis(12)), "12.0 ms");
        assert_eq!(human_duration(Duration::from_millis(2500)), "2.50 s");
        assert_eq!(human_duration(Duration::from_secs(125)), "2m 05s");
    }
}
