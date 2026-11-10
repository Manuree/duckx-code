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
    /// Context window for the current provider and model only.
    AiModelContext,
    /// Longest answer for the current provider and model only.
    AiModelMaxOutput,
    /// Seconds one request may take.
    AiTimeout,
    /// Extra attempts after a 408, 429, or 5xx answer.
    AiRetries,
    /// OpenRouter `X-Title`.
    AiAppName,
    /// OpenRouter `HTTP-Referer`.
    AiReferer,
    /// Forget every remembered server and model.
    AiClearRecent,
    /// Fold older turns into a summary when the prompt nears the context size.
    AiAutoCompact,
    /// Tokens of the newest turns auto compact keeps word for word.
    AiCompactKeep,
    /// Percent of the context size kept free before auto compact starts.
    AiCompactBuffer,
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

/// How many values each recent list keeps.
pub const RECENT_LIMIT: usize = 10;

/// Servers and models typed or used before, newest first, at most [`RECENT_LIMIT`]
/// each. API keys are never kept here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiRecent {
    /// Base URLs per provider.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub servers: BTreeMap<AiProvider, Vec<String>>,
    /// Model names per provider and server, keyed by [`AiRecent::model_key`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, Vec<String>>,
}

impl AiRecent {
    pub fn is_empty(&self) -> bool {
        self.servers.values().all(Vec::is_empty) && self.models.values().all(Vec::is_empty)
    }

    /// Every remembered value, across all providers.
    pub fn len(&self) -> usize {
        self.servers.values().map(Vec::len).sum::<usize>()
            + self.models.values().map(Vec::len).sum::<usize>()
    }

    /// `ollama http://127.0.0.1:11434/v1`: one provider on one server.
    pub fn model_key(provider: AiProvider, server: &str) -> String {
        format!(
            "{} {}",
            provider.as_str(),
            server.trim().trim_end_matches('/')
        )
    }

    /// Moves `value` to the front. Returns true when the list changed.
    fn push(list: &mut Vec<String>, value: &str) -> bool {
        let value = value.trim();
        if value.is_empty() || list.first().is_some_and(|first| first == value) {
            return false;
        }
        list.retain(|item| item != value);
        list.insert(0, value.to_string());
        list.truncate(RECENT_LIMIT);
        true
    }

    fn remove(list: &mut Vec<String>, value: &str) -> bool {
        let before = list.len();
        list.retain(|item| item != value);
        list.len() != before
    }

    /// Drops empty lists and anything past [`RECENT_LIMIT`] from a hand-edited file.
    fn tidy(&mut self) {
        for list in self.servers.values_mut().chain(self.models.values_mut()) {
            let mut seen = std::collections::BTreeSet::new();
            list.retain(|item| !item.trim().is_empty() && seen.insert(item.clone()));
            list.truncate(RECENT_LIMIT);
        }
        self.servers.retain(|_, list| !list.is_empty());
        self.models.retain(|_, list| !list.is_empty());
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
/// Newest tokens auto compact keeps verbatim, unless the context is too small for that.
pub const DEFAULT_COMPACT_KEEP: u32 = 15_000;
pub const COMPACT_KEEP_RANGE: std::ops::RangeInclusive<u32> = 1_000..=200_000;
/// Percent of the context size left free for the answer before compacting.
pub const DEFAULT_COMPACT_BUFFER: u32 = 15;
pub const COMPACT_BUFFER_RANGE: std::ops::RangeInclusive<u32> = 5..=50;
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
    /// The last servers and models, for the list under the Server and Model rows.
    #[serde(default, skip_serializing_if = "AiRecent::is_empty")]
    pub recent: AiRecent,
    /// Summarize older turns when the prompt would pass the context size minus the buffer.
    #[serde(default = "enabled")]
    pub auto_compact: bool,
    /// Tokens of the newest turns kept word for word when compacting.
    #[serde(default = "default_compact_keep")]
    pub compact_keep: u32,
    /// Percent of the context size kept free before compacting.
    #[serde(default = "default_compact_buffer")]
    pub compact_buffer: u32,
    /// Limits for one model, keyed `"<provider> <model>"` (see [`AiPrefs::model_key`]).
    /// A model without an entry uses Context size and Max tokens.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_limits: BTreeMap<String, ModelLimits>,
}

fn default_compact_keep() -> u32 {
    DEFAULT_COMPACT_KEEP
}

fn default_compact_buffer() -> u32 {
    DEFAULT_COMPACT_BUFFER
}

/// One model's own limits. `None` falls back to the global row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelLimits {
    /// Context window in tokens: Ollama's `num_ctx`, and the history budget everywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u32>,
    /// Longest answer (`max_tokens`, or `num_predict` on Ollama).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u32>,
}

impl ModelLimits {
    pub fn is_empty(&self) -> bool {
        self.context.is_none() && self.max_output.is_none()
    }
}

/// Context a model may be given on its own row: wider than Context size, for
/// cloud models with very long windows.
pub const MODEL_CONTEXT_RANGE: std::ops::RangeInclusive<u32> = 1024..=2_000_000;

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
            recent: AiRecent::default(),
            auto_compact: true,
            compact_keep: DEFAULT_COMPACT_KEEP,
            compact_buffer: DEFAULT_COMPACT_BUFFER,
            model_limits: BTreeMap::new(),
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
                | SettingId::AiModelContext
                | SettingId::AiModelMaxOutput
                | SettingId::AiTimeout
                | SettingId::AiRetries
                | SettingId::AiAppName
                | SettingId::AiReferer
                | SettingId::AiCompactKeep
                | SettingId::AiCompactBuffer
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

    /// Recent servers for the selected provider, newest first.
    pub fn recent_servers(&self) -> &[String] {
        self.recent
            .servers
            .get(&self.provider)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Recent models for the selected provider on its current server, newest first.
    pub fn recent_models(&self) -> &[String] {
        self.recent
            .models
            .get(&AiRecent::model_key(self.provider, &self.endpoint()))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Remembers the current server (when one is typed) and model. True when a list changed.
    pub fn remember_current(&mut self) -> bool {
        let mut changed = false;
        if !self.server.trim().is_empty() {
            let server = self.endpoint();
            changed |= AiRecent::push(
                self.recent.servers.entry(self.provider).or_default(),
                &server,
            );
        }
        if !self.model.trim().is_empty() {
            let key = AiRecent::model_key(self.provider, &self.endpoint());
            let model = self.model.clone();
            changed |= AiRecent::push(self.recent.models.entry(key).or_default(), &model);
        }
        changed
    }

    /// Forgets one value from the Server or Model list. True when it was there.
    pub fn forget_recent(&mut self, id: SettingId, value: &str) -> bool {
        let removed = match id {
            SettingId::AiServer => self
                .recent
                .servers
                .get_mut(&self.provider)
                .is_some_and(|list| AiRecent::remove(list, value)),
            SettingId::AiModel => {
                let key = AiRecent::model_key(self.provider, &self.endpoint());
                self.recent
                    .models
                    .get_mut(&key)
                    .is_some_and(|list| AiRecent::remove(list, value))
            }
            _ => false,
        };
        self.recent.tidy();
        removed
    }

    /// Entries the Server or Model list shows even with nothing remembered: the
    /// saved value, and for Server the provider's default. Never the API key.
    pub fn seeds_for(&self, id: SettingId) -> Vec<(String, PickSource)> {
        match id {
            SettingId::AiServer => {
                let mut seeds = Vec::new();
                if !self.server.trim().is_empty() {
                    seeds.push((self.endpoint(), PickSource::Current));
                }
                seeds.push((
                    self.provider.default_server().to_string(),
                    PickSource::Default,
                ));
                seeds
            }
            SettingId::AiModel if !self.model.trim().is_empty() => {
                vec![(self.model.trim().to_string(), PickSource::Current)]
            }
            _ => Vec::new(),
        }
    }

    /// Recent values for a text row. Only Server and Model keep a list.
    pub fn recent_for(&self, id: SettingId) -> &[String] {
        match id {
            SettingId::AiServer => self.recent_servers(),
            SettingId::AiModel => self.recent_models(),
            _ => &[],
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

    /// `"<provider> <model>"`, the key of this model's limits. `None` without a model.
    pub fn model_key(&self) -> Option<String> {
        let model = self.model.trim();
        (!model.is_empty()).then(|| format!("{} {model}", self.provider.as_str()))
    }

    /// The current model's own limits (empty when it has none).
    pub fn model_limits(&self) -> ModelLimits {
        self.model_key()
            .and_then(|key| self.model_limits.get(&key).copied())
            .unwrap_or_default()
    }

    /// Changes the current model's limits; an entry with nothing set is removed.
    /// Returns whether anything changed. Without a model nothing is stored.
    pub fn set_model_limits(&mut self, limits: ModelLimits) -> bool {
        let Some(key) = self.model_key() else {
            return false;
        };
        let before = self.model_limits.get(&key).copied().unwrap_or_default();
        if limits.is_empty() {
            self.model_limits.remove(&key);
        } else {
            self.model_limits.insert(key, limits);
        }
        before != limits
    }

    /// `max_tokens` / `num_predict` for this model: its own Max output, else
    /// [`Self::global_max_tokens`]. `None` leaves it out.
    pub fn resolved_max_tokens(&self) -> Option<u32> {
        self.model_limits()
            .max_output
            .filter(|n| (1..=MAX_TOKENS_LIMIT).contains(n))
            .or_else(|| self.global_max_tokens())
    }

    /// The Max tokens row alone, for every model without its own Max output.
    pub fn global_max_tokens(&self) -> Option<u32> {
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

    pub fn resolved_compact_keep(&self) -> u32 {
        self.compact_keep
            .clamp(*COMPACT_KEEP_RANGE.start(), *COMPACT_KEEP_RANGE.end())
    }

    pub fn resolved_compact_buffer(&self) -> u32 {
        self.compact_buffer
            .clamp(*COMPACT_BUFFER_RANGE.start(), *COMPACT_BUFFER_RANGE.end())
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
            SettingId::AiThink => self.provider == AiProvider::Ollama,
            SettingId::AiAppName | SettingId::AiReferer => self.provider == AiProvider::OpenRouter,
            _ => true,
        }
    }

    pub fn choice_names(id: SettingId) -> Vec<String> {
        match id {
            SettingId::AiEnabled
            | SettingId::AiAutoApprove
            | SettingId::AiStreaming
            | SettingId::AiThink
            | SettingId::AiAutoCompact => vec!["On".into(), "Off".into()],
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
            SettingId::AiAutoCompact => usize::from(!self.auto_compact),
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
            SettingId::AiAutoCompact => on_off(self.auto_compact),
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
            SettingId::AiAutoCompact => self.auto_compact = index == 0,
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
            SettingId::AiAutoCompact => self.auto_compact = token != "off",
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
            SettingId::AiModelContext => {
                let limits = ModelLimits {
                    context: None,
                    ..self.model_limits()
                };
                self.set_model_limits(limits);
            }
            SettingId::AiModelMaxOutput => {
                let limits = ModelLimits {
                    max_output: None,
                    ..self.model_limits()
                };
                self.set_model_limits(limits);
            }
            SettingId::AiTimeout => self.request_timeout = DEFAULT_TIMEOUT_SECS,
            SettingId::AiRetries => self.retries = DEFAULT_RETRIES,
            SettingId::AiAppName => self.openrouter_app_name = default_app_name(),
            SettingId::AiReferer => self.openrouter_referer = default_referer(),
            SettingId::AiClearRecent => self.recent = AiRecent::default(),
            SettingId::AiAutoCompact => self.auto_compact = true,
            SettingId::AiCompactKeep => self.compact_keep = DEFAULT_COMPACT_KEEP,
            SettingId::AiCompactBuffer => self.compact_buffer = DEFAULT_COMPACT_BUFFER,
            _ => {}
        }
    }

    /// True when the context window is a number the user set, or Ollama, which is
    /// sent `num_ctx` and so really uses it. Otherwise the ctx counter shows `?`.
    pub fn context_size_known(&self) -> bool {
        let raw = self.context_size.trim();
        self.provider == AiProvider::Ollama
            || raw
                .parse::<u32>()
                .is_ok_and(|n| (1024..=262_144).contains(&n))
    }

    /// Context window for this model: its own Model context, else
    /// [`Self::global_context_size`].
    pub fn resolved_context_size(&self) -> u32 {
        self.model_limits()
            .context
            .filter(|n| MODEL_CONTEXT_RANGE.contains(n))
            .unwrap_or_else(|| self.global_context_size())
    }

    /// The Context size row alone, for every model without its own context.
    pub fn global_context_size(&self) -> u32 {
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
        self.ai.recent.tidy();
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
/// Edited with the same line editor as the command bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDraft {
    pub input: crate::line_edit::LineInput,
    /// Highlighted entry in the list under the row. `None` while typing.
    pub pick: Option<usize>,
    /// What was typed last. The list shows entries that contain it.
    pub filter: String,
}

impl TextDraft {
    /// Longest value a row accepts, in characters.
    pub const LIMIT: usize = 512;

    pub fn new(value: String) -> Self {
        Self {
            input: crate::line_edit::LineInput::new(value),
            pick: None,
            filter: String::new(),
        }
    }

    pub fn value(&self) -> &str {
        self.input.text()
    }
}

/// Where an entry in the list under the Server or Model row comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickSource {
    /// Typed or used before. `Delete` forgets it.
    Recent,
    /// Listed by the server's `/models`.
    Server,
    /// The value saved now, shown even before anything was remembered.
    Current,
    /// The provider's built-in server.
    Default,
}

impl PickSource {
    /// The tag drawn at the right of the entry.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Server => "server",
            Self::Current => "current",
            Self::Default => "default",
        }
    }
}

/// Recent values first, then `seeds` (the saved value and the default), then the
/// server's models, without repeats, keeping the ones that contain `filter` (any
/// case). An empty filter keeps everything.
pub fn pick_list(
    recent: &[String],
    seeds: &[(String, PickSource)],
    listed: &[String],
    filter: &str,
) -> Vec<(String, PickSource)> {
    let needle = filter.trim().to_lowercase();
    let keep = |value: &str| needle.is_empty() || value.to_lowercase().contains(&needle);
    let mut out: Vec<(String, PickSource)> = Vec::new();
    let candidates = recent
        .iter()
        .map(|value| (value, PickSource::Recent))
        .chain(seeds.iter().map(|(value, source)| (value, *source)))
        .chain(listed.iter().map(|value| (value, PickSource::Server)));
    for (value, source) in candidates {
        if value.trim().is_empty() || !keep(value) || out.iter().any(|(have, _)| have == value) {
            continue;
        }
        out.push((value.clone(), source));
    }
    out
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
                            detail: "Ollama, LM Studio, OpenAI-compatible, or OpenRouter. Each keeps its own server, model, and key.",
                            id: SettingId::AiProvider,
                        },
                        Item {
                            label: "Server",
                            detail: "Base URL for this provider, including /v1. Empty uses the default.",
                            id: SettingId::AiServer,
                        },
                        Item {
                            label: "Model",
                            detail: "Model name for this provider, e.g. qwen3:8b or anthropic/claude-sonnet-4.",
                            id: SettingId::AiModel,
                        },
                        Item {
                            label: "Model context",
                            detail: "Token window for this model only. Blank uses Context size. Ollama gets it as num_ctx.",
                            id: SettingId::AiModelContext,
                        },
                        Item {
                            label: "Model max output",
                            detail: "Longest answer for this model only. Blank uses Max tokens.",
                            id: SettingId::AiModelMaxOutput,
                        },
                        Item {
                            label: "API key",
                            detail: "Bearer token for this provider. OPENROUTER_API_KEY or OPENAI_API_KEY wins when set.",
                            id: SettingId::AiApiKey,
                        },
                        Item {
                            label: "Max tokens",
                            detail: "Longest answer. Blank: 16384 on Ollama / LM Studio, left out on cloud. off leaves it out.",
                            id: SettingId::AiMaxTokens,
                        },
                        Item {
                            label: "Request timeout",
                            detail: "Seconds one request may take, streaming included. 10 to 7200.",
                            id: SettingId::AiTimeout,
                        },
                        Item {
                            label: "Retries",
                            detail: "Extra tries after 408, 429, or 5xx, waiting longer each time. 0 to 10.",
                            id: SettingId::AiRetries,
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
                            label: "Thinking",
                            detail: "Ollama only: ask for think: true when the model supports it.",
                            id: SettingId::AiThink,
                        },
                        Item {
                            label: "Context size",
                            detail: "Token window. Sent to Ollama as num_ctx; for every provider the budget auto compact works to. auto = 16384.",
                            id: SettingId::AiContextSize,
                        },
                        Item {
                            label: "Auto compact",
                            detail: "Near the context size, summarize older turns with the same model and keep the newest ones.",
                            id: SettingId::AiAutoCompact,
                        },
                        Item {
                            label: "Keep tokens",
                            detail: "Newest tokens kept word for word when compacting (less on a small context). 1000 to 200000.",
                            id: SettingId::AiCompactKeep,
                        },
                        Item {
                            label: "Compact buffer",
                            detail: "Percent of the context size left free for the answer before compacting. 5 to 50.",
                            id: SettingId::AiCompactBuffer,
                        },
                        Item {
                            label: "OpenRouter app name",
                            detail: "OpenRouter only: sent as X-Title so the app shows on openrouter.ai.",
                            id: SettingId::AiAppName,
                        },
                        Item {
                            label: "OpenRouter referer",
                            detail: "OpenRouter only: sent as HTTP-Referer. Empty sends none.",
                            id: SettingId::AiReferer,
                        },
                        Item {
                            label: "Clear recent history",
                            detail: "Forget the last 10 servers and models kept for every provider.",
                            id: SettingId::AiClearRecent,
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
    fn settings_without_compact_keys_load_with_auto_compact_on() {
        let text = r#"{"ai":{"provider":"ollama","model":"qwen3:8b","context_size":"8192"}}"#;
        let file: SettingsFile = serde_json::from_str(text).unwrap();
        assert!(file.ai.auto_compact);
        assert_eq!(file.ai.compact_keep, DEFAULT_COMPACT_KEEP);
        assert_eq!(file.ai.compact_buffer, DEFAULT_COMPACT_BUFFER);
        let mut ai = file.ai;
        ai.auto_compact = false;
        ai.compact_keep = 4_000;
        ai.compact_buffer = 25;
        let back: AiPrefs = serde_json::from_str(&serde_json::to_string(&ai).unwrap()).unwrap();
        assert_eq!(back, ai);
        ai.compact_keep = 5;
        ai.compact_buffer = 99;
        assert_eq!(ai.resolved_compact_keep(), *COMPACT_KEEP_RANGE.start());
        assert_eq!(ai.resolved_compact_buffer(), *COMPACT_BUFFER_RANGE.end());
        assert!(ai.applies(SettingId::AiContextSize));
        ai.switch_provider(AiProvider::OpenRouter);
        assert!(
            ai.applies(SettingId::AiContextSize),
            "the compact budget everywhere"
        );
        assert!(ai.applies(SettingId::AiAutoCompact));
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
    fn model_limits_round_trip_and_stay_out_of_an_older_file() {
        let text = r#"{"ai":{"provider":"ollama","model":"qwen3:8b","max_tokens":"1000"}}"#;
        let file: SettingsFile = serde_json::from_str(text).unwrap();
        assert!(file.ai.model_limits.is_empty());
        assert_eq!(file.ai.resolved_max_tokens(), Some(1000));
        let saved = serde_json::to_string(&file.ai).unwrap();
        assert!(!saved.contains("model_limits"), "{saved}");

        let mut ai = file.ai;
        assert!(ai.set_model_limits(ModelLimits {
            context: Some(131_072),
            max_output: None,
        }));
        assert!(!ai.set_model_limits(ai.model_limits()), "no change");
        let saved = serde_json::to_string(&ai).unwrap();
        assert!(
            saved.contains(r#""model_limits":{"ollama qwen3:8b":{"context":131072}}"#),
            "{saved}"
        );
        let back: AiPrefs = serde_json::from_str(&saved).unwrap();
        assert_eq!(back, ai);
        assert_eq!(back.resolved_context_size(), 131_072);
        assert_eq!(back.global_context_size(), 16384);
        assert_eq!(
            back.resolved_max_tokens(),
            Some(1000),
            "Max tokens still applies"
        );

        // A hand-edited value out of range falls back to the global row.
        ai.model_limits.insert(
            "ollama qwen3:8b".into(),
            ModelLimits {
                context: Some(10),
                max_output: Some(0),
            },
        );
        assert_eq!(ai.resolved_context_size(), 16384);
        assert_eq!(ai.resolved_max_tokens(), Some(1000));
        ai.reset(SettingId::AiModelContext);
        ai.reset(SettingId::AiModelMaxOutput);
        assert!(ai.model_limits.is_empty());
        ai.model.clear();
        assert!(!ai.set_model_limits(ModelLimits {
            context: Some(4096),
            max_output: None,
        }));
        assert!(ai.model_limits.is_empty());
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

    #[test]
    fn the_saved_value_and_default_seed_the_lists_but_never_the_key() {
        let mut ai = AiPrefs::default();
        ai.switch_provider(AiProvider::LmStudio);
        assert_eq!(
            ai.seeds_for(SettingId::AiServer),
            [("http://127.0.0.1:1234/v1".to_string(), PickSource::Default)]
        );
        assert!(ai.seeds_for(SettingId::AiModel).is_empty());
        ai.server = "http://10.0.0.9:1234/v1".into();
        ai.model = " prism-ml/bonsai-27b ".into();
        ai.api_key = "sk-secret".into();
        assert_eq!(
            ai.seeds_for(SettingId::AiServer),
            [
                ("http://10.0.0.9:1234/v1".to_string(), PickSource::Current),
                ("http://127.0.0.1:1234/v1".to_string(), PickSource::Default),
            ]
        );
        assert_eq!(
            ai.seeds_for(SettingId::AiModel),
            [("prism-ml/bonsai-27b".to_string(), PickSource::Current)]
        );
        assert!(ai.seeds_for(SettingId::AiApiKey).is_empty());
        assert!(ai.seeds_for(SettingId::AiReferer).is_empty());
    }

    #[test]
    fn pick_list_puts_recent_then_seeds_then_server_without_repeats() {
        let recent = vec!["b".to_string(), "a".to_string()];
        let seeds = vec![
            ("a".to_string(), PickSource::Current),
            ("c".to_string(), PickSource::Default),
            (" ".to_string(), PickSource::Current),
        ];
        let listed = vec!["c".to_string(), "d".to_string(), "b".to_string()];
        assert_eq!(
            pick_list(&recent, &seeds, &listed, ""),
            [
                ("b".to_string(), PickSource::Recent),
                ("a".to_string(), PickSource::Recent),
                ("c".to_string(), PickSource::Default),
                ("d".to_string(), PickSource::Server),
            ]
        );
        assert_eq!(
            pick_list(&recent, &seeds, &listed, "D"),
            [("d".to_string(), PickSource::Server)]
        );
        assert!(pick_list(&[], &[], &[], "").is_empty());
        assert_eq!(PickSource::Default.tag(), "default");
        assert_eq!(PickSource::Current.tag(), "current");
    }

    #[test]
    fn recent_lists_keep_ten_newest_first_without_repeats() {
        let mut ai = AiPrefs::default();
        for n in 0..12 {
            ai.model = format!("model-{n}");
            assert!(ai.remember_current());
        }
        assert!(!ai.remember_current(), "same value again changes nothing");
        ai.model = "model-5".into();
        assert!(ai.remember_current());
        let models = ai.recent_models();
        assert_eq!(models.len(), RECENT_LIMIT);
        assert_eq!(models[0], "model-5");
        assert_eq!(models[1], "model-11");
        assert_eq!(models.iter().filter(|m| *m == "model-5").count(), 1);
        assert!(!models.contains(&"model-0".to_string()));
        assert!(
            ai.recent_servers().is_empty(),
            "the default server is not typed"
        );
    }

    #[test]
    fn recent_models_belong_to_one_provider_and_server() {
        let mut ai = AiPrefs::default();
        ai.model = "qwen3:8b".into();
        ai.remember_current();
        ai.switch_provider(AiProvider::OpenRouter);
        assert!(ai.recent_models().is_empty());
        ai.model = "anthropic/claude-sonnet-4".into();
        ai.server = "https://openrouter.ai/api/v1/".into();
        ai.remember_current();
        assert_eq!(ai.recent_servers(), ["https://openrouter.ai/api/v1/"]);
        assert_eq!(ai.recent_models(), ["anthropic/claude-sonnet-4"]);
        ai.switch_provider(AiProvider::Ollama);
        assert_eq!(ai.recent_models(), ["qwen3:8b"]);
        ai.server = "http://10.0.0.2:11434/v1".into();
        assert!(
            ai.recent_models().is_empty(),
            "another server has its own models"
        );
        assert!(ai.forget_recent(SettingId::AiServer, "nothing") == false);
        let text = serde_json::to_string(&ai).unwrap();
        assert!(!text.contains("sk-"), "keys are never recent values");
        let back: AiPrefs = serde_json::from_str(&text).unwrap();
        assert_eq!(back.recent, ai.recent);
    }

    #[test]
    fn a_hand_edited_recent_list_is_tidied_on_load() {
        let text = r#"{"ai":{"recent":{"models":{"ollama http://127.0.0.1:11434/v1":["a","a","","b","c","d","e","f","g","h","i","j","k","l"]}}}}"#;
        let mut file: SettingsFile = serde_json::from_str(text).unwrap();
        file.sanitize();
        assert_eq!(
            file.ai.recent_models(),
            ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
        );
        assert_eq!(file.ai.recent.len(), RECENT_LIMIT);
        file.ai.reset(SettingId::AiClearRecent);
        assert!(file.ai.recent.is_empty());
        assert!(!file.to_pretty().contains("\"recent\""));
    }
}
