/// Settings page and the `settings.json` document.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::keys::{self, AssignError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingId {
    /// Spaces inserted for a Tab, and the width of a tab character.
    TabWidth,
    /// Use spaces instead of a tab character.
    IndentWithSpaces,
    /// Ending written when a file is saved. Auto keeps the ending the file already has.
    LineEnding,
    /// Interface colors. `Enter` opens the theme list.
    ColorTheme,
    /// Code colors. `Enter` opens the theme list.
    SyntaxTheme,
    /// Remove spaces at the end of a line on save.
    TrimTrailingSpace,
    /// Add a newline at the end of a file on save.
    FinalNewline,
    /// One rebindable command chord.
    Binding(&'static str),
    /// Start language servers for open files.
    LanguageServer,
    /// rust-analyzer runs `cargo check` when a file is saved.
    CheckOnSave,
    /// Format the buffer before writing it.
    FormatOnSave,
    /// How strictly pyright checks an open Python file.
    PythonChecking,
    /// The assistant may call tools. Off refuses a turn.
    AiEnabled,
    /// Which default server address the assistant uses.
    AiProvider,
    /// OpenAI-compatible base URL, including `/v1`.
    AiServer,
    /// Model name sent as `model`.
    AiModel,
    /// Mutating tools run without asking Allow / Deny.
    AiAutoApprove,
    /// Optional Bearer token for OpenAI-compatible servers.
    AiApiKey,
    /// Stream assistant tokens with SSE; falls back when the server cannot.
    AiStreaming,
    /// Prompt context window size in tokens (Ollama `num_ctx`). `auto` uses 16384.
    AiContextSize,
}

/// How a new line is ended. `Auto` keeps the ending detected in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LineEndingPref {
    #[default]
    Auto,
    Lf,
    Crlf,
}

impl LineEndingPref {
    fn index(self) -> usize {
        match self {
            Self::Auto => 0,
            Self::Lf => 1,
            Self::Crlf => 2,
        }
    }

    fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Lf,
            2 => Self::Crlf,
            _ => Self::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Lf => "lf",
            Self::Crlf => "crlf",
        }
    }

    /// `None` while the file's own ending is kept.
    pub fn forced(self) -> Option<crate::buffer::LineEnding> {
        match self {
            Self::Auto => None,
            Self::Lf => Some(crate::buffer::LineEnding::Lf),
            Self::Crlf => Some(crate::buffer::LineEnding::Crlf),
        }
    }
}

/// How pyright and basedpyright check an open file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PythonCheck {
    Off,
    #[default]
    Basic,
    Standard,
}

impl PythonCheck {
    const CHOICES: [&str; 3] = ["Off", "Basic", "Standard"];

    fn index(self) -> usize {
        match self {
            Self::Off => 0,
            Self::Basic => 1,
            Self::Standard => 2,
        }
    }

    fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Off,
            2 => Self::Standard,
            _ => Self::Basic,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Basic => "basic",
            Self::Standard => "standard",
        }
    }
}

/// Language-server choices stored in `settings.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspPrefs {
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default = "enabled")]
    pub check_on_save: bool,
    #[serde(default)]
    pub format_on_save: bool,
    #[serde(default)]
    pub python: PythonCheck,
}

fn enabled() -> bool {
    true
}

impl Default for LspPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            check_on_save: true,
            format_on_save: false,
            python: PythonCheck::Basic,
        }
    }
}

impl LspPrefs {
    pub fn choice_names(id: SettingId) -> Vec<String> {
        let names: &[&str] = match id {
            SettingId::LanguageServer | SettingId::CheckOnSave | SettingId::FormatOnSave => {
                &["On", "Off"]
            }
            SettingId::PythonChecking => &PythonCheck::CHOICES,
            _ => &[],
        };
        names.iter().map(|name| (*name).to_string()).collect()
    }

    pub fn choice_index(&self, id: SettingId) -> usize {
        match id {
            SettingId::LanguageServer => usize::from(!self.enabled),
            SettingId::CheckOnSave => usize::from(!self.check_on_save),
            SettingId::FormatOnSave => usize::from(!self.format_on_save),
            SettingId::PythonChecking => self.python.index(),
            _ => 0,
        }
    }

    /// Value restored when the list is dismissed.
    pub fn choice_token(&self, id: SettingId) -> String {
        match id {
            SettingId::LanguageServer => on_off(self.enabled),
            SettingId::CheckOnSave => on_off(self.check_on_save),
            SettingId::FormatOnSave => on_off(self.format_on_save),
            SettingId::PythonChecking => self.python.as_str().to_string(),
            _ => String::new(),
        }
    }

    pub fn choice_label(&self, id: SettingId) -> String {
        Self::choice_names(id)
            .get(self.choice_index(id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_choice(&mut self, id: SettingId, index: usize) {
        match id {
            SettingId::LanguageServer => self.enabled = index == 0,
            SettingId::CheckOnSave => self.check_on_save = index == 0,
            SettingId::FormatOnSave => self.format_on_save = index == 0,
            SettingId::PythonChecking => self.python = PythonCheck::from_index(index),
            _ => {}
        }
    }

    pub fn restore_choice(&mut self, id: SettingId, token: &str) {
        match id {
            SettingId::LanguageServer => self.enabled = token != "off",
            SettingId::CheckOnSave => self.check_on_save = token != "off",
            SettingId::FormatOnSave => self.format_on_save = token != "off",
            SettingId::PythonChecking => {
                self.python = match token {
                    "off" => PythonCheck::Off,
                    "standard" => PythonCheck::Standard,
                    _ => PythonCheck::Basic,
                };
            }
            _ => {}
        }
    }
}

fn on_off(on: bool) -> String {
    if on { "on" } else { "off" }.to_string()
}

/// Which local server the assistant talks to. Both speak `/v1/chat/completions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    #[default]
    Ollama,
    Openai,
}

impl AiProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::Openai => "OpenAI-compatible / LM Studio",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::Openai => "openai",
        }
    }

    /// Base URL, including `/v1`, when the server row is empty.
    pub fn default_server(self) -> &'static str {
        match self {
            Self::Ollama => "http://127.0.0.1:11434/v1",
            Self::Openai => "http://127.0.0.1:1234/v1",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Ollama => 0,
            Self::Openai => 1,
        }
    }

    fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Openai,
            _ => Self::Ollama,
        }
    }
}

/// Assistant choices stored in `settings.json` under `ai`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiPrefs {
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub provider: AiProvider,
    /// Empty uses [`AiProvider::default_server`].
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub model: String,
    /// Optional Bearer token. Empty means no Authorization header.
    #[serde(default)]
    pub api_key: String,
    /// When true, apply_edit / create_file / save run without asking.
    #[serde(default)]
    pub auto_approve: bool,
    /// Try SSE streaming first (default on).
    #[serde(default = "enabled")]
    pub streaming: bool,
    /// Context window size sent to Ollama as `num_ctx`. Empty/`auto` → 16384.
    /// LM Studio ignores this (set context in LM Studio).
    #[serde(default = "default_context_size")]
    pub context_size: String,
}

fn default_context_size() -> String {
    "16384".to_string()
}

impl Default for AiPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: AiProvider::Ollama,
            server: String::new(),
            model: String::new(),
            api_key: String::new(),
            auto_approve: false,
            streaming: true,
            context_size: default_context_size(),
        }
    }
}

impl AiPrefs {
    pub fn is_text(id: SettingId) -> bool {
        matches!(id, SettingId::AiServer | SettingId::AiModel | SettingId::AiApiKey | SettingId::AiContextSize)
    }

    /// URL the assistant posts to, including `/v1`.
    pub fn endpoint(&self) -> String {
        let server = self.server.trim();
        if server.is_empty() {
            self.provider.default_server().to_string()
        } else {
            server.to_string()
        }
    }

    pub fn choice_names(id: SettingId) -> Vec<String> {
        match id {
            SettingId::AiEnabled | SettingId::AiAutoApprove | SettingId::AiStreaming => {
                vec!["On".into(), "Off".into()]
            }
            SettingId::AiProvider => vec![AiProvider::Ollama.label().into(), AiProvider::Openai.label().into()],
            _ => Vec::new(),
        }
    }

    pub fn choice_index(&self, id: SettingId) -> usize {
        match id {
            SettingId::AiEnabled => usize::from(!self.enabled),
            SettingId::AiAutoApprove => usize::from(!self.auto_approve),
            SettingId::AiStreaming => usize::from(!self.streaming),
            SettingId::AiProvider => self.provider.index(),
            _ => 0,
        }
    }

    pub fn choice_token(&self, id: SettingId) -> String {
        match id {
            SettingId::AiEnabled => on_off(self.enabled),
            SettingId::AiAutoApprove => on_off(self.auto_approve),
            SettingId::AiStreaming => on_off(self.streaming),
            SettingId::AiProvider => self.provider.as_str().to_string(),
            _ => String::new(),
        }
    }

    pub fn choice_label(&self, id: SettingId) -> String {
        Self::choice_names(id)
            .get(self.choice_index(id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_choice(&mut self, id: SettingId, index: usize) {
        match id {
            SettingId::AiEnabled => self.enabled = index == 0,
            SettingId::AiAutoApprove => self.auto_approve = index == 0,
            SettingId::AiStreaming => self.streaming = index == 0,
            SettingId::AiProvider => self.provider = AiProvider::from_index(index),
            _ => {}
        }
    }

    pub fn restore_choice(&mut self, id: SettingId, token: &str) {
        match id {
            SettingId::AiEnabled => self.enabled = token != "off",
            SettingId::AiAutoApprove => self.auto_approve = token != "off",
            SettingId::AiStreaming => self.streaming = token != "off",
            SettingId::AiProvider => {
                self.provider = if token == "openai" {
                    AiProvider::Openai
                } else {
                    AiProvider::Ollama
                };
            }
            _ => {}
        }
    }

    pub fn reset(&mut self, id: SettingId) {
        match id {
            SettingId::AiEnabled => self.enabled = true,
            SettingId::AiAutoApprove => self.auto_approve = false,
            SettingId::AiStreaming => self.streaming = true,
            SettingId::AiProvider => self.provider = AiProvider::Ollama,
            SettingId::AiServer => self.server.clear(),
            SettingId::AiModel => self.model.clear(),
            SettingId::AiApiKey => self.api_key.clear(),
            SettingId::AiContextSize => self.context_size = default_context_size(),
            _ => {}
        }
    }

    /// Tokens for the prompt window. `auto` / empty / invalid → 16384.
    pub fn resolved_context_size(&self) -> u32 {
        let raw = self.context_size.trim();
        if raw.is_empty() || raw.eq_ignore_ascii_case("auto") {
            return 16384;
        }
        raw.parse::<u32>()
            .ok()
            .filter(|n| (1024..=262_144).contains(n))
            .unwrap_or(16384)
    }
}

fn four() -> u8 {
    4
}

fn enabled_indent() -> bool {
    true
}

fn default_theme_id() -> String {
    "tokyo-night".to_string()
}

/// Indent and line endings. Missing fields keep the built-in defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorPrefs {
    #[serde(default = "four")]
    pub tab_width: u8,
    #[serde(default = "enabled_indent")]
    pub indent_with_spaces: bool,
    #[serde(default)]
    pub line_ending: LineEndingPref,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            tab_width: 4,
            indent_with_spaces: true,
            line_ending: LineEndingPref::Auto,
        }
    }
}

impl EditorPrefs {
    const TAB_WIDTHS: [u8; 3] = [2, 4, 8];

    pub fn choice_names(id: SettingId) -> Vec<String> {
        match id {
            SettingId::TabWidth => Self::TAB_WIDTHS
                .iter()
                .map(|width| width.to_string())
                .collect(),
            SettingId::IndentWithSpaces => vec!["On".into(), "Off".into()],
            SettingId::LineEnding => vec!["Auto".into(), "LF".into(), "CRLF".into()],
            _ => Vec::new(),
        }
    }

    pub fn choice_index(&self, id: SettingId) -> usize {
        match id {
            SettingId::TabWidth => Self::TAB_WIDTHS
                .iter()
                .position(|width| *width == self.tab_width)
                .unwrap_or(1),
            SettingId::IndentWithSpaces => usize::from(!self.indent_with_spaces),
            SettingId::LineEnding => self.line_ending.index(),
            _ => 0,
        }
    }

    pub fn choice_token(&self, id: SettingId) -> String {
        match id {
            SettingId::TabWidth => self.tab_width.to_string(),
            SettingId::IndentWithSpaces => on_off(self.indent_with_spaces),
            SettingId::LineEnding => self.line_ending.as_str().to_string(),
            _ => String::new(),
        }
    }

    pub fn choice_label(&self, id: SettingId) -> String {
        Self::choice_names(id)
            .get(self.choice_index(id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_choice(&mut self, id: SettingId, index: usize) {
        match id {
            SettingId::TabWidth => {
                if let Some(width) = Self::TAB_WIDTHS.get(index) {
                    self.tab_width = *width;
                }
            }
            SettingId::IndentWithSpaces => self.indent_with_spaces = index == 0,
            SettingId::LineEnding => self.line_ending = LineEndingPref::from_index(index),
            _ => {}
        }
    }

    pub fn restore_choice(&mut self, id: SettingId, token: &str) {
        match id {
            SettingId::TabWidth => {
                self.tab_width = token
                    .parse()
                    .ok()
                    .filter(|width| matches!(width, 2 | 4 | 8))
                    .unwrap_or(4);
            }
            SettingId::IndentWithSpaces => self.indent_with_spaces = token != "off",
            SettingId::LineEnding => {
                self.line_ending = match token {
                    "lf" => LineEndingPref::Lf,
                    "crlf" => LineEndingPref::Crlf,
                    _ => LineEndingPref::Auto,
                };
            }
            _ => {}
        }
    }

    pub fn reset(&mut self, id: SettingId) {
        let defaults = Self::default();
        match id {
            SettingId::TabWidth => self.tab_width = defaults.tab_width,
            SettingId::IndentWithSpaces => self.indent_with_spaces = defaults.indent_with_spaces,
            SettingId::LineEnding => self.line_ending = defaults.line_ending,
            _ => {}
        }
    }
}

/// Changes applied when a file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilePrefs {
    #[serde(default)]
    pub trim_trailing_space: bool,
    #[serde(default)]
    pub final_newline: bool,
}

impl FilePrefs {
    pub fn choice_names(id: SettingId) -> Vec<String> {
        match id {
            SettingId::TrimTrailingSpace | SettingId::FinalNewline => {
                vec!["On".into(), "Off".into()]
            }
            _ => Vec::new(),
        }
    }

    pub fn choice_index(&self, id: SettingId) -> usize {
        let on = match id {
            SettingId::TrimTrailingSpace => self.trim_trailing_space,
            SettingId::FinalNewline => self.final_newline,
            _ => return 0,
        };
        usize::from(!on)
    }

    pub fn choice_token(&self, id: SettingId) -> String {
        match id {
            SettingId::TrimTrailingSpace => on_off(self.trim_trailing_space),
            SettingId::FinalNewline => on_off(self.final_newline),
            _ => String::new(),
        }
    }

    pub fn choice_label(&self, id: SettingId) -> String {
        Self::choice_names(id)
            .get(self.choice_index(id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_choice(&mut self, id: SettingId, index: usize) {
        let on = index == 0;
        match id {
            SettingId::TrimTrailingSpace => self.trim_trailing_space = on,
            SettingId::FinalNewline => self.final_newline = on,
            _ => {}
        }
    }

    pub fn restore_choice(&mut self, id: SettingId, token: &str) {
        let on = token != "off";
        match id {
            SettingId::TrimTrailingSpace => self.trim_trailing_space = on,
            SettingId::FinalNewline => self.final_newline = on,
            _ => {}
        }
    }

    pub fn reset(&mut self, id: SettingId) {
        match id {
            SettingId::TrimTrailingSpace => self.trim_trailing_space = false,
            SettingId::FinalNewline => self.final_newline = false,
            _ => {}
        }
    }
}

/// One `settings.json`. Absent fields keep their defaults, so an older file still loads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsFile {
    #[serde(default = "default_theme_id")]
    pub color_theme: String,
    #[serde(default = "default_theme_id")]
    pub syntax_theme: String,
    #[serde(default)]
    pub editor: EditorPrefs,
    #[serde(default)]
    pub files: FilePrefs,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keybindings: BTreeMap<String, String>,
    #[serde(default)]
    pub lsp: LspPrefs,
    #[serde(default)]
    pub ai: AiPrefs,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            color_theme: default_theme_id(),
            syntax_theme: default_theme_id(),
            editor: EditorPrefs::default(),
            files: FilePrefs::default(),
            keybindings: BTreeMap::new(),
            lsp: LspPrefs::default(),
            ai: AiPrefs::default(),
        }
    }
}

impl SettingsFile {
    pub fn read(path: &Path) -> Result<Option<Self>, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|err| format!("Cannot load {}: {err}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(format!("Cannot read {}: {err}", path.display())),
        }
    }

    pub fn to_pretty(&self) -> String {
        serde_json::to_string_pretty(self)
            .map(|text| format!("{text}\n"))
            .unwrap_or_default()
    }

    /// Drops a tab width or a chord the editor will not use, and says why.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        if !matches!(self.editor.tab_width, 2 | 4 | 8) {
            warnings.push(format!(
                "Tab width {} is not 2, 4, or 8",
                self.editor.tab_width
            ));
            self.editor.tab_width = 4;
        }
        let mut kept = BTreeMap::new();
        for (id, token) in std::mem::take(&mut self.keybindings) {
            if keys::bindings().iter().all(|binding| binding.id != id) {
                warnings.push(format!("Unknown keybinding \"{id}\""));
                continue;
            }
            let Some(chord) = keys::Chord::parse(&token) else {
                warnings.push(format!("Cannot read keybinding \"{id}\""));
                continue;
            };
            if chord.is_locked() {
                warnings.push(format!("Ignored keybinding \"{id}\" ({})", chord.display()));
                continue;
            }
            kept.insert(id, chord.token());
        }
        self.keybindings = kept;
        warnings
    }

    pub fn document(&self, dir: &Path) -> (PathBuf, String) {
        (dir.join("settings.json"), self.to_pretty())
    }
}

pub fn assign_binding(
    overrides: &mut BTreeMap<String, String>,
    id: &str,
    token: &str,
) -> Result<String, String> {
    keys::assign(overrides, id, token).map_err(|err| match err {
        AssignError::Locked => "That key stays with the editor".to_string(),
        AssignError::Reserved => keys::Chord::parse(token)
            .and_then(|chord| chord.reserved_reason())
            .unwrap_or("That key stays with the editor")
            .to_string(),
        AssignError::Conflict(label) => {
            let shown = keys::Chord::parse(token)
                .map(|chord| chord.display())
                .unwrap_or_else(|| token.to_string());
            format!("{shown} is {label}")
        }
        AssignError::Blank => "That key is not a chord".to_string(),
    })
}

/// The open theme list for one Appearance row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceMenu {
    pub index: usize,
    /// Theme id to restore when the list is dismissed.
    origin: String,
}

impl ChoiceMenu {
    pub fn new(index: usize, origin: String) -> Self {
        Self { index, origin }
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub section: usize,
    pub row: usize,
    /// `true` after the highlighted section has been opened.
    pub inside: bool,
    /// Theme list dropped down from the highlighted row. `None` while it is closed.
    pub menu: Option<ChoiceMenu>,
    /// The highlighted keybinding is waiting for the next key.
    pub capture: bool,
    /// The highlighted server or model row is being typed.
    pub text: Option<TextDraft>,
}

/// Text being typed into a settings row. `Enter` keeps it. `Esc` drops it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDraft {
    pub value: String,
}

impl TextDraft {
    pub fn new(value: String) -> Self {
        Self { value }
    }
}

pub struct Section {
    pub name: &'static str,
    pub items: Vec<Item>,
}

#[derive(Clone, Copy)]
pub struct Item {
    pub label: &'static str,
    pub detail: &'static str,
    pub id: SettingId,
}

fn fixed(name: &'static str, items: &'static [Item]) -> Section {
    Section {
        name,
        items: items.to_vec(),
    }
}

pub fn sections() -> &'static [Section] {
    static SECTIONS: OnceLock<Vec<Section>> = OnceLock::new();
    SECTIONS
        .get_or_init(|| {
            let keybindings = keys::bindings()
                .iter()
                .map(|binding| Item {
                    label: binding.label,
                    detail: binding.detail,
                    id: SettingId::Binding(binding.id),
                })
                .collect();
            vec![
                fixed(
                    "Editor",
                    &[
                        Item {
                            label: "Tab width",
                            detail: "Spaces inserted for a Tab.",
                            id: SettingId::TabWidth,
                        },
                        Item {
                            label: "Indent with spaces",
                            detail: "Use spaces instead of a tab character.",
                            id: SettingId::IndentWithSpaces,
                        },
                        Item {
                            label: "Line endings",
                            detail: "Ending written when a file is saved.",
                            id: SettingId::LineEnding,
                        },
                    ],
                ),
                fixed(
                    "Appearance",
                    &[
                        Item {
                            label: "Color theme",
                            detail: "Colors used for the interface.",
                            id: SettingId::ColorTheme,
                        },
                        Item {
                            label: "Syntax colors",
                            detail: "Colors used for code.",
                            id: SettingId::SyntaxTheme,
                        },
                    ],
                ),
                fixed(
                    "Files",
                    &[
                        Item {
                            label: "Trim trailing space",
                            detail: "Remove spaces at the end of a line on save.",
                            id: SettingId::TrimTrailingSpace,
                        },
                        Item {
                            label: "Final newline",
                            detail: "Add a newline at the end of a file on save.",
                            id: SettingId::FinalNewline,
                        },
                    ],
                ),
                Section {
                    name: "Keybindings",
                    items: keybindings,
                },
                fixed(
                    "Language",
                    &[
                        Item {
                            label: "Language server",
                            detail: "Diagnostics, completion, rename, and format.",
                            id: SettingId::LanguageServer,
                        },
                        Item {
                            label: "Check on save",
                            detail: "rust-analyzer runs cargo check when a Rust file is saved.",
                            id: SettingId::CheckOnSave,
                        },
                        Item {
                            label: "Format on save",
                            detail: "Ask the language server to format the file before writing it.",
                            id: SettingId::FormatOnSave,
                        },
                        Item {
                            label: "Python checking",
                            detail: "How strictly pyright checks an open Python file.",
                            id: SettingId::PythonChecking,
                        },
                    ],
                ),
                fixed(
                    "AI",
                    &[
                        Item {
                            label: "Assistant",
                            detail: "Let the assistant read files and edit them.",
                            id: SettingId::AiEnabled,
                        },
                        Item {
                            label: "Provider",
                            detail: "Picks the default server address.",
                            id: SettingId::AiProvider,
                        },
                        Item {
                            label: "Server",
                            detail: "OpenAI-compatible address, including /v1.",
                            id: SettingId::AiServer,
                        },
                        Item {
                            label: "Model",
                            detail: "Model name the server should run.",
                            id: SettingId::AiModel,
                        },
                        Item {
                            label: "API key",
                            detail: "Optional Bearer token. Leave empty for local Ollama or LM Studio.",
                            id: SettingId::AiApiKey,
                        },
                        Item {
                            label: "Streaming",
                            detail: "Show tokens as they arrive. Falls back if the server cannot stream.",
                            id: SettingId::AiStreaming,
                        },
                        Item {
                            label: "Auto-approve tools",
                            detail: "Run apply_edit, create_file, and save without asking Allow / Deny.",
                            id: SettingId::AiAutoApprove,
                        },
                        Item {
                            label: "Context size",
                            detail: "Token window for Ollama (num_ctx). Default 16384. Type auto or a number. LM Studio: set context there instead.",
                            id: SettingId::AiContextSize,
                        },
                    ],
                ),
            ]
        })
        .as_slice()
}

impl Page {
    pub fn new() -> Self {
        Self {
            section: 0,
            row: 0,
            inside: false,
            menu: None,
            capture: false,
            text: None,
        }
    }

    pub fn section(&self) -> &'static Section {
        let sections = sections();
        &sections[self.section.min(sections.len().saturating_sub(1))]
    }

    fn len(&self) -> usize {
        if self.inside {
            self.section().items.len()
        } else {
            sections().len()
        }
    }

    /// Moves the highlight in the menu that is open. The highlight wraps.
    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.len();
        if len == 0 {
            return;
        }
        let cursor = if self.inside { self.row } else { self.section } as isize;
        let next = (cursor + delta).rem_euclid(len as isize) as usize;
        if self.inside {
            self.row = next;
        } else {
            self.section = next;
        }
    }

    pub fn enter(&mut self) {
        if self.inside || sections().is_empty() {
            return;
        }
        self.inside = true;
        self.row = 0;
    }

    /// Leaves the open section. Returns `true` when the page itself should close.
    pub fn back(&mut self) -> bool {
        if self.inside {
            self.inside = false;
            false
        } else {
            true
        }
    }

    pub fn first(&mut self) {
        if self.inside {
            self.row = 0;
        } else {
            self.section = 0;
        }
    }

    pub fn last(&mut self) {
        let last = self.len().saturating_sub(1);
        if self.inside {
            self.row = last;
        } else {
            self.section = last;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_open_one_at_a_time_and_back_returns_to_the_list() {
        let mut page = Page::new();
        page.enter();
        assert!(page.inside);
        assert_eq!(page.row, 0);
        page.move_cursor(1);
        assert_eq!(page.row, 1);
        assert!(!page.back());
        assert!(!page.inside);
        assert_eq!(page.section, 0);
        page.move_cursor(-1);
        assert_eq!(page.section, sections().len() - 1);
        assert!(page.back());
    }

    #[test]
    fn an_older_settings_file_keeps_the_new_defaults() {
        let text = r#"{"color_theme":"tokyo-night","syntax_theme":"tokyo-night"}"#;
        let mut file: SettingsFile = serde_json::from_str(text).unwrap();
        assert!(file.sanitize().is_empty());
        assert_eq!(file.editor, EditorPrefs::default());
        assert_eq!(file.files, FilePrefs::default());
        assert_eq!(file.ai, AiPrefs::default());
        assert!(file.keybindings.is_empty());
        file.ai.server = "http://127.0.0.1:11434/v1".into();
        file.ai.model = "qwen2.5-coder".into();
        file.editor.tab_width = 2;
        file.keybindings
            .insert("save".into(), "ctrl+shift+s".into());
        let saved = file.to_pretty();
        let back: SettingsFile = serde_json::from_str(&saved).unwrap();
        assert_eq!(back.editor.tab_width, 2);
        assert_eq!(back.ai.model, "qwen2.5-coder");
        assert_eq!(back.ai.server, "http://127.0.0.1:11434/v1");
        assert_eq!(
            back.keybindings.get("save").map(String::as_str),
            Some("ctrl+shift+s")
        );
        assert!(!saved.contains("\"keybindings\": {}"));
    }
}
