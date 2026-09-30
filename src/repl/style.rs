use nu_ansi_term::{Color as AnsiColor, Style};
use ratatui::style::Color;

use crate::theme::{ColorDepth, Theme, adapt};

pub fn ansi(c: Color, depth: ColorDepth) -> AnsiColor {
    match adapt(c, depth) {
        Color::Rgb(r, g, b) => AnsiColor::Rgb(r, g, b),
        Color::Indexed(i) => AnsiColor::Fixed(i),
        Color::Black => AnsiColor::Black,
        Color::Red => AnsiColor::Red,
        Color::Green => AnsiColor::Green,
        Color::Yellow => AnsiColor::Yellow,
        Color::Blue => AnsiColor::Blue,
        Color::Magenta => AnsiColor::Purple,
        Color::Cyan => AnsiColor::Cyan,
        Color::Gray => AnsiColor::LightGray,
        Color::DarkGray => AnsiColor::DarkGray,
        Color::LightRed => AnsiColor::LightRed,
        Color::LightGreen => AnsiColor::LightGreen,
        Color::LightYellow => AnsiColor::LightYellow,
        Color::LightBlue => AnsiColor::LightBlue,
        Color::LightMagenta => AnsiColor::LightPurple,
        Color::LightCyan => AnsiColor::LightCyan,
        Color::White => AnsiColor::White,
        _ => AnsiColor::Default,
    }
}

/// Styles derived from the theme for everything reedline paints.
#[derive(Clone)]
pub struct Palette {
    pub depth: ColorDepth,
    pub theme: Theme,
}

impl Palette {
    pub fn new(theme: Theme, depth: ColorDepth) -> Self {
        Palette { depth, theme }
    }

    pub fn fg(&self, c: Color) -> Style {
        if self.depth == ColorDepth::None {
            Style::new()
        } else {
            Style::new().fg(ansi(c, self.depth))
        }
    }

    pub fn on(&self, fg: Color, bg: Color) -> Style {
        if self.depth == ColorDepth::None {
            Style::new()
        } else {
            Style::new().fg(ansi(fg, self.depth)).on(ansi(bg, self.depth))
        }
    }

    /// Paints `text` with an SGR sequence (for prompt strings and messages).
    pub fn paint(&self, style: Style, text: &str) -> String {
        if self.depth == ColorDepth::None {
            text.to_string()
        } else {
            style.paint(text).to_string()
        }
    }

    pub fn error(&self, text: &str) -> String {
        self.paint(self.fg(self.theme.error).bold(), text)
    }

    pub fn warning(&self, text: &str) -> String {
        self.paint(self.fg(self.theme.warning), text)
    }

    pub fn success(&self, text: &str) -> String {
        self.paint(self.fg(self.theme.success), text)
    }

    pub fn muted(&self, text: &str) -> String {
        self.paint(self.fg(self.theme.muted), text)
    }

    pub fn accent(&self, text: &str) -> String {
        self.paint(self.fg(self.theme.accent).bold(), text)
    }

    pub fn info(&self, text: &str) -> String {
        self.paint(self.fg(self.theme.info), text)
    }
}
