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
    /// Ollama `think: true` when the model supports it.
    AiThink,
    /// `max_tokens` cap. Blank keeps the provider's default.
    AiMaxTokens,
    /// Seconds one request may take.
    AiTimeout,
    /// Extra attempts after a 408, 429, or 5xx answer.
    AiRetries,
    /// OpenRouter `X-Title`.
    AiAppName,
    /// OpenRouter `HTTP-Referer`.
    AiReferer,
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

/// Which server the assistant talks to. Ollama uses its native `/api/chat`; the
/// rest speak OpenAI-compatible `/v1/chat/completions`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub enum AiProvider {
    #[default]
    #[serde(rename = "ollama")]
    Ollama,
    /// Older files wrote `"openai"` for LM Studio, so that still loads here.
    #[serde(rename = "lmstudio", alias = "openai")]
    LmStudio,
    /// Any server that speaks `/v1/chat/completions` (OpenAI, vLLM, llama.cpp, Groq, ...).
    #[serde(rename = "openai-compatible")]
    OpenaiCompatible,
    #[serde(rename = "openrouter")]
    OpenRouter,
}

impl AiProvider {
    /// Picker order in Settings → AI.
    pub const ALL: [Self; 4] = [
        Self::Ollama,
        Self::LmStudio,
        Self::OpenaiCompatible,
        Self::OpenRouter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
            Self::OpenaiCompatible => "OpenAI-compatible",
            Self::OpenRouter => "OpenRouter",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lmstudio",
            Self::OpenaiCompatible => "openai-compatible",
            Self::OpenRouter => "openrouter",
        }
    }

    /// A name typed after `/provider`, or a token saved by an older build.
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "ollama" => Some(Self::Ollama),
            "lmstudio" | "lm-studio" | "lm_studio" => Some(Self::LmStudio),
            "openai-compatible" | "openai" | "compatible" | "custom" => {
                Some(Self::OpenaiCompatible)
            }
            "openrouter" | "open-router" => Some(Self::OpenRouter),
            _ => None,
        }
    }

    /// Base URL, including `/v1`, when the server row is empty.
    pub fn default_server(self) -> &'static str {
        match self {
            Self::Ollama => "http://127.0.0.1:11434/v1",
            Self::LmStudio => "http://127.0.0.1:1234/v1",
            Self::OpenaiCompatible => "https://api.openai.com/v1",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
        }
    }

    /// Environment variable that wins over the key saved in `settings.json`.
    pub fn key_env(self) -> Option<&'static str> {
        match self {
            Self::Ollama | Self::LmStudio => None,
            Self::OpenaiCompatible => Some("OPENAI_API_KEY"),
            Self::OpenRouter => Some("OPENROUTER_API_KEY"),
        }
    }

    /// A server on this machine. These keep the legacy 16384-token cap by default.
    pub fn is_local(self) -> bool {
        matches!(self, Self::Ollama | Self::LmStudio)
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|provider| *provider == self)
            .unwrap_or(0)
    }

    fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }
}

/// Server, model, and key kept for a provider while another one is selected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProfile {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
}

impl ProviderProfile {
    fn is_empty(&self) -> bool {
        self.server.is_empty() && self.model.is_empty() && self.api_key.is_empty()
    }
}

/// Where the key a request will send comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    None,
    Saved,
    /// The provider's environment variable, which wins over the saved key.
    Env(&'static str),
}

pub const DEFAULT_TIMEOUT_SECS: u32 = 1800;
pub const TIMEOUT_RANGE: std::ops::RangeInclusive<u32> = 10..=7200;
pub const DEFAULT_RETRIES: u32 = 3;
pub const MAX_RETRIES: u32 = 10;
/// Cap sent to Ollama and LM Studio when Max tokens is blank, as before this setting.
pub const LOCAL_MAX_TOKENS: u32 = 16384;
pub const MAX_TOKENS_LIMIT: u32 = 1_000_000;

/// Assistant choices stored in `settings.json` under `ai`.
///
/// `server`, `model`, and `api_key` belong to the selected `provider`. The other
/// providers keep theirs in `profiles` and get them back when they are selected.
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
    /// Optional Bearer token. Empty means no Authorization header, unless the
    /// provider's environment variable is set.
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
    /// Prefer Ollama `think: true` when the model supports it.
    #[serde(default = "enabled")]
    pub think: bool,
    /// Blank: 16384 for Ollama / LM Studio, left out for cloud providers.
    /// `off` leaves it out everywhere. A number caps every answer.
    #[serde(default)]
    pub max_tokens: String,
    /// Seconds one request may take, streaming included.
    #[serde(default = "default_timeout")]
    pub request_timeout: u32,
    /// Extra attempts after a 408, 429, or 5xx answer.
    #[serde(default = "default_retries")]
    pub retries: u32,
    /// OpenRouter `X-Title`: the app name shown on openrouter.ai. Empty sends none.
    #[serde(default = "default_app_name")]
    pub openrouter_app_name: String,
    /// OpenRouter `HTTP-Referer`: the app's site. Empty sends none.
    #[serde(default = "default_referer")]
    pub openrouter_referer: String,
    /// Server, model, and key of the providers that are not selected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<AiProvider, ProviderProfile>,
}

fn default_context_size() -> String {
    "16384".to_string()
}

fn default_timeout() -> u32 {
    DEFAULT_TIMEOUT_SECS
}

fn default_retries() -> u32 {
    DEFAULT_RETRIES
}

pub fn default_app_name() -> String {
    "duckx-code".to_string()
}

pub fn default_referer() -> String {
    "https://github.com/Manuree/duckx-code".to_string()
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
            think: true,
            max_tokens: String::new(),
            request_timeout: DEFAULT_TIMEOUT_SECS,
            retries: DEFAULT_RETRIES,
            openrouter_app_name: default_app_name(),
            openrouter_referer: default_referer(),
            profiles: BTreeMap::new(),
        }
    }
}

impl AiPrefs {
    pub fn is_text(id: SettingId) -> bool {
        matches!(
            id,
            SettingId::AiServer
                | SettingId::AiModel
                | SettingId::AiApiKey
                | SettingId::AiContextSize
                | SettingId::AiMaxTokens
                | SettingId::AiTimeout
                | SettingId::AiRetries
                | SettingId::AiAppName
                | SettingId::AiReferer
        )
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

    /// Selects `next` and swaps in the server, model, and key it had last time.
    /// The provider being left keeps its own in `profiles`.
    pub fn switch_provider(&mut self, next: AiProvider) {
        if next == self.provider {
            return;
        }
        let leaving = ProviderProfile {
            server: std::mem::take(&mut self.server),
            model: std::mem::take(&mut self.model),
            api_key: std::mem::take(&mut self.api_key),
        };
        if leaving.is_empty() {
            self.profiles.remove(&self.provider);
        } else {
            self.profiles.insert(self.provider, leaving);
        }
        let arriving = self.profiles.remove(&next).unwrap_or_default();
        self.server = arriving.server;
        self.model = arriving.model;
        self.api_key = arriving.api_key;
        self.provider = next;
    }

    /// The key a request sends and where it came from. A set environment variable wins.
    pub fn resolved_api_key(&self) -> (String, KeySource) {
        self.resolved_api_key_with(|name| std::env::var(name).ok())
    }

    pub fn resolved_api_key_with(
        &self,
        env: impl Fn(&str) -> Option<String>,
    ) -> (String, KeySource) {
        if let Some(name) = self.provider.key_env()
            && let Some(value) = env(name).filter(|value| !value.trim().is_empty())
        {
            return (value.trim().to_string(), KeySource::Env(name));
        }
        let saved = self.api_key.trim();
        if saved.is_empty() {
            (String::new(), KeySource::None)
        } else {
            (saved.to_string(), KeySource::Saved)
        }
    }

    /// `max_tokens` / `num_predict` for this provider. `None` leaves it out.
    pub fn resolved_max_tokens(&self) -> Option<u32> {
        let raw = self.max_tokens.trim();
        if raw.is_empty() {
            return self.provider.is_local().then_some(LOCAL_MAX_TOKENS);
        }
        if matches!(raw.to_ascii_lowercase().as_str(), "off" | "none" | "0") {
            return None;
        }
        match raw.parse::<u32>() {
            Ok(n) if (1..=MAX_TOKENS_LIMIT).contains(&n) => Some(n),
            _ => self.provider.is_local().then_some(LOCAL_MAX_TOKENS),
        }
    }

    pub fn resolved_timeout(&self) -> u32 {
        if TIMEOUT_RANGE.contains(&self.request_timeout) {
            self.request_timeout
        } else {
            DEFAULT_TIMEOUT_SECS
        }
    }

    pub fn resolved_retries(&self) -> u32 {
        self.retries.min(MAX_RETRIES)
    }

    /// OpenRouter's attribution headers. Other providers get none.
    pub fn extra_headers(&self) -> Vec<(String, String)> {
        if self.provider != AiProvider::OpenRouter {
            return Vec::new();
        }
        let mut headers = Vec::new();
        let referer = self.openrouter_referer.trim();
        if !referer.is_empty() {
            headers.push(("HTTP-Referer".to_string(), referer.to_string()));
        }
        let title = self.openrouter_app_name.trim();
        if !title.is_empty() {
            headers.push(("X-Title".to_string(), title.to_string()));
        }
        headers
    }

    /// Context size and Thinking only reach Ollama's native API.
    pub fn applies(&self, id: SettingId) -> bool {
        match id {
            SettingId::AiContextSize | SettingId::AiThink => self.provider == AiProvider::Ollama,
            SettingId::AiAppName | SettingId::AiReferer => self.provider == AiProvider::OpenRouter,
            _ => true,
        }
    }

    pub fn choice_names(id: SettingId) -> Vec<String> {
        match id {
            SettingId::AiEnabled
            | SettingId::AiAutoApprove
            | SettingId::AiStreaming
            | SettingId::AiThink => vec!["On".into(), "Off".into()],
            SettingId::AiProvider => AiProvider::ALL
                .iter()
                .map(|provider| provider.label().to_string())
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn choice_index(&self, id: SettingId) -> usize {
        match id {
            SettingId::AiEnabled => usize::from(!self.enabled),
            SettingId::AiAutoApprove => usize::from(!self.auto_approve),
            SettingId::AiStreaming => usize::from(!self.streaming),
            SettingId::AiThink => usize::from(!self.think),
            SettingId::AiProvider => self.provider.index(),
            _ => 0,
        }
    }

    pub fn choice_token(&self, id: SettingId) -> String {
        match id {
            SettingId::AiEnabled => on_off(self.enabled),
            SettingId::AiAutoApprove => on_off(self.auto_approve),
            SettingId::AiStreaming => on_off(self.streaming),
            SettingId::AiThink => on_off(self.think),
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
            SettingId::AiThink => self.think = index == 0,
            SettingId::AiProvider => self.switch_provider(AiProvider::from_index(index)),
            _ => {}
        }
    }

    pub fn restore_choice(&mut self, id: SettingId, token: &str) {
        match id {
            SettingId::AiEnabled => self.enabled = token != "off",
            SettingId::AiAutoApprove => self.auto_approve = token != "off",
            SettingId::AiStreaming => self.streaming = token != "off",
            SettingId::AiThink => self.think = token != "off",
            SettingId::AiProvider => {
                self.switch_provider(AiProvider::parse(token).unwrap_or_default());
            }
            _ => {}
        }
    }

    pub fn reset(&mut self, id: SettingId) {
        match id {
            SettingId::AiEnabled => self.enabled = true,
            SettingId::AiAutoApprove => self.auto_approve = false,
            SettingId::AiStreaming => self.streaming = true,
            SettingId::AiThink => self.think = true,
            SettingId::AiProvider => self.switch_provider(AiProvider::Ollama),
            SettingId::AiServer => self.server.clear(),
            SettingId::AiModel => self.model.clear(),
            SettingId::AiApiKey => self.api_key.clear(),
            SettingId::AiContextSize => self.context_size = default_context_size(),
            SettingId::AiMaxTokens => self.max_tokens.clear(),
            SettingId::AiTimeout => self.request_timeout = DEFAULT_TIMEOUT_SECS,
            SettingId::AiRetries => self.retries = DEFAULT_RETRIES,
            SettingId::AiAppName => self.openrouter_app_name = default_app_name(),
            SettingId::AiReferer => self.openrouter_referer = default_referer(),
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

    #[test]
    fn an_older_openai_provider_loads_as_lm_studio_and_keeps_its_fields() {
        let text = r#"{"ai":{"provider":"openai","server":"http://10.0.0.5:1234/v1","model":"qwen","api_key":"k","think":false}}"#;
        let file: SettingsFile = serde_json::from_str(text).unwrap();
        assert_eq!(file.ai.provider, AiProvider::LmStudio);
        assert_eq!(file.ai.server, "http://10.0.0.5:1234/v1");
        assert_eq!(file.ai.model, "qwen");
        assert_eq!(file.ai.api_key, "k");
        assert!(!file.ai.think);
        assert_eq!(file.ai.max_tokens, "");
        assert_eq!(file.ai.request_timeout, DEFAULT_TIMEOUT_SECS);
        assert_eq!(file.ai.retries, DEFAULT_RETRIES);
        assert_eq!(file.ai.openrouter_app_name, "duckx-code");
        assert!(file.ai.profiles.is_empty());
        let saved = file.to_pretty();
        assert!(saved.contains("\"provider\": \"lmstudio\""), "{saved}");
        assert!(!saved.contains("\"profiles\""), "{saved}");
        let back: SettingsFile = serde_json::from_str(&saved).unwrap();
        assert_eq!(back, file);
    }

    #[test]
    fn every_provider_round_trips_through_settings_json() {
        for provider in AiProvider::ALL {
            let mut ai = AiPrefs::default();
            ai.switch_provider(provider);
            let text = serde_json::to_string(&ai).unwrap();
            let back: AiPrefs = serde_json::from_str(&text).unwrap();
            assert_eq!(back.provider, provider, "{text}");
            assert_eq!(AiProvider::parse(provider.as_str()), Some(provider));
            assert_eq!(AiProvider::from_index(provider.index()), provider);
        }
        assert_eq!(AiPrefs::choice_names(SettingId::AiProvider).len(), 4);
    }

    #[test]
    fn switching_providers_keeps_each_ones_server_model_and_key() {
        let mut ai = AiPrefs::default();
        ai.model = "qwen2.5-coder".into();
        ai.switch_provider(AiProvider::OpenRouter);
        assert_eq!(ai.model, "");
        assert_eq!(ai.endpoint(), "https://openrouter.ai/api/v1");
        ai.model = "anthropic/claude-sonnet-4".into();
        ai.api_key = "sk-or".into();
        ai.switch_provider(AiProvider::Ollama);
        assert_eq!(ai.model, "qwen2.5-coder");
        assert_eq!(ai.api_key, "");
        assert_eq!(ai.endpoint(), "http://127.0.0.1:11434/v1");
        let text = serde_json::to_string(&ai).unwrap();
        let mut back: AiPrefs = serde_json::from_str(&text).unwrap();
        back.switch_provider(AiProvider::OpenRouter);
        assert_eq!(back.model, "anthropic/claude-sonnet-4");
        assert_eq!(back.api_key, "sk-or");
        assert!(!back.profiles.contains_key(&AiProvider::OpenRouter));
        assert!(back.profiles.contains_key(&AiProvider::Ollama));
    }

    #[test]
    fn the_provider_environment_variable_wins_over_the_saved_key() {
        let mut ai = AiPrefs::default();
        ai.switch_provider(AiProvider::OpenRouter);
        let env = |name: &str| (name == "OPENROUTER_API_KEY").then(|| " sk-env ".to_string());
        let none = |_: &str| None;
        assert_eq!(
            ai.resolved_api_key_with(none),
            (String::new(), KeySource::None)
        );
        ai.api_key = "sk-saved".into();
        assert_eq!(
            ai.resolved_api_key_with(none),
            ("sk-saved".into(), KeySource::Saved)
        );
        assert_eq!(
            ai.resolved_api_key_with(env),
            ("sk-env".into(), KeySource::Env("OPENROUTER_API_KEY"))
        );
        ai.switch_provider(AiProvider::OpenaiCompatible);
        assert_eq!(
            ai.resolved_api_key_with(env),
            (String::new(), KeySource::None)
        );
        ai.switch_provider(AiProvider::Ollama);
        let all = |_: &str| Some("x".to_string());
        assert_eq!(
            ai.resolved_api_key_with(all),
            (String::new(), KeySource::None)
        );
    }

    #[test]
    fn max_tokens_blank_keeps_the_local_cap_and_omits_it_for_cloud() {
        let mut ai = AiPrefs::default();
        assert_eq!(ai.resolved_max_tokens(), Some(LOCAL_MAX_TOKENS));
        ai.switch_provider(AiProvider::LmStudio);
        assert_eq!(ai.resolved_max_tokens(), Some(LOCAL_MAX_TOKENS));
        ai.switch_provider(AiProvider::OpenRouter);
        assert_eq!(ai.resolved_max_tokens(), None);
        ai.max_tokens = "4096".into();
        assert_eq!(ai.resolved_max_tokens(), Some(4096));
        ai.switch_provider(AiProvider::Ollama);
        ai.max_tokens = "off".into();
        assert_eq!(ai.resolved_max_tokens(), None);
        ai.max_tokens = "lots".into();
        assert_eq!(ai.resolved_max_tokens(), Some(LOCAL_MAX_TOKENS));
    }

    #[test]
    fn only_openrouter_gets_attribution_headers() {
        let mut ai = AiPrefs::default();
        for provider in [
            AiProvider::Ollama,
            AiProvider::LmStudio,
            AiProvider::OpenaiCompatible,
        ] {
            ai.switch_provider(provider);
            assert!(ai.extra_headers().is_empty(), "{provider:?}");
        }
        ai.switch_provider(AiProvider::OpenRouter);
        assert_eq!(
            ai.extra_headers(),
            vec![
                ("HTTP-Referer".to_string(), default_referer()),
                ("X-Title".to_string(), "duckx-code".to_string()),
            ]
        );
        ai.openrouter_referer.clear();
        assert_eq!(ai.extra_headers().len(), 1);
    }

    #[test]
    fn out_of_range_timeout_and_retries_fall_back() {
        let mut ai = AiPrefs::default();
        ai.request_timeout = 1;
        ai.retries = 99;
        assert_eq!(ai.resolved_timeout(), DEFAULT_TIMEOUT_SECS);
        assert_eq!(ai.resolved_retries(), MAX_RETRIES);
    }
}
