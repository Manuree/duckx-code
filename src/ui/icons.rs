//! Nerd Font glyphs used by the status bar and sidebar. Requires a Nerd Font in the terminal.

use std::path::Path;

use ratatui::style::Color;

/// Git branch glyph, shown on the status line while a repository is open.
pub const BRANCH: &str = "\u{e0a0}";
/// Powerline separator for sections growing left-to-right.
pub const SEPARATOR_LEFT: &str = "\u{e0b0}";
/// Powerline separator for sections growing right-to-left.
pub const SEPARATOR_RIGHT: &str = "\u{e0b2}";
/// Magnifying glass (`nf-fa-search`), the sidebar filter prompt.
pub const SEARCH: &str = "\u{f002}";
pub const FOLDER_CLOSED: &str = "\u{f07b}";
pub const FOLDER_OPEN: &str = "\u{f07c}";

const DEFAULT_FILE: (&str, Color) = ("\u{f15b}", Color::Rgb(0x6d, 0x80, 0x86));

/// Icon and color for a recognized file type. Unknown names and untitled buffers get none.
pub fn known_file_icon(path: Option<&Path>) -> Option<(&'static str, Color)> {
    let extension = path
        .and_then(Path::extension)
        .and_then(|ext| ext.to_str())?;
    let icon = match extension.to_ascii_lowercase().as_str() {
        "rs" => ("\u{e7a8}", Color::Rgb(0xde, 0xa5, 0x84)),
        "md" | "markdown" => ("\u{e73e}", Color::Rgb(0xdd, 0xdd, 0xdd)),
        "toml" | "yaml" | "yml" | "ini" => ("\u{e615}", Color::Rgb(0x6d, 0x80, 0x86)),
        "json" => ("\u{e60b}", Color::Rgb(0xcb, 0xcb, 0x41)),
        "js" | "mjs" | "cjs" => ("\u{e74e}", Color::Rgb(0xcb, 0xcb, 0x41)),
        "ts" | "tsx" => ("\u{e628}", Color::Rgb(0x51, 0x9a, 0xba)),
        "py" => ("\u{e73c}", Color::Rgb(0xff, 0xbc, 0x03)),
        "lock" => ("\u{f023}", Color::Rgb(0xbb, 0xbb, 0xbb)),
        _ => return None,
    };
    Some(icon)
}

/// Icon and color for a file, following nvim-web-devicons conventions.
/// Unrecognized names use a plain file glyph.
pub fn file_icon(path: Option<&Path>) -> (&'static str, Color) {
    known_file_icon(path).unwrap_or(DEFAULT_FILE)
}
