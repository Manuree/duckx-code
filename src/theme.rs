//! Color themes (interface) and syntax themes (code), loaded from JSON.
//!
//! Built-in documents live in `themes/color` and `themes/syntax` and are embedded
//! in the binary. Files in the user config directory overlay them by `id`.
//! The chosen ids are stored in `settings.json`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ratatui::style::{Color, Modifier, Style};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

const COLOR_THEME_JSON: &[&str] = &[
    include_str!("../themes/color/tokyo-night.json"),
    include_str!("../themes/color/catppuccin-latte.json"),
    include_str!("../themes/color/catppuccin-frappe.json"),
    include_str!("../themes/color/catppuccin-macchiato.json"),
    include_str!("../themes/color/catppuccin-mocha.json"),
];

const SYNTAX_THEME_JSON: &[&str] = &[
    include_str!("../themes/syntax/tokyo-night.json"),
    include_str!("../themes/syntax/catppuccin-latte.json"),
    include_str!("../themes/syntax/catppuccin-frappe.json"),
    include_str!("../themes/syntax/catppuccin-macchiato.json"),
    include_str!("../themes/syntax/catppuccin-mocha.json"),
];

const DEFAULT_ID: &str = "tokyo-night";

/// `#rrggbb` color stored in a theme file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl From<Rgb> for Color {
    fn from(color: Rgb) -> Self {
        Color::Rgb(color.r, color.g, color.b)
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b))
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_rgb(&text).map_err(de::Error::custom)
    }
}

fn parse_rgb(text: &str) -> Result<Rgb, String> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("expected #rrggbb, found {text}"));
    }
    let value =
        u32::from_str_radix(hex, 16).map_err(|_| format!("expected #rrggbb, found {text}"))?;
    Ok(Rgb::new(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ))
}

/// Interface colors. One JSON document under `themes/color`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorTheme {
    pub id: String,
    pub name: String,
    pub editor: EditorColors,
    pub status: StatusColors,
    pub sidebar: SidebarColors,
    pub git: GitColors,
    pub chrome: ChromeColors,
    /// Language server diagnostics. A theme file without this section uses the Tokyo Night colors.
    #[serde(default)]
    pub diagnostics: DiagnosticColors,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticColors {
    pub error: Rgb,
    pub warning: Rgb,
    pub info: Rgb,
    pub hint: Rgb,
}

impl Default for DiagnosticColors {
    fn default() -> Self {
        Self {
            error: Rgb::new(0xdb, 0x4b, 0x4b),
            warning: Rgb::new(0xe0, 0xaf, 0x68),
            info: Rgb::new(0x0d, 0xb9, 0xd7),
            hint: Rgb::new(0x1a, 0xbc, 0x9c),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorColors {
    pub bg: Rgb,
    pub fg: Rgb,
    pub gutter: Rgb,
    pub gutter_active: Rgb,
    pub selection: Rgb,
    pub divider: Rgb,
    /// Background of a matched bracket pair at the cursor. Missing uses [`Self::divider`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bracket_match: Option<Rgb>,
}

fn default_select_color() -> Rgb {
    Rgb::new(0xff, 0x9e, 0x64)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusColors {
    pub badge_fg: Rgb,
    pub normal: Rgb,
    pub command: Rgb,
    pub settings: Rgb,
    /// Badge while the explorer is in FILE mode. Missing keys keep the Tokyo Night orange.
    #[serde(default = "default_select_color")]
    pub select: Rgb,
    pub b_bg: Rgb,
    pub b_fg: Rgb,
    pub c_bg: Rgb,
    pub c_fg: Rgb,
    pub dim: Rgb,
    pub message: Rgb,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidebarColors {
    pub bg: Rgb,
    pub fg: Rgb,
    pub dim: Rgb,
    pub guide: Rgb,
    pub folder: Rgb,
    pub border: Rgb,
    pub border_focused: Rgb,
    pub prompt: Rgb,
    pub r#match: Rgb,
    pub selected: Rgb,
    pub selected_unfocused: Rgb,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitColors {
    pub untracked: Rgb,
    pub modified: Rgb,
    pub added: Rgb,
    pub deleted: Rgb,
    pub renamed: Rgb,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChromeColors {
    pub title: Rgb,
    pub title_active: Rgb,
    pub hint_bg: Rgb,
    pub hint_fg: Rgb,
    pub hint_dim: Rgb,
    pub hint_active_bg: Rgb,
    pub hint_active_name: Rgb,
    pub dialog_button: Rgb,
    pub dialog_active_fg: Rgb,
    pub dialog_active_bg: Rgb,
    pub dialog_message: Rgb,
    pub command_border: Rgb,
    pub command_prompt: Rgb,
}

/// Token colors. One JSON document under `themes/syntax`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntaxTheme {
    pub id: String,
    pub name: String,
    pub text: Rgb,
    pub comment: Rgb,
    pub keyword: Rgb,
    pub function: Rgb,
    pub r#type: Rgb,
    pub string: Rgb,
    pub number: Rgb,
    pub constant: Rgb,
    pub operator: Rgb,
    pub punctuation: Rgb,
    pub variable: Rgb,
    pub parameter: Rgb,
    pub property: Rgb,
    pub tag: Rgb,
    pub attribute: Rgb,
    pub error: Rgb,
    /// Doc comments. Missing in a theme file uses [`Self::comment`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Rgb>,
    /// `function.macro`. Missing uses [`Self::function`].
    #[serde(default, rename = "macro", skip_serializing_if = "Option::is_none")]
    pub macro_color: Option<Rgb>,
    /// Built-in types and `defaultLibrary` tokens. Missing uses [`Self::r#type`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<Rgb>,
    /// String escapes. Missing uses [`Self::constant`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escape: Option<Rgb>,
    /// Lifetimes. Missing uses [`Self::attribute`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<Rgb>,
    /// Bracket pair colors by nesting depth, cycled. Missing or empty uses
    /// [`Self::constant`], [`Self::keyword`], [`Self::function`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brackets: Option<Vec<Rgb>>,
    /// An unmatched bracket. Missing uses [`Self::error`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bracket_error: Option<Rgb>,
}

/// Roles a highlighter can ask the syntax theme to color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxRole {
    Text,
    Comment,
    Keyword,
    Function,
    Type,
    String,
    Number,
    Constant,
    Operator,
    Punctuation,
    Variable,
    Parameter,
    Property,
    Tag,
    Attribute,
    Error,
    Documentation,
    Macro,
    Builtin,
    Escape,
    Lifetime,
}

impl SyntaxTheme {
    pub fn color(&self, role: SyntaxRole) -> Color {
        let rgb = match role {
            SyntaxRole::Text => self.text,
            SyntaxRole::Comment => self.comment,
            SyntaxRole::Keyword => self.keyword,
            SyntaxRole::Function => self.function,
            SyntaxRole::Type => self.r#type,
            SyntaxRole::String => self.string,
            SyntaxRole::Number => self.number,
            SyntaxRole::Constant => self.constant,
            SyntaxRole::Operator => self.operator,
            SyntaxRole::Punctuation => self.punctuation,
            SyntaxRole::Variable => self.variable,
            SyntaxRole::Parameter => self.parameter,
            SyntaxRole::Property => self.property,
            SyntaxRole::Tag => self.tag,
            SyntaxRole::Attribute => self.attribute,
            SyntaxRole::Error => self.error,
            SyntaxRole::Documentation => self.documentation.unwrap_or(self.comment),
            SyntaxRole::Macro => self.macro_color.unwrap_or(self.function),
            SyntaxRole::Builtin => self.builtin.unwrap_or(self.r#type),
            SyntaxRole::Escape => self.escape.unwrap_or(self.constant),
            SyntaxRole::Lifetime => self.lifetime.unwrap_or(self.attribute),
        };
        rgb.into()
    }
}

impl SyntaxTheme {
    /// Color for a bracket at nesting `depth` (0 is outermost); the palette repeats.
    pub fn bracket_color(&self, depth: usize) -> Color {
        match self.brackets.as_deref() {
            Some(palette) if !palette.is_empty() => palette[depth % palette.len()].into(),
            _ => {
                let palette = [self.constant, self.keyword, self.function];
                palette[depth % palette.len()].into()
            }
        }
    }

    /// Color for an unmatched bracket.
    pub fn bracket_error_color(&self) -> Color {
        self.bracket_error.unwrap_or(self.error).into()
    }
}

/// Styles the widgets read. Built from the active [`ColorTheme`].
#[derive(Clone, Copy, Debug)]
pub struct UiStyles {
    pub editor_bg: Color,
    pub editor_fg: Color,
    pub gutter: Style,
    pub gutter_active: Style,
    pub selection: Style,
    /// The bracket at the cursor and its partner: a background and bold.
    pub bracket_match: Style,
    pub status_a_bg: Color,
    pub status_b_bg: Color,
    pub status_c_bg: Color,
    pub status_a: Style,
    pub status_command_bg: Color,
    pub status_command: Style,
    pub status_settings_bg: Color,
    pub status_settings: Style,
    pub status_select_bg: Color,
    pub status_select: Style,
    pub status_find_bg: Color,
    pub status_find: Style,
    pub status_filter_bg: Color,
    pub status_filter: Style,
    pub status_b: Style,
    pub status_c: Style,
    pub status_c_dim: Style,
    pub status_c_bold: Style,
    pub status_message: Style,
    pub dim: Style,
    pub title: Style,
    pub title_active: Style,
    pub command_border: Style,
    pub command_prompt: Style,
    pub hint: Style,
    pub hint_dim: Style,
    pub hint_keys: Style,
    pub hint_active: Style,
    pub hint_active_name: Style,
    pub hint_active_dim: Style,
    pub sidebar: Style,
    pub sidebar_bg: Color,
    pub sidebar_fg: Color,
    pub sidebar_guide: Style,
    pub sidebar_dim: Style,
    pub sidebar_folder: Color,
    pub sidebar_root: Style,
    pub sidebar_border: Style,
    pub sidebar_border_focused: Style,
    pub sidebar_prompt: Style,
    pub sidebar_match: Style,
    pub sidebar_selected: Color,
    pub sidebar_selected_unfocused: Color,
    pub git_untracked: Color,
    pub git_modified: Color,
    pub git_added: Color,
    pub git_deleted: Color,
    pub git_renamed: Color,
    pub dialog_button: Style,
    pub dialog_button_active: Style,
    pub dialog_message: Style,
    pub diagnostic_error: Color,
    pub diagnostic_warning: Color,
    pub diagnostic_info: Color,
    pub diagnostic_hint: Color,
}

impl UiStyles {
    pub fn diagnostic(&self, severity: crate::lsp::Severity) -> Color {
        match severity {
            crate::lsp::Severity::Error => self.diagnostic_error,
            crate::lsp::Severity::Warning => self.diagnostic_warning,
            crate::lsp::Severity::Info => self.diagnostic_info,
            crate::lsp::Severity::Hint => self.diagnostic_hint,
        }
    }

    pub fn from_color(theme: &ColorTheme) -> Self {
        let editor = &theme.editor;
        let status = &theme.status;
        let side = &theme.sidebar;
        let git = &theme.git;
        let chrome = &theme.chrome;
        let badge = |bg: Rgb| bold(fg_bg(status.badge_fg, bg));
        let hint = fg_bg(chrome.hint_fg, chrome.hint_bg);
        let hint_dim = fg_bg(chrome.hint_dim, chrome.hint_bg);
        let hint_active = fg_bg(chrome.hint_fg, chrome.hint_active_bg);
        Self {
            editor_bg: editor.bg.into(),
            editor_fg: editor.fg.into(),
            gutter: fg_bg(editor.gutter, editor.bg),
            gutter_active: bold(fg_bg(editor.gutter_active, editor.bg)),
            selection: Style::new().bg(editor.selection.into()),
            bracket_match: bold(
                Style::new().bg(editor.bracket_match.unwrap_or(editor.divider).into()),
            ),
            status_a_bg: status.normal.into(),
            status_b_bg: status.b_bg.into(),
            status_c_bg: status.c_bg.into(),
            status_a: badge(status.normal),
            status_command_bg: status.command.into(),
            status_command: badge(status.command),
            status_settings_bg: status.settings.into(),
            status_settings: badge(status.settings),
            status_select_bg: status.select.into(),
            status_select: badge(status.select),
            status_find_bg: status.message.into(),
            status_find: badge(status.message),
            status_filter_bg: git.untracked.into(),
            status_filter: badge(git.untracked),
            status_b: fg_bg(status.b_fg, status.b_bg),
            status_c: fg_bg(status.c_fg, status.c_bg),
            status_c_dim: fg_bg(status.dim, status.c_bg),
            status_c_bold: bold(fg_bg(status.c_fg, status.c_bg)),
            status_message: bold(fg_bg(status.message, status.c_bg)),
            dim: fg_bg(editor.divider, editor.bg),
            title: fg_bg(chrome.title, editor.bg),
            title_active: bold(fg_bg(chrome.title_active, editor.bg)),
            command_border: fg(chrome.command_border),
            command_prompt: bold(fg(chrome.command_prompt)),
            hint,
            hint_dim,
            hint_keys: hint_dim,
            hint_active,
            hint_active_name: bold(fg_bg(chrome.hint_active_name, chrome.hint_active_bg)),
            hint_active_dim: fg_bg(chrome.hint_dim, chrome.hint_active_bg),
            sidebar: fg_bg(side.fg, side.bg),
            sidebar_bg: side.bg.into(),
            sidebar_fg: side.fg.into(),
            sidebar_guide: fg(side.guide),
            sidebar_dim: fg(side.dim),
            sidebar_folder: side.folder.into(),
            sidebar_root: bold(fg(side.folder)),
            sidebar_border: fg(side.border),
            sidebar_border_focused: fg(side.border_focused),
            sidebar_prompt: fg(side.prompt),
            sidebar_match: bold(fg(side.r#match)),
            sidebar_selected: side.selected.into(),
            sidebar_selected_unfocused: side.selected_unfocused.into(),
            git_untracked: git.untracked.into(),
            git_modified: git.modified.into(),
            git_added: git.added.into(),
            git_deleted: git.deleted.into(),
            git_renamed: git.renamed.into(),
            dialog_button: fg(chrome.dialog_button),
            dialog_button_active: bold(fg_bg(chrome.dialog_active_fg, chrome.dialog_active_bg)),
            dialog_message: bold(fg(chrome.dialog_message)),
            diagnostic_error: theme.diagnostics.error.into(),
            diagnostic_warning: theme.diagnostics.warning.into(),
            diagnostic_info: theme.diagnostics.info.into(),
            diagnostic_hint: theme.diagnostics.hint.into(),
        }
    }
}

fn fg(color: Rgb) -> Style {
    Style::new().fg(color.into())
}

fn fg_bg(color: Rgb, bg: Rgb) -> Style {
    Style::new().fg(color.into()).bg(bg.into())
}

fn bold(style: Style) -> Style {
    style.add_modifier(Modifier::BOLD)
}

#[derive(Clone, Debug)]
struct Catalog {
    colors: Vec<ColorTheme>,
    syntax: Vec<SyntaxTheme>,
}

/// Active themes plus the catalog they were chosen from.
#[derive(Clone, Debug)]
pub struct Appearance {
    pub color: ColorTheme,
    pub syntax: SyntaxTheme,
    pub styles: UiStyles,
    catalog: Catalog,
    config_dir: Option<PathBuf>,
    pub lsp: crate::settings::LspPrefs,
}

impl Appearance {
    /// Embedded themes, with Tokyo Night selected. Does not read the disk.
    pub fn builtin() -> Self {
        let catalog = Catalog {
            colors: builtin_colors(),
            syntax: builtin_syntax(),
        };
        let color = pick(&catalog.colors, DEFAULT_ID);
        let syntax = pick_syntax(&catalog.syntax, DEFAULT_ID);
        let styles = UiStyles::from_color(&color);
        Self {
            color,
            syntax,
            styles,
            catalog,
            config_dir: None,
            lsp: crate::settings::LspPrefs::default(),
        }
    }

    /// Built-ins, then JSON files under `dir/themes`, then `dir/settings.json`.
    /// A missing config directory is the built-in default, not an error.
    /// The returned document is sanitized: a bad tab width or chord is replaced.
    pub fn load(dir: &Path) -> (Self, crate::settings::SettingsFile, Option<String>) {
        let mut appearance = Self::builtin();
        appearance.config_dir = Some(dir.to_path_buf());
        let mut warnings = Vec::new();
        appearance.overlay_user_themes(dir, &mut warnings);
        let file = match crate::settings::SettingsFile::read(&dir.join("settings.json")) {
            Ok(None) => crate::settings::SettingsFile::default(),
            Ok(Some(mut file)) => {
                warnings.extend(file.sanitize());
                if let Err(err) = appearance.select(&file.color_theme, &file.syntax_theme) {
                    warnings.push(err);
                }
                appearance.lsp = file.lsp.clone();
                file
            }
            Err(err) => {
                warnings.push(err);
                crate::settings::SettingsFile::default()
            }
        };
        let warning = (!warnings.is_empty()).then(|| warnings.join(" "));
        (appearance, file, warning)
    }

    /// `Some` when a config directory was set by [`Self::load`].
    /// Theme tests write the theme ids. The app writes editor, files, and keybindings too.
    pub fn settings_document(&self) -> Option<(PathBuf, String)> {
        let dir = self.config_dir.as_ref()?;
        let mut file = crate::settings::SettingsFile::default();
        file.color_theme = self.color.id.clone();
        file.syntax_theme = self.syntax.id.clone();
        file.lsp = self.lsp.clone();
        Some(file.document(dir))
    }

    pub fn config_dir(&self) -> Option<&Path> {
        self.config_dir.as_deref()
    }

    /// Selects the next color theme. Returns `false` when the choice does not change.
    pub fn cycle_color(&mut self, step: isize) -> bool {
        let Some(next) = step_by(&self.catalog.colors, &self.color.id, step, |theme| {
            &theme.id
        }) else {
            return false;
        };
        if next.id == self.color.id {
            return false;
        }
        self.set_color(next);
        true
    }

    pub fn color_count(&self) -> usize {
        self.catalog.colors.len()
    }

    pub fn syntax_count(&self) -> usize {
        self.catalog.syntax.len()
    }

    pub fn color_index(&self) -> usize {
        self.catalog
            .colors
            .iter()
            .position(|theme| theme.id == self.color.id)
            .unwrap_or(0)
    }

    pub fn syntax_index(&self) -> usize {
        self.catalog
            .syntax
            .iter()
            .position(|theme| theme.id == self.syntax.id)
            .unwrap_or(0)
    }

    pub fn color_names(&self) -> Vec<String> {
        self.catalog
            .colors
            .iter()
            .map(|theme| theme.name.clone())
            .collect()
    }

    pub fn syntax_names(&self) -> Vec<String> {
        self.catalog
            .syntax
            .iter()
            .map(|theme| theme.name.clone())
            .collect()
    }

    /// Selects a color theme by its place in [`Self::color_names`].
    /// Returns `false` when `index` is the theme already in use.
    pub fn select_color_index(&mut self, index: usize) -> bool {
        let Some(theme) = self.catalog.colors.get(index).cloned() else {
            return false;
        };
        if theme.id == self.color.id {
            return false;
        }
        self.set_color(theme);
        true
    }

    /// Selects a syntax theme by its place in [`Self::syntax_names`].
    pub fn select_syntax_index(&mut self, index: usize) -> bool {
        let Some(theme) = self.catalog.syntax.get(index).cloned() else {
            return false;
        };
        if theme.id == self.syntax.id {
            return false;
        }
        self.syntax = theme;
        true
    }

    pub fn select_color_id(&mut self, id: &str) -> bool {
        let Some(index) = self.catalog.colors.iter().position(|theme| theme.id == id) else {
            return false;
        };
        self.select_color_index(index)
    }

    pub fn select_syntax_id(&mut self, id: &str) -> bool {
        let Some(index) = self.catalog.syntax.iter().position(|theme| theme.id == id) else {
            return false;
        };
        self.select_syntax_index(index)
    }

    /// Selects the next syntax theme. Returns `false` when the choice does not change.
    pub fn cycle_syntax(&mut self, step: isize) -> bool {
        let Some(next) = step_by(&self.catalog.syntax, &self.syntax.id, step, |theme| {
            &theme.id
        }) else {
            return false;
        };
        if next.id == self.syntax.id {
            return false;
        }
        self.syntax = next;
        true
    }

    fn set_color(&mut self, color: ColorTheme) {
        self.styles = UiStyles::from_color(&color);
        self.color = color;
    }

    fn select(&mut self, color_id: &str, syntax_id: &str) -> Result<(), String> {
        let mut missing = Vec::new();
        if let Some(theme) = self
            .catalog
            .colors
            .iter()
            .find(|theme| theme.id == color_id)
            .cloned()
        {
            self.set_color(theme);
        } else {
            missing.push(format!("color theme \"{color_id}\""));
        }
        if let Some(theme) = self
            .catalog
            .syntax
            .iter()
            .find(|theme| theme.id == syntax_id)
            .cloned()
        {
            self.syntax = theme;
        } else {
            missing.push(format!("syntax theme \"{syntax_id}\""));
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!("Unknown {}", missing.join(" and ")))
        }
    }

    fn overlay_user_themes(&mut self, dir: &Path, warnings: &mut Vec<String>) {
        let root = dir.join("themes");
        for theme in read_theme_dir::<ColorTheme>(&root.join("color"), warnings) {
            if !valid_id(&theme.id) {
                warnings.push(format!("Ignored color theme with id \"{}\"", theme.id));
                continue;
            }
            upsert(&mut self.catalog.colors, theme, |item| &item.id);
        }
        for theme in read_theme_dir::<SyntaxTheme>(&root.join("syntax"), warnings) {
            if !valid_id(&theme.id) {
                warnings.push(format!("Ignored syntax theme with id \"{}\"", theme.id));
                continue;
            }
            upsert(&mut self.catalog.syntax, theme, |item| &item.id);
        }
    }
}

pub fn builtin_colors() -> Vec<ColorTheme> {
    parse_builtins(COLOR_THEME_JSON, "color")
}

pub fn builtin_syntax() -> Vec<SyntaxTheme> {
    parse_builtins(SYNTAX_THEME_JSON, "syntax")
}

fn parse_builtins<T: for<'de> Deserialize<'de>>(files: &[&str], kind: &str) -> Vec<T> {
    files
        .iter()
        .enumerate()
        .map(|(index, text)| {
            serde_json::from_str(text).unwrap_or_else(|err| {
                panic!("builtin {kind} theme {index} is not valid JSON: {err}")
            })
        })
        .collect()
}

fn pick(themes: &[ColorTheme], id: &str) -> ColorTheme {
    themes
        .iter()
        .find(|theme| theme.id == id)
        .cloned()
        .or_else(|| themes.first().cloned())
        .expect("at least one color theme is embedded")
}

fn pick_syntax(themes: &[SyntaxTheme], id: &str) -> SyntaxTheme {
    themes
        .iter()
        .find(|theme| theme.id == id)
        .cloned()
        .or_else(|| themes.first().cloned())
        .expect("at least one syntax theme is embedded")
}

fn step_by<T: Clone>(
    themes: &[T],
    current: &str,
    step: isize,
    id: impl Fn(&T) -> &str,
) -> Option<T> {
    if themes.is_empty() || step == 0 {
        return None;
    }
    let index = themes
        .iter()
        .position(|theme| id(theme) == current)
        .unwrap_or(0);
    let next = (index as isize + step).rem_euclid(themes.len() as isize) as usize;
    Some(themes[next].clone())
}

fn upsert<T>(themes: &mut Vec<T>, incoming: T, id: impl Fn(&T) -> &str) {
    let incoming_id = id(&incoming).to_string();
    if let Some(slot) = themes.iter_mut().find(|theme| id(theme) == incoming_id) {
        *slot = incoming;
    } else {
        themes.push(incoming);
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.contains(['/', '\\', '\0']) && id.chars().all(|ch| !ch.is_whitespace())
}

fn read_theme_dir<T: for<'de> Deserialize<'de>>(dir: &Path, warnings: &mut Vec<String>) -> Vec<T> {
    let mut paths = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect::<Vec<_>>(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            warnings.push(format!("Cannot read {}: {err}", dir.display()));
            return Vec::new();
        }
    };
    paths.sort();
    let mut themes = Vec::new();
    for path in paths {
        match fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(theme) => themes.push(theme),
                Err(err) => warnings.push(format!("Cannot load {}: {err}", path.display())),
            },
            Err(err) => warnings.push(format!("Cannot read {}: {err}", path.display())),
        }
    }
    themes
}

/// Directory that holds `settings.json` and the user's `themes` folder.
/// `APPDATA` when it is set, otherwise `XDG_CONFIG_HOME` or `~/.config`.
pub fn config_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("duckx-code");
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("duckx-code");
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(home).join(".config").join("duckx-code");
    }
    PathBuf::from(".duckx-code")
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;

    use super::*;

    fn temp_dir() -> (PathBuf, impl Drop) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("duckx-theme-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        struct Guard(PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        (path.clone(), Guard(path))
    }

    #[test]
    fn every_builtin_theme_sets_the_bracket_colors() {
        for theme in builtin_syntax() {
            let palette = theme.brackets.as_deref().unwrap_or_default();
            assert_eq!(palette.len(), 3, "{}", theme.id);
            assert!(theme.bracket_error.is_some(), "{}", theme.id);
            assert_ne!(
                theme.bracket_color(0),
                theme.bracket_color(1),
                "{}",
                theme.id
            );
        }
        for theme in builtin_colors() {
            let match_bg = theme.editor.bracket_match.expect("bracket_match");
            assert_ne!(match_bg, theme.editor.selection, "{}", theme.id);
            assert_ne!(match_bg, theme.editor.bg, "{}", theme.id);
        }
    }

    #[test]
    fn themes_without_bracket_keys_still_load_and_fall_back() {
        let mut syntax: serde_json::Value = serde_json::from_str(SYNTAX_THEME_JSON[0]).unwrap();
        let object = syntax.as_object_mut().unwrap();
        object.remove("brackets");
        object.remove("bracket_error");
        let syntax: SyntaxTheme = serde_json::from_value(syntax).unwrap();
        assert_eq!(syntax.bracket_color(0), Color::from(syntax.constant));
        assert_eq!(syntax.bracket_color(1), Color::from(syntax.keyword));
        assert_eq!(syntax.bracket_color(2), Color::from(syntax.function));
        assert_eq!(syntax.bracket_error_color(), Color::from(syntax.error));
        let empty = SyntaxTheme {
            brackets: Some(Vec::new()),
            ..syntax.clone()
        };
        assert_eq!(empty.bracket_color(4), syntax.bracket_color(4));

        let mut color: serde_json::Value = serde_json::from_str(COLOR_THEME_JSON[0]).unwrap();
        color["editor"]
            .as_object_mut()
            .unwrap()
            .remove("bracket_match");
        let color: ColorTheme = serde_json::from_value(color).unwrap();
        let styles = UiStyles::from_color(&color);
        assert_eq!(styles.bracket_match.bg, Some(color.editor.divider.into()));
        assert!(styles.bracket_match.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn builtins_round_trip_and_pair_catppuccin_flavors() {
        let colors = builtin_colors();
        let syntax = builtin_syntax();
        assert_eq!(colors.len(), 5);
        assert_eq!(
            colors
                .iter()
                .map(|theme| theme.id.as_str())
                .collect::<Vec<_>>(),
            syntax
                .iter()
                .map(|theme| theme.id.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(colors[0].id, "tokyo-night");
        assert_eq!(colors[0].editor.fg, Rgb::new(0xa9, 0xb1, 0xd6));
        assert_eq!(colors[0].editor.selection, Rgb::new(0x2b, 0x2f, 0x41));
        assert_eq!(colors[0].editor.gutter_active, Rgb::new(0xe0, 0xaf, 0x68));
        assert_eq!(colors[0].sidebar.selected, Rgb::new(0x20, 0x23, 0x30));
        assert_eq!(colors[0].status.select, Rgb::new(0xff, 0x9e, 0x64));
        assert_eq!(colors[0].git.modified, Rgb::new(0x61, 0x83, 0xbb));
        let night = syntax
            .iter()
            .find(|theme| theme.id == "tokyo-night")
            .unwrap();
        assert_eq!(night.text, Rgb::new(0xa9, 0xb1, 0xd6));
        assert_eq!(night.comment, Rgb::new(0x51, 0x59, 0x7d));
        assert_eq!(night.punctuation, Rgb::new(0x89, 0xdd, 0xff));
        assert_eq!(night.r#type, Rgb::new(0x0d, 0xb9, 0xd7));
        assert_eq!(night.property, Rgb::new(0x7d, 0xcf, 0xff));
        assert_eq!(night.error, Rgb::new(0xdb, 0x4b, 0x4b));
        let mocha = syntax
            .iter()
            .find(|theme| theme.id == "catppuccin-mocha")
            .unwrap();
        assert_eq!(mocha.name, "Catppuccin Mocha");
        assert_eq!(mocha.keyword, Rgb::new(0xcb, 0xa6, 0xf7));
        assert_eq!(mocha.string, Rgb::new(0xa6, 0xe3, 0xa1));
        assert_eq!(
            mocha.color(SyntaxRole::Keyword),
            Color::Rgb(0xcb, 0xa6, 0xf7)
        );
        for theme in &colors {
            let text = serde_json::to_string(theme).unwrap();
            let back: ColorTheme = serde_json::from_str(&text).unwrap();
            assert_eq!(theme, &back);
        }
        for theme in &syntax {
            let text = serde_json::to_string(theme).unwrap();
            let back: SyntaxTheme = serde_json::from_str(&text).unwrap();
            assert_eq!(theme, &back);
        }
        let styles = UiStyles::from_color(&colors[4]);
        assert_eq!(styles.sidebar_bg, Color::Rgb(0x18, 0x18, 0x25));
        assert_eq!(styles.editor_bg, Color::Rgb(0x1e, 0x1e, 0x2e));
        assert!(styles.gutter_active.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn hex_accepts_a_hash_and_rejects_short_values() {
        assert_eq!(parse_rgb("#CBA6F7").unwrap(), Rgb::new(0xcb, 0xa6, 0xf7));
        assert_eq!(parse_rgb("1a1b26").unwrap(), Rgb::new(0x1a, 0x1b, 0x26));
        assert!(parse_rgb("#fff").is_err());
        assert!(parse_rgb("blue").is_err());
        assert!(serde_json::from_str::<ColorTheme>(r#"{"id":"x","name":"X"}"#).is_err());
    }

    #[test]
    fn user_json_overrides_a_builtin_and_settings_round_trip() {
        let (dir, _guard) = temp_dir();
        let (mut appearance, _file, warning) = Appearance::load(&dir);
        assert!(warning.is_none());
        assert_eq!(appearance.color.id, "tokyo-night");
        assert!(appearance.cycle_color(1));
        assert_eq!(appearance.color.id, "catppuccin-latte");
        assert!(appearance.cycle_syntax(4));
        assert_eq!(appearance.syntax.id, "catppuccin-mocha");
        assert!(!appearance.cycle_color(0));

        let mut custom = appearance
            .catalog
            .colors
            .iter()
            .find(|theme| theme.id == "catppuccin-mocha")
            .unwrap()
            .clone();
        custom.name = "Mocha Custom".into();
        let color_dir = dir.join("themes").join("color");
        fs::create_dir_all(&color_dir).unwrap();
        fs::write(
            color_dir.join("mocha.json"),
            serde_json::to_string_pretty(&custom).unwrap(),
        )
        .unwrap();
        fs::write(color_dir.join("broken.json"), "{").unwrap();
        fs::write(
            color_dir.join("extra.json"),
            serde_json::to_string_pretty(&{
                let mut extra = custom.clone();
                extra.id = "extra".into();
                extra.name = "Extra".into();
                extra
            })
            .unwrap(),
        )
        .unwrap();

        let (path, text) = appearance.settings_document().unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();

        let (loaded, _file, warning) = Appearance::load(&dir);
        let warning = warning.expect("broken theme file");
        assert!(warning.contains("broken.json"), "{warning}");
        assert_eq!(loaded.color.id, "catppuccin-latte");
        assert_eq!(loaded.syntax.id, "catppuccin-mocha");
        let mocha = loaded
            .catalog
            .colors
            .iter()
            .find(|theme| theme.id == "catppuccin-mocha")
            .unwrap();
        assert_eq!(mocha.name, "Mocha Custom");
        assert!(
            loaded
                .catalog
                .colors
                .iter()
                .any(|theme| theme.id == "extra")
        );
    }

    #[test]
    fn unknown_settings_id_keeps_the_default() {
        let (dir, _guard) = temp_dir();
        fs::write(
            dir.join("settings.json"),
            r#"{"color_theme":"nope","syntax_theme":"tokyo-night"}"#,
        )
        .unwrap();
        let (appearance, _file, warning) = Appearance::load(&dir);
        assert_eq!(appearance.color.id, "tokyo-night");
        assert_eq!(appearance.syntax.id, "tokyo-night");
        assert!(warning.unwrap().contains("nope"));
    }
}
