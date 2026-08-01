//! Colour themes.
//!
//! A theme is a flat palette of semantic roles — no widget ever names a
//! literal colour, it asks for `theme.danger` or `theme.border_focus`. That
//! keeps every panel consistent when a new palette is added, and it is what
//! makes the live theme picker able to restyle the whole UI in one frame.
//!
//! Themes are `const` data, so switching one is a pointer swap with no
//! allocation — cheap enough to re-resolve on every frame of the picker.

use ratatui::style::Color;

/// A complete palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Stable id used in config and on the CLI.
    pub id: &'static str,
    /// Human-readable name for the picker.
    pub label: &'static str,
    /// Painted behind the whole UI. `Color::Reset` keeps the terminal's own.
    pub bg: Color,
    /// Default body text.
    pub fg: Color,
    /// Secondary text, units, inactive labels.
    pub muted: Color,
    /// Brand accent: app name, selected tab, primary gauges.
    pub accent: Color,
    /// Secondary accent: alternate series, secondary highlights.
    pub accent_alt: Color,
    /// Unfocused panel border.
    pub border: Color,
    /// Focused panel border.
    pub border_focus: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub info: Color,
    /// Selected-row background and its readable foreground.
    pub highlight_bg: Color,
    pub highlight_fg: Color,
}

macro_rules! rgb {
    ($r:expr, $g:expr, $b:expr) => {
        Color::Rgb($r, $g, $b)
    };
}

/// Every built-in theme, in picker order.
pub const THEMES: &[Theme] = &[
    Theme {
        id: "claude",
        label: "Claude",
        bg: rgb!(0x1B, 0x19, 0x16),
        fg: rgb!(0xEC, 0xE8, 0xE1),
        muted: rgb!(0x8A, 0x86, 0x7D),
        accent: rgb!(0xD9, 0x77, 0x57),
        accent_alt: rgb!(0xE8, 0xC5, 0x9E),
        border: rgb!(0x4A, 0x46, 0x40),
        border_focus: rgb!(0xD9, 0x77, 0x57),
        success: rgb!(0x87, 0xB3, 0x7B),
        warning: rgb!(0xE0, 0xB4, 0x4C),
        danger: rgb!(0xE0, 0x6C, 0x5F),
        info: rgb!(0x7B, 0xA9, 0xC7),
        highlight_bg: rgb!(0xD9, 0x77, 0x57),
        highlight_fg: rgb!(0x1B, 0x19, 0x16),
    },
    Theme {
        id: "obs",
        label: "OBS Studio",
        bg: rgb!(0x1F, 0x22, 0x26),
        fg: rgb!(0xE6, 0xE8, 0xEB),
        muted: rgb!(0x7A, 0x82, 0x8C),
        accent: rgb!(0x30, 0x9A, 0xE0),
        accent_alt: rgb!(0x8A, 0xD1, 0xFF),
        border: rgb!(0x33, 0x38, 0x3F),
        border_focus: rgb!(0x30, 0x9A, 0xE0),
        success: rgb!(0x5C, 0xC9, 0x7A),
        warning: rgb!(0xF0, 0xB4, 0x3C),
        danger: rgb!(0xE5, 0x4B, 0x4B),
        info: rgb!(0x8A, 0xD1, 0xFF),
        highlight_bg: rgb!(0x30, 0x9A, 0xE0),
        highlight_fg: rgb!(0x0F, 0x12, 0x16),
    },
    Theme {
        id: "btop",
        label: "Btop",
        bg: rgb!(0x0A, 0x14, 0x0A),
        fg: rgb!(0xD4, 0xE6, 0xD4),
        muted: rgb!(0x5A, 0x6A, 0x5A),
        accent: rgb!(0x6A, 0xE0, 0x5A),
        accent_alt: rgb!(0xF0, 0xE0, 0x50),
        border: rgb!(0x30, 0x40, 0x30),
        border_focus: rgb!(0x6A, 0xE0, 0x5A),
        success: rgb!(0x6A, 0xE0, 0x5A),
        warning: rgb!(0xF0, 0xE0, 0x50),
        danger: rgb!(0xE0, 0x50, 0x50),
        info: rgb!(0x50, 0xC0, 0xE0),
        highlight_bg: rgb!(0x6A, 0xE0, 0x5A),
        highlight_fg: rgb!(0x0A, 0x14, 0x0A),
    },
    Theme {
        id: "nord",
        label: "Nord",
        bg: rgb!(0x2E, 0x34, 0x40),
        fg: rgb!(0xE5, 0xE9, 0xF0),
        muted: rgb!(0x61, 0x6E, 0x88),
        accent: rgb!(0x88, 0xC0, 0xD0),
        accent_alt: rgb!(0x81, 0xA1, 0xC1),
        border: rgb!(0x3B, 0x42, 0x52),
        border_focus: rgb!(0x88, 0xC0, 0xD0),
        success: rgb!(0xA3, 0xBE, 0x8C),
        warning: rgb!(0xEB, 0xCB, 0x8B),
        danger: rgb!(0xBF, 0x61, 0x6A),
        info: rgb!(0x81, 0xA1, 0xC1),
        highlight_bg: rgb!(0x88, 0xC0, 0xD0),
        highlight_fg: rgb!(0x2E, 0x34, 0x40),
    },
    Theme {
        id: "dracula",
        label: "Dracula",
        bg: rgb!(0x28, 0x2A, 0x36),
        fg: rgb!(0xF8, 0xF8, 0xF2),
        muted: rgb!(0x62, 0x72, 0xA4),
        accent: rgb!(0xBD, 0x93, 0xF9),
        accent_alt: rgb!(0xFF, 0x79, 0xC6),
        border: rgb!(0x3A, 0x3D, 0x52),
        border_focus: rgb!(0xBD, 0x93, 0xF9),
        success: rgb!(0x50, 0xFA, 0x7B),
        warning: rgb!(0xF1, 0xFA, 0x8C),
        danger: rgb!(0xFF, 0x55, 0x55),
        info: rgb!(0x8B, 0xE9, 0xFD),
        highlight_bg: rgb!(0xBD, 0x93, 0xF9),
        highlight_fg: rgb!(0x1E, 0x1F, 0x29),
    },
    Theme {
        id: "gruvbox",
        label: "Gruvbox Dark",
        bg: rgb!(0x28, 0x28, 0x28),
        fg: rgb!(0xEB, 0xDB, 0xB2),
        muted: rgb!(0x92, 0x83, 0x74),
        accent: rgb!(0xFE, 0x80, 0x19),
        accent_alt: rgb!(0xD3, 0x86, 0x9B),
        border: rgb!(0x3C, 0x38, 0x36),
        border_focus: rgb!(0xFE, 0x80, 0x19),
        success: rgb!(0xB8, 0xBB, 0x26),
        warning: rgb!(0xFA, 0xBD, 0x2F),
        danger: rgb!(0xFB, 0x49, 0x34),
        info: rgb!(0x83, 0xA5, 0x98),
        highlight_bg: rgb!(0xFE, 0x80, 0x19),
        highlight_fg: rgb!(0x28, 0x28, 0x28),
    },
    Theme {
        id: "gruvbox-light",
        label: "Gruvbox Light",
        bg: rgb!(0xFB, 0xF1, 0xC7),
        fg: rgb!(0x3C, 0x38, 0x36),
        muted: rgb!(0x7C, 0x6F, 0x64),
        accent: rgb!(0xAF, 0x3A, 0x03),
        accent_alt: rgb!(0x8F, 0x3F, 0x71),
        border: rgb!(0xD5, 0xC4, 0xA1),
        border_focus: rgb!(0xAF, 0x3A, 0x03),
        success: rgb!(0x79, 0x74, 0x0E),
        warning: rgb!(0xB5, 0x76, 0x14),
        danger: rgb!(0x9D, 0x00, 0x06),
        info: rgb!(0x07, 0x66, 0x78),
        highlight_bg: rgb!(0xAF, 0x3A, 0x03),
        highlight_fg: rgb!(0xFB, 0xF1, 0xC7),
    },
    Theme {
        id: "solarized-dark",
        label: "Solarized Dark",
        bg: rgb!(0x00, 0x2B, 0x36),
        fg: rgb!(0x93, 0xA1, 0xA1),
        muted: rgb!(0x58, 0x6E, 0x75),
        accent: rgb!(0x26, 0x8B, 0xD2),
        accent_alt: rgb!(0x2A, 0xA1, 0x98),
        border: rgb!(0x07, 0x36, 0x42),
        border_focus: rgb!(0x26, 0x8B, 0xD2),
        success: rgb!(0x85, 0x99, 0x00),
        warning: rgb!(0xB5, 0x89, 0x00),
        danger: rgb!(0xDC, 0x32, 0x2F),
        info: rgb!(0x2A, 0xA1, 0x98),
        highlight_bg: rgb!(0x26, 0x8B, 0xD2),
        highlight_fg: rgb!(0x00, 0x2B, 0x36),
    },
    Theme {
        id: "solarized-light",
        label: "Solarized Light",
        bg: rgb!(0xFD, 0xF6, 0xE3),
        fg: rgb!(0x58, 0x6E, 0x75),
        muted: rgb!(0x93, 0xA1, 0xA1),
        accent: rgb!(0x26, 0x8B, 0xD2),
        accent_alt: rgb!(0x2A, 0xA1, 0x98),
        border: rgb!(0xEE, 0xE8, 0xD5),
        border_focus: rgb!(0x26, 0x8B, 0xD2),
        success: rgb!(0x85, 0x99, 0x00),
        warning: rgb!(0xB5, 0x89, 0x00),
        danger: rgb!(0xDC, 0x32, 0x2F),
        info: rgb!(0x2A, 0xA1, 0x98),
        highlight_bg: rgb!(0x26, 0x8B, 0xD2),
        highlight_fg: rgb!(0xFD, 0xF6, 0xE3),
    },
    Theme {
        id: "tokyo-night",
        label: "Tokyo Night",
        bg: rgb!(0x1A, 0x1B, 0x26),
        fg: rgb!(0xC0, 0xCA, 0xF5),
        muted: rgb!(0x56, 0x5F, 0x89),
        accent: rgb!(0x7A, 0xA2, 0xF7),
        accent_alt: rgb!(0xBB, 0x9A, 0xF7),
        border: rgb!(0x29, 0x2E, 0x42),
        border_focus: rgb!(0x7A, 0xA2, 0xF7),
        success: rgb!(0x9E, 0xCE, 0x6A),
        warning: rgb!(0xE0, 0xAF, 0x68),
        danger: rgb!(0xF7, 0x76, 0x8E),
        info: rgb!(0x7D, 0xCF, 0xFF),
        highlight_bg: rgb!(0x7A, 0xA2, 0xF7),
        highlight_fg: rgb!(0x1A, 0x1B, 0x26),
    },
    Theme {
        id: "catppuccin-mocha",
        label: "Catppuccin Mocha",
        bg: rgb!(0x1E, 0x1E, 0x2E),
        fg: rgb!(0xCD, 0xD6, 0xF4),
        muted: rgb!(0x6C, 0x70, 0x86),
        accent: rgb!(0xCB, 0xA6, 0xF7),
        accent_alt: rgb!(0xF5, 0xC2, 0xE7),
        border: rgb!(0x31, 0x32, 0x44),
        border_focus: rgb!(0xCB, 0xA6, 0xF7),
        success: rgb!(0xA6, 0xE3, 0xA1),
        warning: rgb!(0xF9, 0xE2, 0xAF),
        danger: rgb!(0xF3, 0x8B, 0xA8),
        info: rgb!(0x89, 0xB4, 0xFA),
        highlight_bg: rgb!(0xCB, 0xA6, 0xF7),
        highlight_fg: rgb!(0x1E, 0x1E, 0x2E),
    },
    Theme {
        id: "catppuccin-latte",
        label: "Catppuccin Latte",
        bg: rgb!(0xEF, 0xF1, 0xF5),
        fg: rgb!(0x4C, 0x4F, 0x69),
        muted: rgb!(0x9C, 0xA0, 0xB0),
        accent: rgb!(0x88, 0x39, 0xEF),
        accent_alt: rgb!(0xEA, 0x76, 0xCB),
        border: rgb!(0xDC, 0xE0, 0xE8),
        border_focus: rgb!(0x88, 0x39, 0xEF),
        success: rgb!(0x40, 0xA0, 0x2B),
        warning: rgb!(0xDF, 0x8E, 0x1D),
        danger: rgb!(0xD2, 0x0F, 0x39),
        info: rgb!(0x1E, 0x66, 0xF5),
        highlight_bg: rgb!(0x88, 0x39, 0xEF),
        highlight_fg: rgb!(0xEF, 0xF1, 0xF5),
    },
    Theme {
        id: "everforest",
        label: "Everforest",
        bg: rgb!(0x2D, 0x35, 0x3B),
        fg: rgb!(0xD3, 0xC6, 0xAA),
        muted: rgb!(0x85, 0x92, 0x89),
        accent: rgb!(0xA7, 0xC0, 0x80),
        accent_alt: rgb!(0x83, 0xC0, 0x92),
        border: rgb!(0x3D, 0x48, 0x4D),
        border_focus: rgb!(0xA7, 0xC0, 0x80),
        success: rgb!(0xA7, 0xC0, 0x80),
        warning: rgb!(0xDB, 0xBC, 0x7F),
        danger: rgb!(0xE6, 0x7E, 0x80),
        info: rgb!(0x7F, 0xBB, 0xB3),
        highlight_bg: rgb!(0xA7, 0xC0, 0x80),
        highlight_fg: rgb!(0x2D, 0x35, 0x3B),
    },
    Theme {
        id: "kanagawa",
        label: "Kanagawa",
        bg: rgb!(0x1F, 0x1F, 0x28),
        fg: rgb!(0xDC, 0xD7, 0xBA),
        muted: rgb!(0x72, 0x71, 0x69),
        accent: rgb!(0x7E, 0x9C, 0xD8),
        accent_alt: rgb!(0x95, 0x7F, 0xB8),
        border: rgb!(0x2A, 0x2A, 0x37),
        border_focus: rgb!(0x7E, 0x9C, 0xD8),
        success: rgb!(0x98, 0xBB, 0x6C),
        warning: rgb!(0xE6, 0xC3, 0x84),
        danger: rgb!(0xE8, 0x24, 0x24),
        info: rgb!(0x7A, 0xA8, 0x9F),
        highlight_bg: rgb!(0x7E, 0x9C, 0xD8),
        highlight_fg: rgb!(0x1F, 0x1F, 0x28),
    },
    Theme {
        id: "rose-pine",
        label: "Rosé Pine",
        bg: rgb!(0x19, 0x17, 0x24),
        fg: rgb!(0xE0, 0xDE, 0xF4),
        muted: rgb!(0x6E, 0x6A, 0x86),
        accent: rgb!(0xC4, 0xA7, 0xE7),
        accent_alt: rgb!(0xEB, 0xBC, 0xBA),
        border: rgb!(0x26, 0x23, 0x3A),
        border_focus: rgb!(0xC4, 0xA7, 0xE7),
        success: rgb!(0x9C, 0xCF, 0xD8),
        warning: rgb!(0xF6, 0xC1, 0x77),
        danger: rgb!(0xEB, 0x6F, 0x92),
        info: rgb!(0x31, 0x74, 0x8F),
        highlight_bg: rgb!(0xC4, 0xA7, 0xE7),
        highlight_fg: rgb!(0x19, 0x17, 0x24),
    },
    Theme {
        id: "one-dark",
        label: "One Dark",
        bg: rgb!(0x28, 0x2C, 0x34),
        fg: rgb!(0xAB, 0xB2, 0xBF),
        muted: rgb!(0x5C, 0x63, 0x70),
        accent: rgb!(0x61, 0xAF, 0xEF),
        accent_alt: rgb!(0xC6, 0x78, 0xDD),
        border: rgb!(0x3E, 0x44, 0x51),
        border_focus: rgb!(0x61, 0xAF, 0xEF),
        success: rgb!(0x98, 0xC3, 0x79),
        warning: rgb!(0xE5, 0xC0, 0x7B),
        danger: rgb!(0xE0, 0x6C, 0x75),
        info: rgb!(0x56, 0xB6, 0xC2),
        highlight_bg: rgb!(0x61, 0xAF, 0xEF),
        highlight_fg: rgb!(0x28, 0x2C, 0x34),
    },
    Theme {
        id: "monokai",
        label: "Monokai",
        bg: rgb!(0x27, 0x28, 0x22),
        fg: rgb!(0xF8, 0xF8, 0xF2),
        muted: rgb!(0x75, 0x71, 0x5E),
        accent: rgb!(0xA6, 0xE2, 0x2E),
        accent_alt: rgb!(0xFD, 0x97, 0x1F),
        border: rgb!(0x3E, 0x3D, 0x32),
        border_focus: rgb!(0xA6, 0xE2, 0x2E),
        success: rgb!(0xA6, 0xE2, 0x2E),
        warning: rgb!(0xE6, 0xDB, 0x74),
        danger: rgb!(0xF9, 0x26, 0x72),
        info: rgb!(0x66, 0xD9, 0xEF),
        highlight_bg: rgb!(0xA6, 0xE2, 0x2E),
        highlight_fg: rgb!(0x27, 0x28, 0x22),
    },
    Theme {
        id: "ayu-dark",
        label: "Ayu Dark",
        bg: rgb!(0x0B, 0x0E, 0x14),
        fg: rgb!(0xBF, 0xBD, 0xB6),
        muted: rgb!(0x56, 0x5B, 0x66),
        accent: rgb!(0xE6, 0xB4, 0x50),
        accent_alt: rgb!(0x59, 0xC2, 0xFF),
        border: rgb!(0x1C, 0x21, 0x2B),
        border_focus: rgb!(0xE6, 0xB4, 0x50),
        success: rgb!(0xAA, 0xD9, 0x4C),
        warning: rgb!(0xFF, 0xB4, 0x54),
        danger: rgb!(0xF0, 0x71, 0x78),
        info: rgb!(0x59, 0xC2, 0xFF),
        highlight_bg: rgb!(0xE6, 0xB4, 0x50),
        highlight_fg: rgb!(0x0B, 0x0E, 0x14),
    },
    Theme {
        id: "github-dark",
        label: "GitHub Dark",
        bg: rgb!(0x0D, 0x11, 0x17),
        fg: rgb!(0xC9, 0xD1, 0xD9),
        muted: rgb!(0x6E, 0x76, 0x81),
        accent: rgb!(0x58, 0xA6, 0xFF),
        accent_alt: rgb!(0xBC, 0x8C, 0xFF),
        border: rgb!(0x21, 0x26, 0x2D),
        border_focus: rgb!(0x58, 0xA6, 0xFF),
        success: rgb!(0x3F, 0xB9, 0x50),
        warning: rgb!(0xD2, 0x99, 0x22),
        danger: rgb!(0xF8, 0x51, 0x49),
        info: rgb!(0x79, 0xC0, 0xFF),
        highlight_bg: rgb!(0x58, 0xA6, 0xFF),
        highlight_fg: rgb!(0x0D, 0x11, 0x17),
    },
    Theme {
        id: "github-light",
        label: "GitHub Light",
        bg: rgb!(0xFF, 0xFF, 0xFF),
        fg: rgb!(0x24, 0x29, 0x2F),
        muted: rgb!(0x65, 0x6D, 0x76),
        accent: rgb!(0x09, 0x69, 0xDA),
        accent_alt: rgb!(0x82, 0x50, 0xDF),
        border: rgb!(0xD0, 0xD7, 0xDE),
        border_focus: rgb!(0x09, 0x69, 0xDA),
        success: rgb!(0x1A, 0x7F, 0x37),
        warning: rgb!(0x9A, 0x66, 0x00),
        danger: rgb!(0xCF, 0x22, 0x2E),
        info: rgb!(0x09, 0x69, 0xDA),
        highlight_bg: rgb!(0x09, 0x69, 0xDA),
        highlight_fg: rgb!(0xFF, 0xFF, 0xFF),
    },
    Theme {
        id: "synthwave",
        label: "Synthwave",
        bg: rgb!(0x26, 0x22, 0x35),
        fg: rgb!(0xF9, 0xF5, 0xFF),
        muted: rgb!(0x84, 0x7A, 0x9E),
        accent: rgb!(0xFF, 0x2E, 0x97),
        accent_alt: rgb!(0x2D, 0xE2, 0xE6),
        border: rgb!(0x39, 0x33, 0x4F),
        border_focus: rgb!(0xFF, 0x2E, 0x97),
        success: rgb!(0x72, 0xF1, 0xB8),
        warning: rgb!(0xFE, 0xDE, 0x5D),
        danger: rgb!(0xFE, 0x44, 0x50),
        info: rgb!(0x2D, 0xE2, 0xE6),
        highlight_bg: rgb!(0xFF, 0x2E, 0x97),
        highlight_fg: rgb!(0x26, 0x22, 0x35),
    },
    Theme {
        id: "matrix",
        label: "Matrix",
        bg: rgb!(0x00, 0x0A, 0x00),
        fg: rgb!(0x00, 0xD0, 0x40),
        muted: rgb!(0x00, 0x60, 0x20),
        accent: rgb!(0x00, 0xFF, 0x55),
        accent_alt: rgb!(0x90, 0xFF, 0xA0),
        border: rgb!(0x00, 0x35, 0x12),
        border_focus: rgb!(0x00, 0xFF, 0x55),
        success: rgb!(0x00, 0xFF, 0x55),
        warning: rgb!(0xCC, 0xFF, 0x00),
        danger: rgb!(0xFF, 0x30, 0x30),
        info: rgb!(0x40, 0xFF, 0xC0),
        highlight_bg: rgb!(0x00, 0xFF, 0x55),
        highlight_fg: rgb!(0x00, 0x0A, 0x00),
    },
    Theme {
        id: "high-contrast",
        label: "High Contrast",
        bg: rgb!(0x00, 0x00, 0x00),
        fg: rgb!(0xFF, 0xFF, 0xFF),
        muted: rgb!(0xB0, 0xB0, 0xB0),
        accent: rgb!(0xFF, 0xFF, 0x00),
        accent_alt: rgb!(0x00, 0xFF, 0xFF),
        border: rgb!(0x80, 0x80, 0x80),
        border_focus: rgb!(0xFF, 0xFF, 0x00),
        success: rgb!(0x00, 0xFF, 0x00),
        warning: rgb!(0xFF, 0xA5, 0x00),
        danger: rgb!(0xFF, 0x00, 0x00),
        info: rgb!(0x00, 0xFF, 0xFF),
        highlight_bg: rgb!(0xFF, 0xFF, 0x00),
        highlight_fg: rgb!(0x00, 0x00, 0x00),
    },
    Theme {
        // Uses only the 16 ANSI colours and never sets a background, so it
        // works on a Linux VT and inherits the user's own terminal colours.
        id: "mono",
        label: "Mono (TTY-safe)",
        bg: Color::Reset,
        fg: Color::White,
        muted: Color::DarkGray,
        accent: Color::White,
        accent_alt: Color::Gray,
        border: Color::DarkGray,
        border_focus: Color::White,
        success: Color::Green,
        warning: Color::Yellow,
        danger: Color::Red,
        info: Color::Cyan,
        highlight_bg: Color::White,
        highlight_fg: Color::Black,
    },
];

/// The theme used when config names one that no longer exists.
pub const FALLBACK: &Theme = &THEMES[0];

/// Look up a theme by id. Case- and separator-insensitive, so `Tokyo_Night`
/// and `tokyo-night` both resolve.
pub fn find(id: &str) -> Option<&'static Theme> {
    let wanted = normalize(id);
    THEMES.iter().find(|theme| normalize(theme.id) == wanted)
}

/// Look up a theme by id, falling back to [`FALLBACK`].
pub fn resolve(id: &str) -> &'static Theme {
    find(id).unwrap_or(FALLBACK)
}

/// Index of a theme in [`THEMES`], for positioning the picker's cursor.
pub fn index_of(id: &str) -> usize {
    let wanted = normalize(id);
    THEMES
        .iter()
        .position(|theme| normalize(theme.id) == wanted)
        .unwrap_or(0)
}

fn normalize(id: &str) -> String {
    id.trim().to_ascii_lowercase().replace(['_', ' '], "-")
}

impl Theme {
    /// Colour for a value that gets worse as it grows: green below `warn`,
    /// amber up to `danger`, red beyond.
    ///
    /// Used for CPU, congestion, frame budget, and drop rates so those all
    /// read the same way across panels.
    pub fn scale_up(&self, value: f64, warn: f64, danger: f64) -> Color {
        if value >= danger {
            self.danger
        } else if value >= warn {
            self.warning
        } else {
            self.success
        }
    }

    /// Colour for a value that gets worse as it *shrinks* — FPS against its
    /// target, free disk space.
    pub fn scale_down(&self, value: f64, warn: f64, danger: f64) -> Color {
        if value <= danger {
            self.danger
        } else if value <= warn {
            self.warning
        } else {
            self.success
        }
    }
}

/// Linear blend between two colours, `t` in `0.0..=1.0`.
///
/// Non-RGB colours (the `mono` theme's ANSI entries) have no numeric channels
/// to interpolate, so the endpoint nearest `t` is returned instead.
pub fn blend(from: Color, to: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            Color::Rgb(lerp(r1, r2, t), lerp(g1, g2, t), lerp(b1, b2, t))
        }
        _ if t < 0.5 => from,
        _ => to,
    }
}

fn lerp(from: u8, to: u8, t: f32) -> u8 {
    (f32::from(from) + (f32::from(to) - f32::from(from)) * t).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn theme_ids_are_unique() {
        let ids: HashSet<&str> = THEMES.iter().map(|theme| theme.id).collect();
        assert_eq!(ids.len(), THEMES.len(), "duplicate theme id");
    }

    #[test]
    fn every_theme_is_findable_by_its_own_id() {
        for theme in THEMES {
            let found = find(theme.id).unwrap_or_else(|| panic!("{} not found", theme.id));
            assert_eq!(found.id, theme.id);
            assert_eq!(
                index_of(theme.id),
                THEMES.iter().position(|t| t.id == theme.id).unwrap()
            );
        }
    }

    #[test]
    fn ids_are_kebab_case_and_labels_are_non_empty() {
        for theme in THEMES {
            assert!(!theme.label.is_empty(), "{} has no label", theme.id);
            assert_eq!(
                theme.id,
                normalize(theme.id),
                "{} is not normalized",
                theme.id
            );
            assert!(
                theme
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{} has an unexpected character",
                theme.id
            );
        }
    }

    #[test]
    fn lookup_is_case_and_separator_insensitive() {
        assert_eq!(find("Tokyo_Night").unwrap().id, "tokyo-night");
        assert_eq!(find("  NORD  ").unwrap().id, "nord");
        assert_eq!(find("Catppuccin Mocha").unwrap().id, "catppuccin-mocha");
    }

    #[test]
    fn unknown_ids_resolve_to_the_fallback() {
        assert!(find("does-not-exist").is_none());
        assert_eq!(resolve("does-not-exist").id, FALLBACK.id);
        assert_eq!(index_of("does-not-exist"), 0);
    }

    #[test]
    fn the_default_config_theme_exists() {
        // Keeps the shipped default and the theme list from drifting apart.
        let default_theme = crate::config::UiConfig::default().theme;
        assert!(find(&default_theme).is_some(), "{default_theme} is missing");
    }

    #[test]
    fn the_mono_theme_never_paints_a_background() {
        let mono = find("mono").unwrap();
        assert_eq!(mono.bg, Color::Reset, "mono must inherit the terminal bg");
    }

    #[test]
    fn selected_rows_stay_readable() {
        // A highlight that reuses the body foreground would be invisible.
        for theme in THEMES {
            assert_ne!(
                theme.highlight_bg, theme.highlight_fg,
                "{} has an unreadable selection",
                theme.id
            );
        }
    }

    #[test]
    fn scale_up_moves_green_to_red_as_the_value_grows() {
        let theme = FALLBACK;
        assert_eq!(theme.scale_up(10.0, 50.0, 80.0), theme.success);
        assert_eq!(theme.scale_up(60.0, 50.0, 80.0), theme.warning);
        assert_eq!(theme.scale_up(95.0, 50.0, 80.0), theme.danger);
        // Boundaries belong to the more severe band.
        assert_eq!(theme.scale_up(50.0, 50.0, 80.0), theme.warning);
        assert_eq!(theme.scale_up(80.0, 50.0, 80.0), theme.danger);
    }

    #[test]
    fn scale_down_moves_green_to_red_as_the_value_shrinks() {
        let theme = FALLBACK;
        assert_eq!(theme.scale_down(60.0, 55.0, 30.0), theme.success);
        assert_eq!(theme.scale_down(50.0, 55.0, 30.0), theme.warning);
        assert_eq!(theme.scale_down(10.0, 55.0, 30.0), theme.danger);
    }

    #[test]
    fn blend_interpolates_rgb_endpoints() {
        let black = Color::Rgb(0, 0, 0);
        let white = Color::Rgb(255, 255, 255);
        assert_eq!(blend(black, white, 0.0), black);
        assert_eq!(blend(black, white, 1.0), white);
        assert_eq!(blend(black, white, 0.5), Color::Rgb(128, 128, 128));
        // Out-of-range factors clamp rather than wrapping.
        assert_eq!(blend(black, white, 5.0), white);
        assert_eq!(blend(black, white, -1.0), black);
    }

    #[test]
    fn blend_snaps_to_an_endpoint_for_ansi_colors() {
        assert_eq!(blend(Color::Red, Color::Blue, 0.2), Color::Red);
        assert_eq!(blend(Color::Red, Color::Blue, 0.8), Color::Blue);
    }
}
