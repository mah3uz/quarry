use ratatui::style::Color;

/// One palette shared by the REPL (highlighting, prompt, menus, table output) and the TUI.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    pub dark: bool,

    // surfaces
    pub bg: Color,
    /// Panels, sidebar, popups.
    pub surface: Color,
    /// Current line, hovered row, zebra stripe.
    pub highlight: Color,
    pub selection: Color,
    pub fg: Color,
    pub muted: Color,
    pub border: Color,
    pub border_focus: Color,
    pub accent: Color,
    pub accent2: Color,

    // syntax
    pub keyword: Color,
    pub datatype: Color,
    pub function: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub operator: Color,
    pub identifier: Color,
    pub quoted_ident: Color,
    pub parameter: Color,
    pub punctuation: Color,

    // values / semantics
    pub null: Color,
    pub boolean: Color,
    pub temporal: Color,
    pub json: Color,
    pub error: Color,
    pub warning: Color,
    pub success: Color,
    pub info: Color,
    pub header: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Ansi256,
    Ansi16,
    None,
}

impl ColorDepth {
    pub fn detect() -> ColorDepth {
        if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return ColorDepth::None;
        }
        let colorterm = std::env::var("COLORTERM").unwrap_or_default().to_ascii_lowercase();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return ColorDepth::TrueColor;
        }
        let term = std::env::var("TERM").unwrap_or_default();
        if term.contains("256color") || term.contains("kitty") || term.contains("alacritty") || term.contains("ghostty")
            || term.contains("wezterm") || std::env::var_os("WT_SESSION").is_some()
        {
            // modern terminals advertising 256 colors virtually all do truecolor, but stay safe
            return if term.contains("kitty") || term.contains("alacritty") || term.contains("ghostty") || term.contains("wezterm") {
                ColorDepth::TrueColor
            } else {
                ColorDepth::Ansi256
            };
        }
        if term == "dumb" { ColorDepth::None } else { ColorDepth::Ansi16 }
    }
}

pub const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Nearest xterm-256 index for an RGB color.
pub fn to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    let cube = |v: u8| -> u8 {
        if v < 48 { 0 } else if v < 115 { 1 } else { ((v - 35) / 40).min(5) }
    };
    let (cr, cg, cb) = (cube(r), cube(g), cube(b));
    let level = |i: u8| if i == 0 { 0 } else { 55 + i as i32 * 40 };
    let cube_idx = 16 + 36 * cr + 6 * cg + cb;
    let cube_dist = sq(level(cr) - r as i32) + sq(level(cg) - g as i32) + sq(level(cb) - b as i32);
    let avg = (r as i32 + g as i32 + b as i32) / 3;
    let gray_i = if avg > 238 { 23 } else { ((avg - 3).max(0) / 10) as u8 };
    let gray_v = 8 + gray_i as i32 * 10;
    let gray_dist = sq(gray_v - r as i32) + sq(gray_v - g as i32) + sq(gray_v - b as i32);
    if gray_dist < cube_dist { 232 + gray_i } else { cube_idx }
}

fn sq(x: i32) -> i32 {
    x * x
}

/// Nearest of the 16 base ANSI colors (index 0-15).
pub fn to_ansi16(r: u8, g: u8, b: u8) -> u8 {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16), (36, 114, 200), (188, 63, 188), (17, 168, 205), (229, 229, 229),
        (102, 102, 102), (241, 76, 76), (35, 209, 139), (245, 245, 67), (59, 142, 234), (214, 112, 214), (41, 184, 219), (255, 255, 255),
    ];
    BASE.iter()
        .enumerate()
        .min_by_key(|(_, (br, bg, bb))| sq(*br as i32 - r as i32) + sq(*bg as i32 - g as i32) + sq(*bb as i32 - b as i32))
        .map(|(i, _)| i as u8)
        .unwrap_or(7)
}

/// Adapts a color to the terminal's capability.
pub fn adapt(c: Color, depth: ColorDepth) -> Color {
    match (c, depth) {
        (_, ColorDepth::None) => Color::Reset,
        (Color::Rgb(r, g, b), ColorDepth::Ansi256) => Color::Indexed(to_ansi256(r, g, b)),
        (Color::Rgb(r, g, b), ColorDepth::Ansi16) => Color::Indexed(to_ansi16(r, g, b)),
        (c, _) => c,
    }
}

/// ANSI SGR foreground escape for `c` at the given depth ("" for None/Reset).
pub fn ansi_fg(c: Color, depth: ColorDepth) -> String {
    sgr(c, depth, false)
}

pub fn ansi_bg(c: Color, depth: ColorDepth) -> String {
    sgr(c, depth, true)
}

fn sgr(c: Color, depth: ColorDepth, bg: bool) -> String {
    let base = if bg { 48 } else { 38 };
    match adapt(c, depth) {
        Color::Reset => String::new(),
        Color::Rgb(r, g, b) => format!("\x1b[{base};2;{r};{g};{b}m"),
        Color::Indexed(i) if depth == ColorDepth::Ansi16 && i < 16 => {
            let code = if i < 8 { 30 + i as u16 } else { 90 + (i as u16 - 8) } + if bg { 10 } else { 0 };
            format!("\x1b[{code}m")
        }
        Color::Indexed(i) => format!("\x1b[{base};5;{i}m"),
        other => {
            let i = named_index(other);
            let code = if i < 8 { 30 + i as u16 } else { 90 + (i as u16 - 8) } + if bg { 10 } else { 0 };
            format!("\x1b[{code}m")
        }
    }
}

fn named_index(c: Color) -> u8 {
    match c {
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        _ => 7,
    }
}

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const ITALIC: &str = "\x1b[3m";

impl Theme {
    pub fn tokyo_night() -> Theme {
        Theme {
            name: "tokyo-night".into(),
            dark: true,
            bg: rgb(0x1a1b26),
            surface: rgb(0x16161e),
            highlight: rgb(0x292e42),
            selection: rgb(0x33467c),
            fg: rgb(0xc0caf5),
            muted: rgb(0x565f89),
            border: rgb(0x3b4261),
            border_focus: rgb(0x7aa2f7),
            accent: rgb(0x7aa2f7),
            accent2: rgb(0xbb9af7),
            keyword: rgb(0xbb9af7),
            datatype: rgb(0x2ac3de),
            function: rgb(0x7aa2f7),
            string: rgb(0x9ece6a),
            number: rgb(0xff9e64),
            comment: rgb(0x565f89),
            operator: rgb(0x89ddff),
            identifier: rgb(0xc0caf5),
            quoted_ident: rgb(0x73daca),
            parameter: rgb(0xe0af68),
            punctuation: rgb(0x9aa5ce),
            null: rgb(0x565f89),
            boolean: rgb(0xff9e64),
            temporal: rgb(0x7dcfff),
            json: rgb(0x73daca),
            error: rgb(0xf7768e),
            warning: rgb(0xe0af68),
            success: rgb(0x9ece6a),
            info: rgb(0x7dcfff),
            header: rgb(0x7aa2f7),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::tokyo_night()
    }
}
