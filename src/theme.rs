use std::path::Path;
use std::str::FromStr;

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

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Per-character colours for `len` characters with a band of `peak` sweeping over `base`;
/// each `tick` moves the band one character. Colours that aren't RGB switch instead of blending.
pub fn shimmer(len: usize, tick: usize, base: Color, peak: Color) -> Vec<Color> {
    const HALF: f32 = 4.0;
    let span = len + 2 * HALF as usize;
    let center = (tick % span.max(1)) as f32 - HALF;
    (0..len)
        .map(|i| {
            let t = (1.0 - (i as f32 - center).abs() / HALF).max(0.0);
            match (base, peak) {
                (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
                    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
                    Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
                }
                _ if t >= 0.5 => peak,
                _ => base,
            }
        })
        .collect()
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
    /// A copy with every color reduced to what the terminal can show.
    pub fn adapted(&self, depth: ColorDepth) -> Theme {
        let mut t = self.clone();
        for (_, c) in t.colors_mut() {
            *c = adapt(*c, depth);
        }
        t
    }

    fn colors_mut(&mut self) -> [(&'static str, &mut Color); 30] {
        [
            ("bg", &mut self.bg), ("surface", &mut self.surface), ("highlight", &mut self.highlight),
            ("selection", &mut self.selection), ("fg", &mut self.fg), ("muted", &mut self.muted),
            ("border", &mut self.border), ("border_focus", &mut self.border_focus), ("accent", &mut self.accent),
            ("accent2", &mut self.accent2), ("keyword", &mut self.keyword), ("datatype", &mut self.datatype),
            ("function", &mut self.function), ("string", &mut self.string), ("number", &mut self.number),
            ("comment", &mut self.comment), ("operator", &mut self.operator), ("identifier", &mut self.identifier),
            ("quoted_ident", &mut self.quoted_ident), ("parameter", &mut self.parameter),
            ("punctuation", &mut self.punctuation), ("null", &mut self.null), ("boolean", &mut self.boolean),
            ("temporal", &mut self.temporal), ("json", &mut self.json), ("error", &mut self.error),
            ("warning", &mut self.warning), ("success", &mut self.success), ("info", &mut self.info),
            ("header", &mut self.header),
        ]
    }

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

const BUILTIN_NAMES: &[&str] = &[
    "tokyo-night", "tokyo-night-storm", "tokyo-night-day",
    "catppuccin-mocha", "catppuccin-macchiato", "catppuccin-frappe", "catppuccin-latte",
    "gruvbox-dark", "gruvbox-light", "dracula", "nord", "one-dark", "solarized-dark", "solarized-light",
    "rose-pine", "rose-pine-moon", "rose-pine-dawn", "kanagawa", "everforest-dark", "everforest-light",
    "github-dark", "github-light", "monokai", "ayu-dark", "nightfox", "material-ocean", "ansi",
];

/// Family names that resolve to their default variant.
const ALIASES: &[(&str, &str)] = &[
    ("tokyonight", "tokyo-night"), ("catppuccin", "catppuccin-mocha"), ("gruvbox", "gruvbox-dark"),
    ("solarized", "solarized-dark"), ("rosepinemain", "rose-pine"), ("everforest", "everforest-dark"),
    ("github", "github-dark"), ("ayu", "ayu-dark"), ("kanagawawave", "kanagawa"), ("material", "material-ocean"),
];

pub fn builtin_names() -> &'static [&'static str] {
    BUILTIN_NAMES
}

/// Squashes case, separators and accents so `Tokyo_Night`, `tokyo night`, `tokyonight` and `Rosé Pine` all match.
fn squash(name: &str) -> String {
    name.chars()
        .map(|c| if c == 'é' || c == 'É' { 'e' } else { c.to_ascii_lowercase() })
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

fn canonical_builtin(name: &str) -> Option<&'static str> {
    let key = squash(name);
    BUILTIN_NAMES
        .iter()
        .copied()
        .find(|n| squash(n) == key)
        .or_else(|| ALIASES.iter().find(|(a, _)| *a == key).map(|(_, n)| *n))
}

pub fn builtin(name: &str) -> Option<Theme> {
    let name = canonical_builtin(name)?;
    let n = name.to_string();
    Some(match name {
        "tokyo-night" => Theme::tokyo_night(),
        "tokyo-night-storm" => Theme {
            name: n,
            bg: rgb(0x24283b), surface: rgb(0x1f2335), selection: rgb(0x2e3c64),
            ..Theme::tokyo_night()
        },
        "tokyo-night-day" => Theme {
            name: n, dark: false,
            bg: rgb(0xe1e2e7), surface: rgb(0xd0d5e3), highlight: rgb(0xc4c8da), selection: rgb(0xb7c1e3),
            fg: rgb(0x3760bf), muted: rgb(0x848cb5), border: rgb(0xa8aecb), border_focus: rgb(0x2e7de9),
            accent: rgb(0x2e7de9), accent2: rgb(0x9854f1),
            keyword: rgb(0x9854f1), datatype: rgb(0x188092), function: rgb(0x2e7de9), string: rgb(0x587539),
            number: rgb(0xb15c00), comment: rgb(0x848cb5), operator: rgb(0x006a83), identifier: rgb(0x3760bf),
            quoted_ident: rgb(0x387068), parameter: rgb(0x8c6c3e), punctuation: rgb(0x6172b0),
            null: rgb(0x848cb5), boolean: rgb(0xb15c00), temporal: rgb(0x007197), json: rgb(0x387068),
            error: rgb(0xf52a65), warning: rgb(0x8c6c3e), success: rgb(0x587539), info: rgb(0x007197), header: rgb(0x2e7de9),
        },
        "catppuccin-mocha" => Theme {
            name: n, dark: true,
            bg: rgb(0x1e1e2e), surface: rgb(0x181825), highlight: rgb(0x313244), selection: rgb(0x45475a),
            fg: rgb(0xcdd6f4), muted: rgb(0x7f849c), border: rgb(0x45475a), border_focus: rgb(0xb4befe),
            accent: rgb(0xcba6f7), accent2: rgb(0x89b4fa),
            keyword: rgb(0xcba6f7), datatype: rgb(0xf9e2af), function: rgb(0x89b4fa), string: rgb(0xa6e3a1),
            number: rgb(0xfab387), comment: rgb(0x9399b2), operator: rgb(0x89dceb), identifier: rgb(0xcdd6f4),
            quoted_ident: rgb(0xb4befe), parameter: rgb(0xeba0ac), punctuation: rgb(0x9399b2),
            null: rgb(0x7f849c), boolean: rgb(0xfab387), temporal: rgb(0x74c7ec), json: rgb(0x94e2d5),
            error: rgb(0xf38ba8), warning: rgb(0xf9e2af), success: rgb(0xa6e3a1), info: rgb(0x89dceb), header: rgb(0x89b4fa),
        },
        "catppuccin-macchiato" => Theme {
            name: n, dark: true,
            bg: rgb(0x24273a), surface: rgb(0x1e2030), highlight: rgb(0x363a4f), selection: rgb(0x494d64),
            fg: rgb(0xcad3f5), muted: rgb(0x8087a2), border: rgb(0x494d64), border_focus: rgb(0xb7bdf8),
            accent: rgb(0xc6a0f6), accent2: rgb(0x8aadf4),
            keyword: rgb(0xc6a0f6), datatype: rgb(0xeed49f), function: rgb(0x8aadf4), string: rgb(0xa6da95),
            number: rgb(0xf5a97f), comment: rgb(0x939ab7), operator: rgb(0x91d7e3), identifier: rgb(0xcad3f5),
            quoted_ident: rgb(0xb7bdf8), parameter: rgb(0xee99a0), punctuation: rgb(0x939ab7),
            null: rgb(0x8087a2), boolean: rgb(0xf5a97f), temporal: rgb(0x7dc4e4), json: rgb(0x8bd5ca),
            error: rgb(0xed8796), warning: rgb(0xeed49f), success: rgb(0xa6da95), info: rgb(0x91d7e3), header: rgb(0x8aadf4),
        },
        "catppuccin-frappe" => Theme {
            name: n, dark: true,
            bg: rgb(0x303446), surface: rgb(0x292c3c), highlight: rgb(0x414559), selection: rgb(0x51576d),
            fg: rgb(0xc6d0f5), muted: rgb(0x838ba7), border: rgb(0x51576d), border_focus: rgb(0xbabbf1),
            accent: rgb(0xca9ee6), accent2: rgb(0x8caaee),
            keyword: rgb(0xca9ee6), datatype: rgb(0xe5c890), function: rgb(0x8caaee), string: rgb(0xa6d189),
            number: rgb(0xef9f76), comment: rgb(0x949cbb), operator: rgb(0x99d1db), identifier: rgb(0xc6d0f5),
            quoted_ident: rgb(0xbabbf1), parameter: rgb(0xea999c), punctuation: rgb(0x949cbb),
            null: rgb(0x838ba7), boolean: rgb(0xef9f76), temporal: rgb(0x85c1dc), json: rgb(0x81c8be),
            error: rgb(0xe78284), warning: rgb(0xe5c890), success: rgb(0xa6d189), info: rgb(0x99d1db), header: rgb(0x8caaee),
        },
        "catppuccin-latte" => Theme {
            name: n, dark: false,
            bg: rgb(0xeff1f5), surface: rgb(0xe6e9ef), highlight: rgb(0xccd0da), selection: rgb(0xbcc0cc),
            fg: rgb(0x4c4f69), muted: rgb(0x8c8fa1), border: rgb(0xbcc0cc), border_focus: rgb(0x7287fd),
            accent: rgb(0x8839ef), accent2: rgb(0x1e66f5),
            keyword: rgb(0x8839ef), datatype: rgb(0xdf8e1d), function: rgb(0x1e66f5), string: rgb(0x40a02b),
            number: rgb(0xfe640b), comment: rgb(0x7c7f93), operator: rgb(0x04a5e5), identifier: rgb(0x4c4f69),
            quoted_ident: rgb(0x7287fd), parameter: rgb(0xe64553), punctuation: rgb(0x7c7f93),
            null: rgb(0x8c8fa1), boolean: rgb(0xfe640b), temporal: rgb(0x209fb5), json: rgb(0x179299),
            error: rgb(0xd20f39), warning: rgb(0xdf8e1d), success: rgb(0x40a02b), info: rgb(0x04a5e5), header: rgb(0x1e66f5),
        },
        "gruvbox-dark" => Theme {
            name: n, dark: true,
            bg: rgb(0x282828), surface: rgb(0x1d2021), highlight: rgb(0x3c3836), selection: rgb(0x504945),
            fg: rgb(0xebdbb2), muted: rgb(0x928374), border: rgb(0x665c54), border_focus: rgb(0xfabd2f),
            accent: rgb(0xfabd2f), accent2: rgb(0xfe8019),
            keyword: rgb(0xfb4934), datatype: rgb(0xfabd2f), function: rgb(0x8ec07c), string: rgb(0xb8bb26),
            number: rgb(0xd3869b), comment: rgb(0x928374), operator: rgb(0xfe8019), identifier: rgb(0xebdbb2),
            quoted_ident: rgb(0x83a598), parameter: rgb(0x83a598), punctuation: rgb(0xa89984),
            null: rgb(0x928374), boolean: rgb(0xd3869b), temporal: rgb(0x83a598), json: rgb(0x8ec07c),
            error: rgb(0xfb4934), warning: rgb(0xfabd2f), success: rgb(0xb8bb26), info: rgb(0x83a598), header: rgb(0xfabd2f),
        },
        "gruvbox-light" => Theme {
            name: n, dark: false,
            bg: rgb(0xfbf1c7), surface: rgb(0xf2e5bc), highlight: rgb(0xebdbb2), selection: rgb(0xd5c4a1),
            fg: rgb(0x3c3836), muted: rgb(0x928374), border: rgb(0xbdae93), border_focus: rgb(0xb57614),
            accent: rgb(0xb57614), accent2: rgb(0xaf3a03),
            keyword: rgb(0x9d0006), datatype: rgb(0xb57614), function: rgb(0x427b58), string: rgb(0x79740e),
            number: rgb(0x8f3f71), comment: rgb(0x928374), operator: rgb(0xaf3a03), identifier: rgb(0x3c3836),
            quoted_ident: rgb(0x076678), parameter: rgb(0x076678), punctuation: rgb(0x7c6f64),
            null: rgb(0x928374), boolean: rgb(0x8f3f71), temporal: rgb(0x076678), json: rgb(0x427b58),
            error: rgb(0x9d0006), warning: rgb(0xb57614), success: rgb(0x79740e), info: rgb(0x076678), header: rgb(0xb57614),
        },
        "dracula" => Theme {
            name: n, dark: true,
            bg: rgb(0x282a36), surface: rgb(0x21222c), highlight: rgb(0x343746), selection: rgb(0x44475a),
            fg: rgb(0xf8f8f2), muted: rgb(0x6272a4), border: rgb(0x44475a), border_focus: rgb(0xbd93f9),
            accent: rgb(0xbd93f9), accent2: rgb(0xff79c6),
            keyword: rgb(0xff79c6), datatype: rgb(0x8be9fd), function: rgb(0x50fa7b), string: rgb(0xf1fa8c),
            number: rgb(0xbd93f9), comment: rgb(0x6272a4), operator: rgb(0xff79c6), identifier: rgb(0xf8f8f2),
            quoted_ident: rgb(0x8be9fd), parameter: rgb(0xffb86c), punctuation: rgb(0xf8f8f2),
            null: rgb(0x6272a4), boolean: rgb(0xbd93f9), temporal: rgb(0x8be9fd), json: rgb(0xffb86c),
            error: rgb(0xff5555), warning: rgb(0xffb86c), success: rgb(0x50fa7b), info: rgb(0x8be9fd), header: rgb(0xbd93f9),
        },
        "nord" => Theme {
            name: n, dark: true,
            bg: rgb(0x2e3440), surface: rgb(0x3b4252), highlight: rgb(0x3b4252), selection: rgb(0x434c5e),
            fg: rgb(0xd8dee9), muted: rgb(0x616e88), border: rgb(0x4c566a), border_focus: rgb(0x88c0d0),
            accent: rgb(0x88c0d0), accent2: rgb(0x81a1c1),
            keyword: rgb(0x81a1c1), datatype: rgb(0x8fbcbb), function: rgb(0x88c0d0), string: rgb(0xa3be8c),
            number: rgb(0xb48ead), comment: rgb(0x616e88), operator: rgb(0x81a1c1), identifier: rgb(0xd8dee9),
            quoted_ident: rgb(0x8fbcbb), parameter: rgb(0xd08770), punctuation: rgb(0xeceff4),
            null: rgb(0x616e88), boolean: rgb(0x81a1c1), temporal: rgb(0xebcb8b), json: rgb(0x8fbcbb),
            error: rgb(0xbf616a), warning: rgb(0xebcb8b), success: rgb(0xa3be8c), info: rgb(0x88c0d0), header: rgb(0x88c0d0),
        },
        "one-dark" => Theme {
            name: n, dark: true,
            bg: rgb(0x282c34), surface: rgb(0x21252b), highlight: rgb(0x2c313c), selection: rgb(0x3e4451),
            fg: rgb(0xabb2bf), muted: rgb(0x5c6370), border: rgb(0x3e4451), border_focus: rgb(0x61afef),
            accent: rgb(0x61afef), accent2: rgb(0xc678dd),
            keyword: rgb(0xc678dd), datatype: rgb(0xe5c07b), function: rgb(0x61afef), string: rgb(0x98c379),
            number: rgb(0xd19a66), comment: rgb(0x5c6370), operator: rgb(0x56b6c2), identifier: rgb(0xabb2bf),
            quoted_ident: rgb(0xe06c75), parameter: rgb(0xd19a66), punctuation: rgb(0xabb2bf),
            null: rgb(0x5c6370), boolean: rgb(0xd19a66), temporal: rgb(0x56b6c2), json: rgb(0xe5c07b),
            error: rgb(0xe06c75), warning: rgb(0xe5c07b), success: rgb(0x98c379), info: rgb(0x56b6c2), header: rgb(0x61afef),
        },
        "solarized-dark" => Theme {
            name: n, dark: true,
            bg: rgb(0x002b36), surface: rgb(0x073642), highlight: rgb(0x073642), selection: rgb(0x274642),
            fg: rgb(0x839496), muted: rgb(0x586e75), border: rgb(0x586e75), border_focus: rgb(0x268bd2),
            accent: rgb(0x268bd2), accent2: rgb(0xd33682),
            keyword: rgb(0x859900), datatype: rgb(0xb58900), function: rgb(0x268bd2), string: rgb(0x2aa198),
            number: rgb(0xd33682), comment: rgb(0x586e75), operator: rgb(0x859900), identifier: rgb(0x839496),
            quoted_ident: rgb(0x268bd2), parameter: rgb(0xcb4b16), punctuation: rgb(0x657b83),
            null: rgb(0x586e75), boolean: rgb(0x6c71c4), temporal: rgb(0xb58900), json: rgb(0x2aa198),
            error: rgb(0xdc322f), warning: rgb(0xb58900), success: rgb(0x859900), info: rgb(0x2aa198), header: rgb(0x268bd2),
        },
        // fg is base01 rather than base00: base00 on base3 is only 4.1:1.
        "solarized-light" => Theme {
            name: n, dark: false,
            bg: rgb(0xfdf6e3), surface: rgb(0xeee8d5), highlight: rgb(0xeee8d5), selection: rgb(0xddd6c1),
            fg: rgb(0x586e75), muted: rgb(0x93a1a1), border: rgb(0x93a1a1), border_focus: rgb(0x268bd2),
            accent: rgb(0x268bd2), accent2: rgb(0xd33682),
            keyword: rgb(0x859900), datatype: rgb(0xb58900), function: rgb(0x268bd2), string: rgb(0x2aa198),
            number: rgb(0xd33682), comment: rgb(0x93a1a1), operator: rgb(0x859900), identifier: rgb(0x586e75),
            quoted_ident: rgb(0x268bd2), parameter: rgb(0xcb4b16), punctuation: rgb(0x657b83),
            null: rgb(0x93a1a1), boolean: rgb(0x6c71c4), temporal: rgb(0xb58900), json: rgb(0x2aa198),
            error: rgb(0xdc322f), warning: rgb(0xb58900), success: rgb(0x859900), info: rgb(0x2aa198), header: rgb(0x268bd2),
        },
        "rose-pine" => Theme {
            name: n, dark: true,
            bg: rgb(0x191724), surface: rgb(0x1f1d2e), highlight: rgb(0x21202e), selection: rgb(0x403d52),
            fg: rgb(0xe0def4), muted: rgb(0x6e6a86), border: rgb(0x524f67), border_focus: rgb(0xebbcba),
            accent: rgb(0xebbcba), accent2: rgb(0xc4a7e7),
            keyword: rgb(0x31748f), datatype: rgb(0x9ccfd8), function: rgb(0xeb6f92), string: rgb(0xf6c177),
            number: rgb(0xebbcba), comment: rgb(0x6e6a86), operator: rgb(0x908caa), identifier: rgb(0xe0def4),
            quoted_ident: rgb(0x9ccfd8), parameter: rgb(0xc4a7e7), punctuation: rgb(0x908caa),
            null: rgb(0x6e6a86), boolean: rgb(0xebbcba), temporal: rgb(0x9ccfd8), json: rgb(0xf6c177),
            error: rgb(0xeb6f92), warning: rgb(0xf6c177), success: rgb(0x9ccfd8), info: rgb(0xc4a7e7), header: rgb(0xebbcba),
        },
        "rose-pine-moon" => Theme {
            name: n, dark: true,
            bg: rgb(0x232136), surface: rgb(0x2a273f), highlight: rgb(0x2a283e), selection: rgb(0x44415a),
            fg: rgb(0xe0def4), muted: rgb(0x6e6a86), border: rgb(0x56526e), border_focus: rgb(0xea9a97),
            accent: rgb(0xea9a97), accent2: rgb(0xc4a7e7),
            keyword: rgb(0x3e8fb0), datatype: rgb(0x9ccfd8), function: rgb(0xeb6f92), string: rgb(0xf6c177),
            number: rgb(0xea9a97), comment: rgb(0x6e6a86), operator: rgb(0x908caa), identifier: rgb(0xe0def4),
            quoted_ident: rgb(0x9ccfd8), parameter: rgb(0xc4a7e7), punctuation: rgb(0x908caa),
            null: rgb(0x6e6a86), boolean: rgb(0xea9a97), temporal: rgb(0x9ccfd8), json: rgb(0xf6c177),
            error: rgb(0xeb6f92), warning: rgb(0xf6c177), success: rgb(0x9ccfd8), info: rgb(0xc4a7e7), header: rgb(0xea9a97),
        },
        "rose-pine-dawn" => Theme {
            name: n, dark: false,
            bg: rgb(0xfaf4ed), surface: rgb(0xfffaf3), highlight: rgb(0xf4ede8), selection: rgb(0xdfdad9),
            fg: rgb(0x575279), muted: rgb(0x9893a5), border: rgb(0xcecacd), border_focus: rgb(0xd7827e),
            accent: rgb(0xd7827e), accent2: rgb(0x907aa9),
            keyword: rgb(0x286983), datatype: rgb(0x56949f), function: rgb(0xb4637a), string: rgb(0xea9d34),
            number: rgb(0xd7827e), comment: rgb(0x9893a5), operator: rgb(0x797593), identifier: rgb(0x575279),
            quoted_ident: rgb(0x56949f), parameter: rgb(0x907aa9), punctuation: rgb(0x797593),
            null: rgb(0x9893a5), boolean: rgb(0xd7827e), temporal: rgb(0x56949f), json: rgb(0xea9d34),
            error: rgb(0xb4637a), warning: rgb(0xea9d34), success: rgb(0x56949f), info: rgb(0x907aa9), header: rgb(0xd7827e),
        },
        "kanagawa" => Theme {
            name: n, dark: true,
            bg: rgb(0x1f1f28), surface: rgb(0x16161d), highlight: rgb(0x2a2a37), selection: rgb(0x223249),
            fg: rgb(0xdcd7ba), muted: rgb(0x727169), border: rgb(0x54546d), border_focus: rgb(0x7e9cd8),
            accent: rgb(0x7e9cd8), accent2: rgb(0x957fb8),
            keyword: rgb(0x957fb8), datatype: rgb(0x7aa89f), function: rgb(0x7e9cd8), string: rgb(0x98bb6c),
            number: rgb(0xd27e99), comment: rgb(0x727169), operator: rgb(0xc0a36e), identifier: rgb(0xdcd7ba),
            quoted_ident: rgb(0xe6c384), parameter: rgb(0xb8b4d0), punctuation: rgb(0x9cabca),
            null: rgb(0x727169), boolean: rgb(0xffa066), temporal: rgb(0x7fb4ca), json: rgb(0xe6c384),
            error: rgb(0xe82424), warning: rgb(0xff9e3b), success: rgb(0x98bb6c), info: rgb(0x658594), header: rgb(0x7e9cd8),
        },
        "everforest-dark" => Theme {
            name: n, dark: true,
            bg: rgb(0x2d353b), surface: rgb(0x232a2e), highlight: rgb(0x343f44), selection: rgb(0x543a48),
            fg: rgb(0xd3c6aa), muted: rgb(0x859289), border: rgb(0x475258), border_focus: rgb(0xa7c080),
            accent: rgb(0xa7c080), accent2: rgb(0x83c092),
            keyword: rgb(0xe67e80), datatype: rgb(0xdbbc7f), function: rgb(0xa7c080), string: rgb(0x83c092),
            number: rgb(0xd699b6), comment: rgb(0x859289), operator: rgb(0xe69875), identifier: rgb(0xd3c6aa),
            quoted_ident: rgb(0x7fbbb3), parameter: rgb(0x7fbbb3), punctuation: rgb(0x9da9a0),
            null: rgb(0x859289), boolean: rgb(0xd699b6), temporal: rgb(0x7fbbb3), json: rgb(0x83c092),
            error: rgb(0xe67e80), warning: rgb(0xdbbc7f), success: rgb(0xa7c080), info: rgb(0x7fbbb3), header: rgb(0xa7c080),
        },
        "everforest-light" => Theme {
            name: n, dark: false,
            bg: rgb(0xfdf6e3), surface: rgb(0xefebd4), highlight: rgb(0xf4f0d9), selection: rgb(0xeaedc8),
            fg: rgb(0x5c6a72), muted: rgb(0x939f91), border: rgb(0xbdc3af), border_focus: rgb(0x8da101),
            accent: rgb(0x8da101), accent2: rgb(0x35a77c),
            keyword: rgb(0xf85552), datatype: rgb(0xdfa000), function: rgb(0x8da101), string: rgb(0x35a77c),
            number: rgb(0xdf69ba), comment: rgb(0x939f91), operator: rgb(0xf57d26), identifier: rgb(0x5c6a72),
            quoted_ident: rgb(0x3a94c5), parameter: rgb(0x3a94c5), punctuation: rgb(0x829181),
            null: rgb(0x939f91), boolean: rgb(0xdf69ba), temporal: rgb(0x3a94c5), json: rgb(0x35a77c),
            error: rgb(0xf85552), warning: rgb(0xdfa000), success: rgb(0x8da101), info: rgb(0x3a94c5), header: rgb(0x8da101),
        },
        "github-dark" => Theme {
            name: n, dark: true,
            bg: rgb(0x0d1117), surface: rgb(0x010409), highlight: rgb(0x161b22), selection: rgb(0x264f78),
            fg: rgb(0xe6edf3), muted: rgb(0x7d8590), border: rgb(0x30363d), border_focus: rgb(0x1f6feb),
            accent: rgb(0x58a6ff), accent2: rgb(0xbc8cff),
            keyword: rgb(0xff7b72), datatype: rgb(0xffa657), function: rgb(0xd2a8ff), string: rgb(0xa5d6ff),
            number: rgb(0x79c0ff), comment: rgb(0x8b949e), operator: rgb(0xff7b72), identifier: rgb(0xe6edf3),
            quoted_ident: rgb(0x7ee787), parameter: rgb(0xffa657), punctuation: rgb(0xc9d1d9),
            null: rgb(0x8b949e), boolean: rgb(0x79c0ff), temporal: rgb(0xd2a8ff), json: rgb(0x7ee787),
            error: rgb(0xf85149), warning: rgb(0xd29922), success: rgb(0x3fb950), info: rgb(0x58a6ff), header: rgb(0x58a6ff),
        },
        "github-light" => Theme {
            name: n, dark: false,
            bg: rgb(0xffffff), surface: rgb(0xf6f8fa), highlight: rgb(0xeaeef2), selection: rgb(0xb6e3ff),
            fg: rgb(0x1f2328), muted: rgb(0x656d76), border: rgb(0xd0d7de), border_focus: rgb(0x0969da),
            accent: rgb(0x0969da), accent2: rgb(0x8250df),
            keyword: rgb(0xcf222e), datatype: rgb(0x953800), function: rgb(0x8250df), string: rgb(0x0a3069),
            number: rgb(0x0550ae), comment: rgb(0x6e7781), operator: rgb(0xcf222e), identifier: rgb(0x1f2328),
            quoted_ident: rgb(0x116329), parameter: rgb(0x953800), punctuation: rgb(0x24292f),
            null: rgb(0x6e7781), boolean: rgb(0x0550ae), temporal: rgb(0x8250df), json: rgb(0x116329),
            error: rgb(0xcf222e), warning: rgb(0x9a6700), success: rgb(0x1a7f37), info: rgb(0x0969da), header: rgb(0x0969da),
        },
        "monokai" => Theme {
            name: n, dark: true,
            bg: rgb(0x272822), surface: rgb(0x1e1f1c), highlight: rgb(0x3e3d32), selection: rgb(0x49483e),
            fg: rgb(0xf8f8f2), muted: rgb(0x75715e), border: rgb(0x49483e), border_focus: rgb(0xa6e22e),
            accent: rgb(0xa6e22e), accent2: rgb(0xf92672),
            keyword: rgb(0xf92672), datatype: rgb(0x66d9ef), function: rgb(0xa6e22e), string: rgb(0xe6db74),
            number: rgb(0xae81ff), comment: rgb(0x75715e), operator: rgb(0xf92672), identifier: rgb(0xf8f8f2),
            quoted_ident: rgb(0xfd971f), parameter: rgb(0xfd971f), punctuation: rgb(0xf8f8f2),
            null: rgb(0x75715e), boolean: rgb(0xae81ff), temporal: rgb(0x66d9ef), json: rgb(0xa6e22e),
            error: rgb(0xf92672), warning: rgb(0xfd971f), success: rgb(0xa6e22e), info: rgb(0x66d9ef), header: rgb(0x66d9ef),
        },
        "ayu-dark" => Theme {
            name: n, dark: true,
            bg: rgb(0x0b0e14), surface: rgb(0x0f131a), highlight: rgb(0x131721), selection: rgb(0x273747),
            fg: rgb(0xbfbdb6), muted: rgb(0x6c7380), border: rgb(0x565b66), border_focus: rgb(0xe6b450),
            accent: rgb(0xe6b450), accent2: rgb(0xff8f40),
            keyword: rgb(0xff8f40), datatype: rgb(0x59c2ff), function: rgb(0xffb454), string: rgb(0xaad94c),
            number: rgb(0xd2a6ff), comment: rgb(0x636a72), operator: rgb(0xf29668), identifier: rgb(0xbfbdb6),
            quoted_ident: rgb(0x39bae6), parameter: rgb(0xe6b673), punctuation: rgb(0xbfbdb6),
            null: rgb(0x6c7380), boolean: rgb(0xd2a6ff), temporal: rgb(0x95e6cb), json: rgb(0xe6b673),
            error: rgb(0xd95757), warning: rgb(0xffb454), success: rgb(0x7fd962), info: rgb(0x59c2ff), header: rgb(0x59c2ff),
        },
        "nightfox" => Theme {
            name: n, dark: true,
            bg: rgb(0x192330), surface: rgb(0x131a24), highlight: rgb(0x212e3f), selection: rgb(0x2b3b51),
            fg: rgb(0xcdcecf), muted: rgb(0x71839b), border: rgb(0x39506d), border_focus: rgb(0x719cd6),
            accent: rgb(0x719cd6), accent2: rgb(0x9d79d6),
            keyword: rgb(0x9d79d6), datatype: rgb(0xdbc074), function: rgb(0x86abdc), string: rgb(0x81b29a),
            number: rgb(0xf4a261), comment: rgb(0x738091), operator: rgb(0xaeafb0), identifier: rgb(0xcdcecf),
            quoted_ident: rgb(0x63cdcf), parameter: rgb(0xd67ad2), punctuation: rgb(0xaeafb0),
            null: rgb(0x738091), boolean: rgb(0xf6b079), temporal: rgb(0x63cdcf), json: rgb(0xdbc074),
            error: rgb(0xc94f6d), warning: rgb(0xdbc074), success: rgb(0x81b29a), info: rgb(0x719cd6), header: rgb(0x719cd6),
        },
        "material-ocean" => Theme {
            name: n, dark: true,
            bg: rgb(0x0f111a), surface: rgb(0x090b10), highlight: rgb(0x1a1c25), selection: rgb(0x1f2233),
            fg: rgb(0xa6accd), muted: rgb(0x717cb4), border: rgb(0x232637), border_focus: rgb(0x84ffff),
            accent: rgb(0x84ffff), accent2: rgb(0xc792ea),
            keyword: rgb(0xc792ea), datatype: rgb(0xffcb6b), function: rgb(0x82aaff), string: rgb(0xc3e88d),
            number: rgb(0xf78c6c), comment: rgb(0x464b5d), operator: rgb(0x89ddff), identifier: rgb(0xeeffff),
            quoted_ident: rgb(0xf07178), parameter: rgb(0xff9cac), punctuation: rgb(0x89ddff),
            null: rgb(0x717cb4), boolean: rgb(0xf78c6c), temporal: rgb(0xb0c9ff), json: rgb(0xffcb6b),
            error: rgb(0xf07178), warning: rgb(0xffcb6b), success: rgb(0xc3e88d), info: rgb(0x82aaff), header: rgb(0x82aaff),
        },
        "ansi" => Theme {
            name: n, dark: true,
            bg: Color::Reset, surface: Color::Reset, highlight: Color::Black, selection: Color::DarkGray,
            fg: Color::Reset, muted: Color::DarkGray, border: Color::DarkGray, border_focus: Color::Blue,
            accent: Color::Blue, accent2: Color::Magenta,
            keyword: Color::Magenta, datatype: Color::Cyan, function: Color::Blue, string: Color::Green,
            number: Color::Yellow, comment: Color::DarkGray, operator: Color::LightCyan, identifier: Color::Reset,
            quoted_ident: Color::Cyan, parameter: Color::LightYellow, punctuation: Color::Gray,
            null: Color::DarkGray, boolean: Color::Yellow, temporal: Color::Cyan, json: Color::LightGreen,
            error: Color::Red, warning: Color::Yellow, success: Color::Green, info: Color::Cyan, header: Color::Blue,
        },
        _ => unreachable!("BUILTIN_NAMES and builtin() out of sync: {name}"),
    })
}

const THEME_EXTS: [&str; 3] = ["toml", "yaml", "yml"];
const MAX_INHERIT_DEPTH: usize = 8;

/// Resolves a builtin first, then `<themes_dir>/<name>.toml|yaml|yml`.
pub fn load(name: &str, themes_dir: &Path) -> Result<Theme, String> {
    load_depth(name, themes_dir, 0)
}

fn load_depth(name: &str, themes_dir: &Path, depth: usize) -> Result<Theme, String> {
    if let Some(t) = builtin(name) {
        return Ok(t);
    }
    for ext in THEME_EXTS {
        let path = themes_dir.join(format!("{name}.{ext}"));
        if !path.is_file() {
            continue;
        }
        let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let parsed = if ext == "toml" {
            parse_toml_theme(name, &src, |base| {
                if depth >= MAX_INHERIT_DEPTH {
                    return Err(format!("theme inheritance deeper than {MAX_INHERIT_DEPTH} (cycle?)"));
                }
                load_depth(base, themes_dir, depth + 1)
            })
        } else {
            parse_base16(name, &src)
        };
        return parsed.map_err(|e| format!("{}: {e}", path.display()));
    }
    Err(format!("unknown theme '{name}'; available: {}", list_all(themes_dir).join(", ")))
}

/// Builtin names followed by the theme files found in `themes_dir`, deduplicated.
pub fn list_all(themes_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = BUILTIN_NAMES.iter().map(|s| s.to_string()).collect();
    let mut files: Vec<String> = std::fs::read_dir(themes_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| THEME_EXTS.contains(&e)))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect();
    files.sort();
    for f in files {
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok().map(rgb)).flatten();
    }
    match Color::from_str(s).ok()? {
        Color::Indexed(_) | Color::Rgb(..) => None,
        c => Some(c),
    }
}

fn parse_toml_theme(
    name: &str,
    src: &str,
    resolve: impl FnOnce(&str) -> Result<Theme, String>,
) -> Result<Theme, String> {
    let table: toml::Table = toml::from_str(src).map_err(|e| e.to_string())?;
    let mut theme = match table.get("inherits") {
        None => Theme::default(),
        Some(toml::Value::String(base)) => resolve(base)?,
        Some(_) => return Err("'inherits' must be a theme name string".into()),
    };
    theme.name = name.to_string();
    for (key, value) in &table {
        match key.as_str() {
            "inherits" => {}
            "dark" => theme.dark = value.as_bool().ok_or("'dark' must be true or false")?,
            _ => {
                let slot = theme
                    .colors_mut()
                    .into_iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, c)| c)
                    .ok_or_else(|| format!("unknown key '{key}'"))?;
                *slot = value
                    .as_str()
                    .and_then(parse_color)
                    .ok_or_else(|| format!("'{key}': expected \"#rrggbb\", an ANSI color name or \"reset\""))?;
            }
        }
    }
    Ok(theme)
}

/// Reads a base16/base24 scheme in the classic (`scheme:` + top-level `baseXX:`) or tinted-theming
/// (`system:` + `palette:` map) layout. Both are flat enough that a line reader suffices.
fn parse_base16(name: &str, src: &str) -> Result<Theme, String> {
    let mut base: [Option<u32>; 24] = [None; 24];
    let mut variant_dark: Option<bool> = None;
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim().trim_matches(|c| c == '"' || c == '\'');
        let value = yaml_scalar(value);
        if key == "variant" {
            variant_dark = Some(!value.eq_ignore_ascii_case("light"));
            continue;
        }
        let Some(idx) = key.strip_prefix("base").filter(|h| h.len() == 2).and_then(|h| usize::from_str_radix(h, 16).ok())
        else {
            continue;
        };
        if idx >= base.len() {
            continue;
        }
        let hex = value.trim_start_matches('#');
        let v = (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten();
        base[idx] = Some(v.ok_or_else(|| format!("{key}: invalid color '{value}'"))?);
    }
    let mut c = [Color::Reset; 24];
    for (i, slot) in base.iter().enumerate().take(16) {
        c[i] = rgb(slot.ok_or_else(|| format!("missing base{i:02X}"))?);
    }
    let base24 = base[16..].iter().all(Option::is_some);
    if base24 {
        for i in 16..24 {
            c[i] = rgb(base[i].unwrap_or_default());
        }
    }
    let dark = variant_dark.unwrap_or_else(|| {
        let v = base[0].unwrap_or_default();
        relative_luminance((v >> 16) as u8, (v >> 8) as u8, v as u8) < 0.179
    });
    let pick = |b24: usize, b16: usize| if base24 { c[b24] } else { c[b16] };
    Ok(Theme {
        name: name.to_string(),
        dark,
        bg: c[0x00], surface: pick(0x10, 0x01), highlight: c[0x01], selection: c[0x02],
        fg: c[0x05], muted: c[0x04], border: c[0x02], border_focus: pick(0x16, 0x0D),
        accent: c[0x0D], accent2: pick(0x17, 0x0E),
        keyword: c[0x0E], datatype: c[0x0A], function: c[0x0D], string: c[0x0B],
        number: c[0x09], comment: c[0x03], operator: c[0x05], identifier: c[0x05],
        quoted_ident: c[0x0C], parameter: c[0x08], punctuation: c[0x04],
        null: c[0x03], boolean: c[0x09], temporal: c[0x0C], json: c[0x0F],
        error: pick(0x12, 0x08), warning: pick(0x13, 0x0A), success: pick(0x14, 0x0B), info: pick(0x15, 0x0C),
        header: c[0x0D],
    })
}

/// Strips quotes, or an unquoted value's trailing ` # comment`.
fn yaml_scalar(v: &str) -> &str {
    let v = v.trim();
    for q in ['"', '\''] {
        if let Some(rest) = v.strip_prefix(q) {
            return rest.split(q).next().unwrap_or("");
        }
    }
    v.split(" #").next().unwrap_or("").trim()
}

/// WCAG relative luminance of an sRGB color.
fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    let lin = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shimmer_band_moves_with_the_tick_and_blends_between_the_colours() {
        let (base, peak) = (Color::Rgb(0, 0, 0), Color::Rgb(200, 200, 200));
        let at = |tick| shimmer(12, tick, base, peak);
        let brightest = |v: Vec<Color>| v.iter().position(|c| *c == peak);
        assert_eq!(brightest(at(4)), Some(0));
        assert_eq!(brightest(at(9)), Some(5), "the band advances one character per tick");
        assert!(at(9).contains(&Color::Rgb(100, 100, 100)), "neighbours are blended, not switched");
        assert!(at(9).iter().filter(|c| **c == base).count() > 3, "far characters keep the base colour");
        let indexed = shimmer(12, 9, Color::Indexed(8), Color::Indexed(15));
        assert!(indexed.iter().all(|c| *c == Color::Indexed(8) || *c == Color::Indexed(15)));
    }


    fn contrast(a: Color, b: Color) -> f64 {
        let lum = |c: Color| match c {
            Color::Rgb(r, g, b) => relative_luminance(r, g, b),
            other => panic!("expected rgb, got {other:?}"),
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("quarry-theme-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn every_builtin_is_legible_and_distinguishes_core_syntax() {
        for name in builtin_names() {
            let t = builtin(name).unwrap_or_else(|| panic!("{name} missing"));
            assert_eq!(t.name, *name);
            assert_ne!(t.keyword, t.string, "{name}: keyword vs string");
            assert_ne!(t.keyword, t.comment, "{name}: keyword vs comment");
            assert_ne!(t.string, t.comment, "{name}: string vs comment");
            if *name == "ansi" {
                continue;
            }
            let fg = contrast(t.fg, t.bg);
            assert!(fg >= 4.5, "{name}: fg/bg contrast {fg:.2} < 4.5");
            let cm = contrast(t.comment, t.bg);
            assert!(cm >= 2.0, "{name}: comment/bg contrast {cm:.2} < 2.0");
            let bg_dark = matches!(t.bg, Color::Rgb(r, g, b) if relative_luminance(r, g, b) < 0.179);
            assert_eq!(t.dark, bg_dark, "{name}: dark flag disagrees with its background");
        }
    }

    #[test]
    fn ansi_theme_keeps_the_terminal_palette() {
        let mut t = builtin("ansi").unwrap();
        assert_eq!(t.bg, Color::Reset);
        for (key, c) in t.colors_mut() {
            assert!(!matches!(c, Color::Rgb(..) | Color::Indexed(_)), "{key} is {c:?}");
        }
    }

    #[test]
    fn names_resolve_case_and_separator_insensitively() {
        for (input, want) in [
            ("Tokyo_Night", "tokyo-night"),
            ("tokyo night storm", "tokyo-night-storm"),
            ("tokyonight-day", "tokyo-night-day"),
            ("CATPPUCCIN_MOCHA", "catppuccin-mocha"),
            ("catppuccin", "catppuccin-mocha"),
            ("Rosé Pine Dawn", "rose-pine-dawn"),
            ("onedark", "one-dark"),
            ("everforest", "everforest-dark"),
        ] {
            assert_eq!(builtin(input).map(|t| t.name), Some(want.to_string()), "{input}");
        }
        assert!(builtin("no-such-theme").is_none());
        assert!(builtin("").is_none());
    }

    #[test]
    fn toml_theme_inherits_and_overrides_only_given_keys() {
        let dir = temp_dir("toml");
        std::fs::write(
            dir.join("mine.toml"),
            "inherits = \"gruvbox-dark\"\nkeyword = \"#ff0000\"\nbg = \"reset\"\ncomment = \"dark gray\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("child.toml"), "inherits = \"mine\"\nstring = \"#00ff00\"\ndark = false\n").unwrap();
        let base = builtin("gruvbox-dark").unwrap();

        let t = load("mine", &dir).unwrap();
        assert_eq!(t.name, "mine");
        assert_eq!(t.keyword, Color::Rgb(255, 0, 0));
        assert_eq!(t.bg, Color::Reset);
        assert_eq!(t.comment, Color::DarkGray);
        assert_eq!(t.string, base.string, "untouched keys come from the base theme");

        let c = load("child", &dir).unwrap();
        assert_eq!(c.keyword, Color::Rgb(255, 0, 0), "inheritance chains through custom themes");
        assert_eq!(c.string, Color::Rgb(0, 255, 0));
        assert!(!c.dark);
    }

    #[test]
    fn toml_theme_errors_name_the_offending_key() {
        let dir = temp_dir("toml-err");
        std::fs::write(dir.join("typo.toml"), "inherits = \"nord\"\nkeywrod = \"#ffffff\"\n").unwrap();
        std::fs::write(dir.join("badcolor.toml"), "keyword = \"#12\"\n").unwrap();
        std::fs::write(dir.join("loop.toml"), "inherits = \"loop\"\n").unwrap();
        assert!(load("typo", &dir).unwrap_err().contains("'keywrod'"));
        assert!(load("badcolor", &dir).unwrap_err().contains("'keyword'"));
        assert!(load("loop", &dir).unwrap_err().contains("inheritance"));
        assert!(load("missing", &dir).unwrap_err().contains("unknown theme 'missing'"));
    }

    #[test]
    fn builtins_win_over_files_and_list_all_merges_both() {
        let dir = temp_dir("list");
        std::fs::write(dir.join("nord.toml"), "keyword = \"#ffffff\"\n").unwrap();
        std::fs::write(dir.join("zeta.toml"), "").unwrap();
        std::fs::write(dir.join("alpha.yaml"), "").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        assert_eq!(load("nord", &dir).unwrap(), builtin("nord").unwrap());
        let all = list_all(&dir);
        assert_eq!(&all[..builtin_names().len()], builtin_names());
        assert_eq!(&all[builtin_names().len()..], ["alpha", "zeta"]);
        assert_eq!(list_all(&dir.join("absent")).len(), builtin_names().len());
    }

    const CLASSIC: &str = r#"scheme: "Tomorrow Night"
author: "Chris Kempson (http://chriskempson.com)"
base00: "1d1f21"
base01: "282a2e"
base02: "373b41"
base03: "969896"
base04: "b4b7b4"
base05: "c5c8c6"
base06: "e0e0e0"
base07: "ffffff"
base08: "cc6666"
base09: "de935f"
base0A: "f0c674"
base0B: "b5bd68"
base0C: "8abeb7"
base0D: "81a2be"
base0E: "b294bb"
base0F: "a3685a"
"#;

    const TINTED_LIGHT: &str = r##"system: "base16"
name: "Tomorrow"
author: "Chris Kempson"
variant: "light"
palette:
  base00: "#ffffff" # background
  base01: "#e0e0e0"
  base02: "#d6d6d6"
  base03: "#8e908c"
  base04: "#969896"
  base05: "#4d4d4c"
  base06: "#282a2e"
  base07: "#1d1f21"
  base08: "#c82829"
  base09: "#f5871f"
  base0A: "#eab700"
  base0B: "#718c00"
  base0C: "#3e999f"
  base0D: "#4271ae"
  base0E: "#8959a8"
  base0F: "#a3685a"
"##;

    #[test]
    fn base16_classic_maps_per_styling_guidelines() {
        let dir = temp_dir("b16");
        std::fs::write(dir.join("tomorrow-night.yaml"), CLASSIC).unwrap();
        let t = load("tomorrow-night", &dir).unwrap();
        assert_eq!(t.name, "tomorrow-night");
        assert!(t.dark, "dark inferred from base00 luminance");
        assert_eq!(t.bg, rgb(0x1d1f21));
        assert_eq!(t.fg, rgb(0xc5c8c6));
        assert_eq!(t.keyword, rgb(0xb294bb), "base0E -> keyword");
        assert_eq!(t.string, rgb(0xb5bd68), "base0B -> string");
        assert_eq!(t.comment, rgb(0x969896), "base03 -> comment");
        assert_eq!(t.number, rgb(0xde935f), "base09 -> number");
        assert_eq!(t.function, rgb(0x81a2be), "base0D -> function");
        assert_eq!(t.datatype, rgb(0xf0c674), "base0A -> type");
        assert_eq!(t.error, rgb(0xcc6666), "base08 -> error");
        assert_eq!(t.selection, rgb(0x373b41), "base02 -> selection");
    }

    #[test]
    fn base16_tinted_format_honours_variant_and_inline_comments() {
        let dir = temp_dir("tinted");
        std::fs::write(dir.join("tomorrow.yml"), TINTED_LIGHT).unwrap();
        let t = load("tomorrow", &dir).unwrap();
        assert!(!t.dark);
        assert_eq!(t.bg, rgb(0xffffff));
        assert_eq!(t.keyword, rgb(0x8959a8));
        assert_eq!(t.string, rgb(0x718c00));

        let inferred = parse_base16("x", &TINTED_LIGHT.replace("variant: \"light\"\n", "")).unwrap();
        assert!(!inferred.dark, "white base00 infers a light theme");
    }

    #[test]
    fn base24_uses_extended_slots_and_missing_slots_error() {
        let mut src = CLASSIC.to_string();
        for i in 0x10..0x18u32 {
            src.push_str(&format!("base{i:02X}: \"{:06x}\"\n", 0x101010 * (i - 0x0f)));
        }
        let t = parse_base16("b24", &src).unwrap();
        assert_eq!(t.surface, rgb(0x101010), "base10 -> surface");
        assert_eq!(t.error, rgb(0x303030), "base12 -> error");
        assert_eq!(t.keyword, rgb(0xb294bb));

        let err = parse_base16("bad", &CLASSIC.replace("base0E: \"b294bb\"\n", "")).unwrap_err();
        assert!(err.contains("base0E"), "{err}");
    }

    #[test]
    fn adaptation_stays_within_the_terminal_palette() {
        for name in builtin_names() {
            let t = builtin(name).unwrap();
            for (key, c) in t.adapted(ColorDepth::Ansi256).colors_mut() {
                assert!(!matches!(c, Color::Rgb(..)), "{name}.{key} still rgb at 256 colors");
            }
            for (key, c) in t.adapted(ColorDepth::Ansi16).colors_mut() {
                let ok = match c {
                    Color::Indexed(i) => *i < 16,
                    Color::Rgb(..) => false,
                    _ => true,
                };
                assert!(ok, "{name}.{key} = {c:?} outside 16 colors");
            }
            for (_, c) in t.adapted(ColorDepth::None).colors_mut() {
                assert_eq!(*c, Color::Reset);
            }
            assert_eq!(t.adapted(ColorDepth::TrueColor), t);
        }
    }
}
