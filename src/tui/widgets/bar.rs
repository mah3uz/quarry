use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::icons;

/// A lualine-style segment: `text` on `bg`, with rounded caps when the icon set has them.
pub struct Pill<'a> {
    pub text: &'a str,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
}

impl Pill<'_> {
    fn ends() -> (&'static str, &'static str) {
        let ic = icons::get();
        if ic.cap_left.is_empty() { (" ", " ") } else { (ic.cap_left, ic.cap_right) }
    }

    pub fn width(&self) -> u16 {
        let (l, r) = Self::ends();
        (l.width() + self.text.width() + r.width()) as u16
    }

    /// Draws at `x` on a bar whose background is `bar_bg`, clipped at `limit`; returns the area drawn.
    pub fn draw(&self, buf: &mut Buffer, x: u16, y: u16, bar_bg: Color, limit: u16) -> Rect {
        let (l, r) = Self::ends();
        let cap = Style::default().fg(self.bg).bg(bar_bg);
        let mut body = Style::default().fg(self.fg).bg(self.bg);
        if self.bold {
            body = body.add_modifier(Modifier::BOLD);
        }
        let room = |at: u16| limit.saturating_sub(at) as usize;
        let mut cx = x;
        for (text, style) in [(l, if l == " " { body } else { cap }), (self.text, body), (r, if r == " " { body } else { cap })] {
            if room(cx) == 0 {
                break;
            }
            let (end, _) = buf.set_stringn(cx, y, text, room(cx), style);
            cx = end;
        }
        Rect { x, y, width: cx - x, height: 1 }
    }
}
