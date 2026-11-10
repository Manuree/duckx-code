//! OpenAI-compatible chat completions. Ollama, LM Studio, OpenRouter, and any
//! OpenAI-compatible server speak this; Ollama also has a native `/api/chat` path.
//! Streaming uses SSE (`data:` lines); a failed stream falls back to one JSON body.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use futures::StreamExt;
use serde_json::{Value, json};

use super::tool_specs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub content: String,
    /// Model reasoning kept out of the transcript and out of tool parsing.
    pub thinking: String,
    pub calls: Vec<ToolCall>,
    /// The assistant message, sent back before each tool result.
    pub message: Value,
    /// True when this completion retried after flattening poisoned tool history.
    pub history_flattened: bool,
    /// Tokens the server says the request and answer used (`usage`, or Ollama's
    /// `prompt_eval_count` + `eval_count`). `None` when it did not say.
    pub usage: Option<u32>,
}

/// Context tokens a response reports: OpenAI `usage.total_tokens` (or prompt +
/// completion), or Ollama `prompt_eval_count` + `eval_count`.
pub fn usage_of(value: &Value) -> Option<u32> {
    let number = |pointer: &str| value.pointer(pointer).and_then(Value::as_u64);
    let total = number("/usage/total_tokens").or_else(|| {
        let prompt = number("/usage/prompt_tokens")?;
        Some(prompt + number("/usage/completion_tokens").unwrap_or(0))
    });
    let total = total.or_else(|| {
        let prompt = number("/prompt_eval_count")?;
        Some(prompt + number("/eval_count").unwrap_or(0))
    })?;
    u32::try_from(total).ok().filter(|tokens| *tokens > 0)
}

/// `usage_of` for one SSE line, parsed only when it mentions usage.
fn sse_usage(line: &str) -> Option<u32> {
    if !line.contains("\"usage\"") {
        return None;
    }
    let data = line.trim().strip_prefix("data:")?.trim();
    usage_of(&serde_json::from_str(data).ok()?)
}

/// Streamed piece: visible answer text, or reasoning for the title only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaKind {
    Content,
    Thinking,
}

/// Request knobs from Settings → AI that do not change which endpoint is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tuning {
    /// `max_tokens` (OpenAI) or `options.num_predict` (Ollama native). `None` leaves it out.
    pub max_tokens: Option<u32>,
    /// Extra headers on every chat request, e.g. OpenRouter `HTTP-Referer` and `X-Title`.
    pub headers: Vec<(String, String)>,
    /// Time limit for one request. `None` keeps the client's own limit.
    pub timeout: Option<Duration>,
    /// Extra attempts after a 408, 429, or 5xx answer. A refused connection is not retried.
    pub retries: u32,
    /// Wait before the first retry. Doubles each time, capped at [`MAX_BACKOFF`].
    pub backoff: Duration,
}

/// The longest wait between two attempts, including a server's `Retry-After`.
pub const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// What a request used before these knobs existed: a 16384-token cap, no extra
/// headers, the client's timeout, and no retries.
pub static DEFAULT_TUNING: Tuning = Tuning {
    max_tokens: Some(16384),
    headers: Vec::new(),
    timeout: None,
    retries: 0,
    backoff: Duration::from_millis(1000),
};

impl Default for Tuning {
    fn default() -> Self {
        DEFAULT_TUNING.clone()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CompleteOptions<'a> {
    pub api_key: &'a str,
    pub stream: bool,
    /// Ollama `options.num_ctx`. Only sent on the native `/api/chat` path.
    pub num_ctx: Option<u32>,
    /// Prefer Ollama native `/api/chat` (honors num_ctx). LM Studio stays on `/v1`.
    pub ollama_native: bool,
    /// When true (and native), try `think: true` with per-model fallback.
    pub think: bool,
    pub tuning: &'a Tuning,
}

impl Default for CompleteOptions<'static> {
    fn default() -> Self {
        Self {
            api_key: "",
            stream: false,
            num_ctx: None,
            ollama_native: false,
            think: true,
            tuning: &DEFAULT_TUNING,
        }
    }
}

fn http_err(url: &str, err: reqwest::Error) -> String {
    if err.is_timeout() {
        return format!(
            "Timed out talking to {url}. Local models (especially reasoning + large create_file) can take several minutes — raise the client timeout or wait; this is not a UTF-8 bug."
        );
    }
    if err.is_connect() {
        return format!(
            "Cannot reach {url}: {err}. On Windows prefer http://127.0.0.1:1234/v1 over localhost if IPv6 ::1 is unbound."
        );
    }
    format!("Cannot reach {url}: {err}")
}

pub fn completions_url(server: &str) -> String {
    format!("{}/chat/completions", server.trim().trim_end_matches('/'))
}

/// Strip a trailing `/v1` and append `/api/chat`.
pub fn ollama_chat_url(server: &str) -> String {
    let trimmed = server.trim().trim_end_matches('/');
    let base = trimmed
        .strip_suffix("/v1")
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    format!("{base}/api/chat")
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn request_body(model: &str, messages: &[Value], tools: &[Value], stream: bool) -> Value {
    request_body_with(model, messages, tools, stream, Some(16384))
}

/// [`request_body`] with the `max_tokens` cap chosen in Settings. `None` leaves it out.
pub fn request_body_with(
    model: &str,
    messages: &[Value],
    tools: &[Value],
    stream: bool,
    max_tokens: Option<u32>,
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages,
        "temperature": 0.2
    });
    if let Some(max_tokens) = max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
    }
    if stream {
        body["stream"] = Value::Bool(true);
    }
    body
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn ollama_request_body(
    model: &str,
    messages: &[Value],
    tools: &[Value],
    stream: bool,
    num_ctx: Option<u32>,
    think: bool,
) -> Value {
    ollama_request_body_with(model, messages, tools, stream, num_ctx, think, Some(16384))
}

/// [`ollama_request_body`] with the `num_predict` cap chosen in Settings. `None` leaves it out.
pub fn ollama_request_body_with(
    model: &str,
    messages: &[Value],
    tools: &[Value],
    stream: bool,
    num_ctx: Option<u32>,
    think: bool,
    num_predict: Option<u32>,
) -> Value {
    let mut options = json!({
        "temperature": 0.2
    });
    if let Some(num_predict) = num_predict {
        options["num_predict"] = json!(num_predict);
    }
    if let Some(n) = num_ctx {
        options["num_ctx"] = json!(n);
    }
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": stream,
        "options": options
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
    }
    if think {
        body["think"] = Value::Bool(true);
    }
    body
}

/// Native Ollama `/api/chat` expects `function.arguments` as a JSON object, not a string.
fn messages_for_ollama_native(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            let mut out = message.clone();
            let Some(calls) = out.get("tool_calls").and_then(|v| v.as_array()).cloned() else {
                return out;
            };
            let mut fixed = Vec::with_capacity(calls.len());
            for (index, call) in calls.iter().enumerate() {
                let id = call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call-{index}"));
                let name = call
                    .pointer("/function/name")
                    .and_then(|v| v.as_str())
                    .or_else(|| call.get("name").and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .to_string();
                let args = call
                    .pointer("/function/arguments")
                    .cloned()
                    .or_else(|| call.get("arguments").cloned())
                    .unwrap_or(json!({}));
                let args_obj = match args {
                    Value::String(s) => serde_json::from_str::<Value>(&normalize_tool_arguments(&s))
                        .unwrap_or_else(|_| json!({})),
                    Value::Object(_) => args,
                    other => other,
                };
                fixed.push(json!({
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": args_obj }
                }));
            }
            out["tool_calls"] = Value::Array(fixed);
            out
        })
        .collect()
}

/// Parse a finished Ollama `/api/chat` JSON body into a [`Reply`].
pub fn parse_ollama_reply(text: &str) -> Result<Reply, String> {
    let value: Value = serde_json::from_str(text).map_err(|err| format!("Bad response: {err}"))?;
    let raw = value
        .get("message")
        .cloned()
        .ok_or_else(|| "Response has no message".to_string())?;
    finish_message_reply(raw)
}

fn models_without_tools() -> &'static Mutex<HashSet<String>> {
    static CELL: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashSet::new()))
}

/// One model on one server. The same model name on another provider is a different
/// deployment, so what one rejected says nothing about the other.
fn capability_key(server: &str, model: &str) -> String {
    format!("{}\n{model}", server.trim().trim_end_matches('/'))
}

fn model_skips_tools(model: &str) -> bool {
    models_without_tools()
        .lock()
        .map(|set| set.contains(model))
        .unwrap_or(false)
}

fn remember_model_without_tools(model: &str) {
    if let Ok(mut set) = models_without_tools().lock() {
        set.insert(model.to_string());
    }
}

fn models_without_think() -> &'static Mutex<HashSet<String>> {
    static CELL: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashSet::new()))
}

fn model_skips_think(model: &str) -> bool {
    models_without_think()
        .lock()
        .map(|set| set.contains(model))
        .unwrap_or(false)
}

fn remember_model_without_think(model: &str) {
    if let Ok(mut set) = models_without_think().lock() {
        set.insert(model.to_string());
    }
}

/// Only a plain client error can say "this model cannot do that". 401 / 402 / 403
/// (key, credits, access), 408 / 429 (timeout, rate limit), and every 5xx are about
/// the account or the server, so they must never mark a model as lacking tools or think.
fn capability_status(status: reqwest::StatusCode) -> bool {
    status.is_client_error() && !matches!(status.as_u16(), 401 | 402 | 403 | 408 | 429)
}

fn think_not_supported(status: reqwest::StatusCode, body: &str) -> bool {
    if !capability_status(status) {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("think")
        && (lower.contains("unknown")
            || lower.contains("invalid")
            || lower.contains("not support")
            || lower.contains("unsupported")
            || lower.contains("does not support")
            || lower.contains("extra")
            || lower.contains("unexpected"))
}

fn tools_not_supported(status: reqwest::StatusCode, body: &str) -> bool {
    if !capability_status(status) {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("does not support tools")
        || lower.contains("does not support tool")
        || lower.contains("tool use is not supported")
        // OpenRouter: "No endpoints found that support tool use."
        || lower.contains("support tool use")
        || (lower.contains("tool")
            && (lower.contains("unsupported")
                || lower.contains("not support")
                || lower.contains("unknown field")
                || lower.contains("not enabled")))
}

/// When the server rejects `tools`, describe them in the system text so textual parsing still works.
pub fn messages_with_tool_prompt(messages: &[Value]) -> Vec<Value> {
    let catalog = tool_catalog_prompt();
    let mut out = Vec::with_capacity(messages.len() + 1);
    let mut injected = false;
    for message in messages {
        if !injected && message.get("role").and_then(|r| r.as_str()) == Some("system") {
            let mut copy = message.clone();
            let existing = copy.get("content").and_then(|c| c.as_str()).unwrap_or("");
            copy["content"] = Value::String(format!("{existing}

{catalog}"));
            out.push(copy);
            injected = true;
        } else {
            out.push(message.clone());
        }
    }
    if !injected {
        out.insert(0, json!({"role": "system", "content": catalog}));
    }
    out
}

fn tool_catalog_prompt() -> String {
    let mut lines = vec![
        "You can call tools by emitting one of these forms (no other prose around the call):".into(),
        r#"{"name":"<tool>","arguments":{...}}"#.into(),
        "or ```json / ```tool_code fences, or <tool_call>...</tool_call>, or Python-style name(arg=value).".into(),
        "Available tools:".into(),
    ];
    for tool in tool_specs() {
        let name = tool["function"]["name"].as_str().unwrap_or("?");
        let desc = tool["function"]["description"].as_str().unwrap_or("");
        lines.push(format!("- {name}: {desc}"));
    }
    lines.join("
")
}

fn apply_auth(request: reqwest::RequestBuilder, api_key: &str) -> reqwest::RequestBuilder {
    let key = api_key.trim();
    if key.is_empty() {
        request
    } else {
        request.header("Authorization", format!("Bearer {key}"))
    }
}

/// Bearer token plus the extra headers and the per-request timeout from [`Tuning`].
fn prepare(
    request: reqwest::RequestBuilder,
    api_key: &str,
    tuning: &Tuning,
) -> reqwest::RequestBuilder {
    let mut request = apply_auth(request, api_key);
    for (name, value) in &tuning.headers {
        if !name.trim().is_empty() && !value.trim().is_empty() {
            request = request.header(name.trim(), value.trim());
        }
    }
    if let Some(timeout) = tuning.timeout {
        request = request.timeout(timeout);
    }
    request
}

/// 408, 429, and 5xx are worth another try. Anything else is the final answer.
fn retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 429) || status.is_server_error()
}

/// `Retry-After` in whole seconds, capped at [`MAX_BACKOFF`]. An HTTP date is ignored.
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let value = response.headers().get(reqwest::header::RETRY_AFTER)?;
    let seconds: u64 = value.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(seconds).min(MAX_BACKOFF))
}

fn backoff_for(tuning: &Tuning, attempt: u32) -> Duration {
    let factor = 1u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX);
    tuning.backoff.saturating_mul(factor).min(MAX_BACKOFF)
}

/// Sends `build()` and retries a 408 / 429 / 5xx answer up to `tuning.retries` times,
/// waiting `Retry-After` when the server gives one and an exponential backoff otherwise.
/// The last answer is returned whatever its status, so callers keep their error text.
async fn send_with_retry<B>(
    build: B,
    url: &str,
    tuning: &Tuning,
) -> Result<reqwest::Response, String>
where
    B: Fn() -> reqwest::RequestBuilder,
{
    let mut attempt = 0u32;
    loop {
        let response = build().send().await.map_err(|err| http_err(url, err))?;
        let status = response.status();
        if !retryable_status(status) || attempt >= tuning.retries {
            return Ok(response);
        }
        let wait = retry_after(&response).unwrap_or_else(|| backoff_for(tuning, attempt));
        drop(response);
        tokio::time::sleep(wait).await;
        attempt += 1;
    }
}

/// `Server returned 401 Unauthorized: ...` with a hint for the answers a cloud
/// provider gives when the key, credits, or rate limit is the problem.
fn status_error(status: reqwest::StatusCode, text: &str) -> String {
    let snippet: String = error_text(text).chars().take(300).collect();
    let hint = match status.as_u16() {
        401 => " (check the API key in Settings → AI)",
        402 => " (the account is out of credits)",
        403 => " (this key cannot use that model)",
        429 => " (rate limited; raise Retries in Settings → AI or wait)",
        _ => "",
    };
    format!("Server returned {status}{hint}: {snippet}")
}

/// `{"error":{"message":"..."}}` → that message. Anything else stays as it is.
fn error_text(text: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return text.to_string();
    };
    value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| text.to_string())
}

/// `/api/tags` for Ollama, `/models` for every OpenAI-compatible server.
pub fn models_url(endpoint: &str, ollama: bool) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if ollama {
        let base = endpoint.strip_suffix("/v1").unwrap_or(endpoint);
        format!("{}/api/tags", base.trim_end_matches('/'))
    } else {
        format!("{endpoint}/models")
    }
}

/// Model ids from an Ollama `/api/tags` (`models[].name`) or an OpenAI `/models` (`data[].id`) body.
pub fn parse_model_names(text: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for (list, field) in [("models", "name"), ("data", "id")] {
        if let Some(arr) = value.get(list).and_then(|v| v.as_array()) {
            for m in arr {
                if let Some(name) = m.get(field).and_then(|v| v.as_str()) {
                    names.push(name.to_string());
                }
            }
        }
    }
    names
}

/// Model names from `url`, sent with the provider's key and headers. Blocking:
/// run it from a job, never on the UI thread.
pub fn list_models(
    url: &str,
    api_key: &str,
    headers: &[(String, String)],
) -> Result<Vec<String>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("Cannot start HTTP: {err}"))?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|err| format!("Cannot start HTTP: {err}"))?;
        let tuning = Tuning {
            headers: headers.to_vec(),
            ..Tuning::default()
        };
        let response = prepare(client.get(url), api_key, &tuning)
            .send()
            .await
            .map_err(|err| http_err(url, err))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| format!("Cannot read the response: {err}"))?;
        if !status.is_success() {
            return Err(status_error(status, &text));
        }
        Ok(parse_model_names(&text))
    })
}

pub fn parse_reply(text: &str) -> Result<Reply, String> {
    let value: Value = serde_json::from_str(text).map_err(|err| format!("Bad response: {err}"))?;
    let raw = value
        .pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| "Response has no message".to_string())?;
    finish_message_reply(raw)
}

fn finish_message_reply(raw: Value) -> Result<Reply, String> {
    let mut content = message_text(&raw["content"]);
    let mut thinking = reasoning_text(&raw);
    let (visible, from_tags) = split_think_tags(&content);
    content = visible;
    if !from_tags.is_empty() {
        if !thinking.is_empty() {
            thinking.push('\n');
        }
        thinking.push_str(&from_tags);
    }
    let mut calls = structured_calls(&raw);
    // Prefer visible content; some reasoning models put textual tool calls in
    // reasoning_content / <think> instead — recover those as a fallback.
    if calls.is_empty() {
        let (found, stripped) = extract_textual(&content);
        if !found.is_empty() {
            calls = found;
            content = stripped;
        }
    }
    if calls.is_empty() {
        let (found, stripped) = extract_textual(&thinking);
        if !found.is_empty() {
            calls = found;
            thinking = stripped;
        }
    }
    Ok(reply_from_parts(content, thinking, calls))
}

fn reply_from_parts(content: String, thinking: String, calls: Vec<ToolCall>) -> Reply {
    let content_value = if content.trim().is_empty() && !calls.is_empty() {
        Value::Null
    } else {
        Value::String(content.clone())
    };
    let mut message = json!({
        "role": "assistant",
        "content": content_value
    });
    if !calls.is_empty() {
        message["tool_calls"] = json_calls(&calls);
    }
    Reply {
        content,
        thinking,
        calls,
        message,
        history_flattened: false,
        usage: None,
    }
}

/// Split `<think>…</think>` (and a leading block closed by `</think>`) out of content.
pub fn split_think_tags(text: &str) -> (String, String) {
    let mut visible = String::new();
    let mut thinking = String::new();
    let mut rest = text;
    // DeepSeek-R1 style: thinking then </think> with no opener.
    if let Some(end) = rest.find("</think>") {
        let before = &rest[..end];
        if !before.contains("<think>") {
            let piece = before.trim();
            if !piece.is_empty() {
                thinking.push_str(piece);
            }
            rest = &rest[end + "</think>".len()..];
        }
    }
    while let Some(start) = rest.find("<think>") {
        visible.push_str(&rest[..start]);
        rest = &rest[start + "<think>".len()..];
        if let Some(end) = rest.find("</think>") {
            let piece = rest[..end].trim();
            if !piece.is_empty() {
                if !thinking.is_empty() {
                    thinking.push('\n');
                }
                thinking.push_str(piece);
            }
            rest = &rest[end + "</think>".len()..];
        } else {
            let piece = rest.trim();
            if !piece.is_empty() {
                if !thinking.is_empty() {
                    thinking.push('\n');
                }
                thinking.push_str(piece);
            }
            rest = "";
            break;
        }
    }
    visible.push_str(rest);
    (visible, thinking)
}

/// Streams content while stripping `<think>…</think>` into Thinking deltas.
#[derive(Debug, Default)]
struct ContentThinkSplit {
    buf: String,
    in_think: bool,
}

impl ContentThinkSplit {
    fn push(&mut self, piece: &str, mut emit: impl FnMut(DeltaKind, &str)) {
        if piece.is_empty() {
            return;
        }
        self.buf.push_str(piece);
        loop {
            if self.in_think {
                if let Some(end) = self.buf.find("</think>") {
                    let think = self.buf[..end].to_string();
                    self.buf.drain(..end + "</think>".len());
                    self.in_think = false;
                    if !think.is_empty() {
                        emit(DeltaKind::Thinking, &think);
                    }
                    continue;
                }
                let hold = longest_tag_prefix(&self.buf, "</think>");
                let emit_len = crate::utf8::floor_char_boundary(
                    &self.buf,
                    self.buf.len().saturating_sub(hold),
                );
                if emit_len == 0 {
                    break;
                }
                let think = self.buf[..emit_len].to_string();
                self.buf.drain(..emit_len);
                if !think.is_empty() {
                    emit(DeltaKind::Thinking, &think);
                }
                break;
            }
            if let Some(start) = self.buf.find("<think>") {
                let before = self.buf[..start].to_string();
                self.buf.drain(..start + "<think>".len());
                self.in_think = true;
                if !before.is_empty() {
                    emit(DeltaKind::Content, &before);
                }
                continue;
            }
            // DeepSeek-R1: thinking then </think> with no opener — hold until close or decide.
            if let Some(end) = self.buf.find("</think>") {
                let think = self.buf[..end].to_string();
                self.buf.drain(..end + "</think>".len());
                if !think.is_empty() {
                    emit(DeltaKind::Thinking, &think);
                }
                continue;
            }
            let hold = longest_tag_prefix(&self.buf, "<think>")
                .max(longest_tag_prefix(&self.buf, "</think>"));
            let emit_len = crate::utf8::floor_char_boundary(
                &self.buf,
                self.buf.len().saturating_sub(hold),
            );
            if emit_len == 0 {
                break;
            }
            // If the buffer still might be R1 thinking (no visible answer yet), keep holding
            // when it looks like an unfinished think block (starts mid-tag already held).
            let content = self.buf[..emit_len].to_string();
            self.buf.drain(..emit_len);
            if !content.is_empty() {
                emit(DeltaKind::Content, &content);
            }
            break;
        }
    }

    fn finish(&mut self, mut emit: impl FnMut(DeltaKind, &str)) {
        if self.buf.is_empty() && !self.in_think {
            return;
        }
        let rest = std::mem::take(&mut self.buf);
        if self.in_think {
            self.in_think = false;
            if !rest.is_empty() {
                emit(DeltaKind::Thinking, &rest);
            }
            return;
        }
        let (visible, thinking) = split_think_tags(&rest);
        if !thinking.is_empty() {
            emit(DeltaKind::Thinking, &thinking);
        }
        if !visible.is_empty() {
            emit(DeltaKind::Content, &visible);
        }
    }
}

fn longest_tag_prefix(text: &str, tag: &str) -> usize {
    let bytes = text.as_bytes();
    let tag_b = tag.as_bytes();
    let max = bytes.len().min(tag_b.len().saturating_sub(1));
    for len in (1..=max).rev() {
        if bytes[bytes.len() - len..] == tag_b[..len] {
            return len;
        }
    }
    0
}

fn message_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                let kind = part["type"].as_str().unwrap_or("text");
                if kind == "reasoning" || kind == "thinking" {
                    continue;
                }
                let Some(text) = part["text"].as_str().or_else(|| part["content"].as_str()) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
            out
        }
        _ => String::new(),
    }
}

fn reasoning_text(message: &Value) -> String {
    for key in ["reasoning", "reasoning_content", "thinking"] {
        if let Some(text) = message[key].as_str() {
            if !text.trim().is_empty() {
                return text.to_string();
            }
        }
    }
    let Value::Array(parts) = &message["content"] else {
        return String::new();
    };
    let mut out = String::new();
    for part in parts {
        let kind = part["type"].as_str().unwrap_or("");
        if kind != "reasoning" && kind != "thinking" {
            continue;
        }
        let Some(text) = part["text"].as_str().or_else(|| part["reasoning"].as_str()) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(text);
    }
    out
}

fn structured_calls(message: &Value) -> Vec<ToolCall> {
    let Some(list) = message["tool_calls"].as_array() else {
        return Vec::new();
    };
    let mut calls = Vec::new();
    for (index, call) in list.iter().enumerate() {
        let Some(call) = tool_call_from_value(call, index) else {
            continue;
        };
        calls.push(call);
    }
    calls
}

fn tool_call_from_value(value: &Value, index: usize) -> Option<ToolCall> {
    if let Some(list) = value.get("tool_calls").and_then(|list| list.as_array()) {
        return list
            .iter()
            .enumerate()
            .filter_map(|(offset, call)| tool_call_from_value(call, index + offset))
            .next();
    }
    let name = value["function"]["name"]
        .as_str()
        .or_else(|| value["name"].as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }
    let arguments = if let Some(args) = value
        .pointer("/function/arguments")
        .filter(|args| !args.is_null())
    {
        arguments_string(args)
    } else if let Some(args) = value.get("arguments").filter(|args| !args.is_null()) {
        arguments_string(args)
    } else if let Some(args) = value.get("parameters").filter(|args| !args.is_null()) {
        arguments_string(args)
    } else {
        let mut copy = value.clone();
        if let Some(obj) = copy.as_object_mut() {
            obj.remove("name");
            obj.remove("id");
            obj.remove("type");
            obj.remove("function");
        }
        copy.to_string()
    };
    let id = value["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("call-{index}"));
    Some(ToolCall {
        id,
        name,
        arguments,
    })
}

fn arguments_string(value: &Value) -> String {
    match value {
        Value::String(text) => normalize_tool_arguments(text),
        Value::Null => "{}".to_string(),
        other => normalize_tool_arguments(&other.to_string()),
    }
}

/// OpenAI / Ollama require `function.arguments` to be a JSON-encoded object string.
/// Empty or invalid input becomes `"{}"` so history never poisons the next turn.
pub fn normalize_tool_arguments(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(_)) => trimmed.to_string(),
        Ok(Value::Null) => "{}".to_string(),
        Ok(Value::Array(arr)) => {
            // Some models wrap args; keep a valid JSON string.
            Value::Array(arr).to_string()
        }
        Ok(other) => json!({ "value": other }).to_string(),
        // Truncated tool JSON (hit max_tokens / client abort mid-stream) used
        // to become "{}". Try to close the object so create_file keeps path/content.
        Err(_) => salvage_truncated_json_object(trimmed).unwrap_or_else(|| "{}".to_string()),
    }
}

/// Best-effort repair when a streamed tool-call `arguments` string was cut off.
fn salvage_truncated_json_object(raw: &str) -> Option<String> {
    if !raw.starts_with('{') {
        return None;
    }
    // Try common closers for a string value cut mid-flight.
    for suffix in ["\"}", "\"\n}", "}", "\"]}", "\"}}"] {
        let candidate = format!("{raw}{suffix}");
        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&candidate) {
            if !map.is_empty() {
                return Some(Value::Object(map).to_string());
            }
        }
    }
    // At least recover a path so the app can report a useful error instead of
    // "Path is empty".
    let path = extract_json_string_field(raw, "path")?;
    let mut obj = serde_json::Map::new();
    obj.insert("path".into(), Value::String(path));
    if let Some(content) = extract_json_string_field_unterminated(raw, "content")
        .or_else(|| extract_json_string_field_unterminated(raw, "text"))
    {
        obj.insert("content".into(), Value::String(content.clone()));
        obj.insert("text".into(), Value::String(content));
    }
    Some(Value::Object(obj).to_string())
}

fn extract_json_string_field(raw: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let at = raw.find(&needle)?;
    let after_key = &raw[at + needle.len()..];
    let colon = after_key.find(':')?;
    let mut rest = after_key[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    rest = &rest[1..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(match next {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '"' => '"',
                    '\\' => '\\',
                    other => other,
                });
            }
            continue;
        }
        if ch == '"' {
            return Some(out);
        }
        out.push(ch);
    }
    None
}

/// Like [`extract_json_string_field`] but keeps text when the closing quote never arrived.
fn extract_json_string_field_unterminated(raw: &str, key: &str) -> Option<String> {
    if let Some(s) = extract_json_string_field(raw, key) {
        return Some(s);
    }
    let needle = format!("\"{key}\"");
    let at = raw.find(&needle)?;
    let after_key = &raw[at + needle.len()..];
    let colon = after_key.find(':')?;
    let mut rest = after_key[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    rest = &rest[1..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(match next {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '"' => '"',
                    '\\' => '\\',
                    other => other,
                });
            }
            continue;
        }
        out.push(ch);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Build OpenAI-shaped tool_calls with `arguments` always a valid JSON string.
pub fn json_calls(calls: &[ToolCall]) -> Value {
    Value::Array(
        calls
            .iter()
            .enumerate()
            .map(|(index, call)| {
                let id = if call.id.trim().is_empty() {
                    format!("call-{index}")
                } else {
                    call.id.clone()
                };
                json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": call.name,
                        "arguments": normalize_tool_arguments(&call.arguments)
                    }
                })
            })
            .collect(),
    )
}

/// Assert / sanitize one assistant tool_calls message for the OpenAI wire format.
pub fn sanitize_tool_message(message: &Value) -> Value {
    let mut out = message.clone();
    let Some(calls) = out.get("tool_calls").and_then(|v| v.as_array()).cloned() else {
        return out;
    };
    let mut fixed = Vec::with_capacity(calls.len());
    for (index, call) in calls.iter().enumerate() {
        let id = call
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("call-{index}"));
        let name = call
            .pointer("/function/name")
            .and_then(|v| v.as_str())
            .or_else(|| call.get("name").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let args = call
            .pointer("/function/arguments")
            .cloned()
            .or_else(|| call.get("arguments").cloned())
            .unwrap_or(Value::String(String::new()));
        let args = match args {
            Value::String(s) => normalize_tool_arguments(&s),
            other => normalize_tool_arguments(&other.to_string()),
        };
        fixed.push(json!({
            "id": id,
            "type": "function",
            "function": { "name": name, "arguments": args }
        }));
    }
    out["tool_calls"] = Value::Array(fixed);
    out
}

/// Turn tool_calls / tool messages into plain chat so a poisoned history can recover.
pub fn flatten_tool_history(messages: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(messages.len());
    for message in messages {
        let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("");
        match role {
            "assistant" if message.get("tool_calls").is_some() => {
                let mut text = message_text(&message["content"]);
                if let Some(calls) = message["tool_calls"].as_array() {
                    for call in calls {
                        let name = call
                            .pointer("/function/name")
                            .and_then(|v| v.as_str())
                            .or_else(|| call.get("name").and_then(|v| v.as_str()))
                            .unwrap_or("tool");
                        let args = call
                            .pointer("/function/arguments")
                            .cloned()
                            .unwrap_or(Value::String("{}".into()));
                        let args = match args {
                            Value::String(s) => normalize_tool_arguments(&s),
                            other => other.to_string(),
                        };
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&format!("Called {name}({args})"));
                    }
                }
                out.push(json!({"role": "assistant", "content": text}));
            }
            "tool" => {
                let content = message
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                out.push(json!({
                    "role": "user",
                    "content": format!("Tool result:\n{content}")
                }));
            }
            _ => out.push(sanitize_tool_message(message)),
        }
    }
    out
}

fn invalid_tool_call_arguments(status: reqwest::StatusCode, body: &str) -> bool {
    status.as_u16() == 400
        && body
            .to_ascii_lowercase()
            .contains("invalid tool call arguments")
}

/// Tool calls some local models write as text. The string is the reply with those calls removed.
/// Model-agnostic across Ollama / LM Studio families: JSON in content, fences (`json` /
/// `tool_code`), Hermes/Qwen `<tool_call>` / `<function=` tags, Llama `<|python_tag|>`,
/// Mistral `[TOOL_CALLS]`, and Python-style `name(arg=...)`. Unknown shapes stay as reply text.
fn extract_textual(text: &str) -> (Vec<ToolCall>, String) {
    let text = strip_special_markers(text);
    let (mut calls, after_tags) = extract_tag_calls(&text);
    let (more, after_json) = extract_json_calls(&after_tags);
    calls.extend(more);
    let (py, stripped) = extract_python_calls(&after_json);
    calls.extend(py);
    (calls, stripped)
}

fn strip_special_markers(text: &str) -> String {
    text.replace("<|python_tag|>", "")
        .replace("<|tool_call|>", "")
        .replace("<|tool_calls_section_begin|>", "")
        .replace("<|tool_calls_section_end|>", "")
        .replace("<|tool_call_begin|>", "")
        .replace("<|tool_call_end|>", "")
        .replace("[TOOL_CALLS]", "")
}

fn extract_tag_calls(text: &str) -> (Vec<ToolCall>, String) {
    let mut calls = Vec::new();
    let mut stripped = String::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let rest = &text[cursor..];
        let tool_at = rest.find("<tool_call>").map(|at| cursor + at);
        let fn_at = rest.find("<function=").map(|at| cursor + at);
        let Some(at) = (match (tool_at, fn_at) {
            (Some(tool_at), Some(fn_at)) => Some(tool_at.min(fn_at)),
            (Some(tool_at), None) => Some(tool_at),
            (None, Some(fn_at)) => Some(fn_at),
            (None, None) => None,
        }) else {
            stripped.push_str(&text[cursor..]);
            break;
        };
        stripped.push_str(&text[cursor..at]);
        if text[at..].starts_with("<tool_call>") {
            let inner_start = at + "<tool_call>".len();
            let Some(rel) = text[inner_start..].find("</tool_call>") else {
                stripped.push_str(&text[at..]);
                break;
            };
            let inner = &text[inner_start..inner_start + rel];
            let before = calls.len();
            push_block_calls(inner, &mut calls);
            let end = inner_start + rel + "</tool_call>".len();
            if calls.len() == before {
                stripped.push_str(&text[at..end]);
            }
            cursor = end;
        } else if let Some(end) = push_function(text, at, &mut calls) {
            cursor = end;
        } else {
            let ch = text[at..].chars().next().unwrap_or('<');
            stripped.push(ch);
            cursor = at + ch.len_utf8();
        }
    }
    (calls, stripped)
}

fn extract_json_calls(text: &str) -> (Vec<ToolCall>, String) {
    let mut calls = Vec::new();
    let mut stripped = String::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let rest = &text[cursor..];
        let fence_at = rest.find("```").map(|at| cursor + at);
        let brace_at = rest.find('{').map(|at| cursor + at);
        let bracket_at = rest.find('[').map(|at| cursor + at);
        let Some(at) = [fence_at, brace_at, bracket_at]
            .into_iter()
            .flatten()
            .min()
        else {
            stripped.push_str(&text[cursor..]);
            break;
        };
        stripped.push_str(&text[cursor..at]);
        if text[at..].starts_with("```") {
            if let Some(end) = take_fence_tool_calls(&text[at..], &mut calls) {
                cursor = at + end;
                continue;
            }
            stripped.push_str("```");
            cursor = at + 3;
            continue;
        }
        if let Some((value, len)) = first_json_span(&text[at..]) {
            if value_looks_like_tool_call(&value) {
                let before = calls.len();
                push_value_calls(&value, &mut calls);
                if calls.len() > before {
                    cursor = at + len;
                    while cursor < text.len()
                        && matches!(text.as_bytes()[cursor], b' ' | b'\t' | b'\n' | b'\r')
                    {
                        cursor += 1;
                    }
                    continue;
                }
            }
        }
        let ch = text[at..].chars().next().unwrap_or('{');
        stripped.push(ch);
        cursor = at + ch.len_utf8();
    }
    (calls, stripped)
}

/// ` ```lang?\n...\n``` ` — JSON tool calls or Python-style `name(...)` (Gemma `tool_code`).
fn take_fence_tool_calls(text: &str, calls: &mut Vec<ToolCall>) -> Option<usize> {
    if !text.starts_with("```") {
        return None;
    }
    let after_ticks = &text[3..];
    let nl = after_ticks.find('\n')?;
    let lang = after_ticks[..nl].trim().to_ascii_lowercase();
    let body_start = 3 + nl + 1;
    let close_rel = text[body_start..].find("```")?;
    let body = text[body_start..body_start + close_rel].trim();
    let before = calls.len();
    if let Some(value) = first_json(body).filter(value_looks_like_tool_call) {
        push_value_calls(&value, calls);
    }
    if calls.len() == before
        && matches!(lang.as_str(), "tool_code" | "python" | "py" | "json" | "")
    {
        let (py, _) = extract_python_calls(body);
        calls.extend(py);
    }
    if calls.len() == before {
        return None;
    }
    Some(body_start + close_rel + 3)
}

/// Python-style tool lines: `editor_state()` / `read_file(path="a.rs", start_line=1)`.
fn extract_python_calls(text: &str) -> (Vec<ToolCall>, String) {
    let mut calls = Vec::new();
    let mut stripped = String::new();
    let mut any = false;
    for (line_idx, line) in text.split_inclusive('\n').enumerate() {
        let trimmed = line.trim();
        let core = trimmed.strip_suffix(';').unwrap_or(trimmed).trim();
        if let Some(call) = parse_python_style_call(core, calls.len() + line_idx) {
            calls.push(call);
            any = true;
        } else {
            stripped.push_str(line);
        }
    }
    if !any {
        return (calls, text.to_string());
    }
    (calls, stripped)
}

fn parse_python_style_call(line: &str, index: usize) -> Option<ToolCall> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let open = line.find('(')?;
    let name = line[..open].trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        || !name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    {
        return None;
    }
    if !KNOWN_TOOL_NAMES.contains(&name) && !looks_like_tool_alias(name) {
        return None;
    }
    if !line.ends_with(')') {
        return None;
    }
    let inner = line[open + 1..line.len() - 1].trim();
    let args = parse_python_args(inner)?;
    Some(ToolCall {
        id: format!("call-{index}"),
        name: name.to_string(),
        arguments: Value::Object(args).to_string(),
    })
}

fn looks_like_tool_alias(name: &str) -> bool {
    matches!(
        name,
        "write"
            | "write_file"
            | "save_file"
            | "create"
            | "read"
            | "read_text"
            | "read_text_file"
            | "list"
            | "list_files"
            | "ls"
            | "dir"
            | "edit"
            | "edit_file"
            | "search_replace"
            | "str_replace"
            | "replace_in_file"
            | "open"
            | "goto_line"
            | "bash"
            | "shell"
            | "run_command"
    )
}

const KNOWN_TOOL_NAMES: &[&str] = &[
    "read_file",
    "list_dir",
    "editor_state",
    "diagnostics",
    "apply_edit",
    "create_file",
    "write_file",
    "search_replace",
    "list_files",
    "open_file",
    "save",
    "goto",
];

fn parse_python_args(inner: &str) -> Option<serde_json::Map<String, Value>> {
    let mut map = serde_json::Map::new();
    if inner.is_empty() {
        return Some(map);
    }
    for part in split_python_args(inner) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let eq = part.find('=')?;
        let key = part[..eq].trim();
        if key.is_empty() {
            return None;
        }
        let raw = part[eq + 1..].trim();
        map.insert(key.to_string(), python_value(raw)?);
    }
    Some(map)
}

fn split_python_args(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut in_string: Option<char> = None;
    let mut escape = false;
    for (idx, ch) in inner.char_indices() {
        if let Some(quote) = in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == quote {
                in_string = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => in_string = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(inner[start..idx].to_string());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(inner[start..].to_string());
    parts
}

fn python_value(raw: &str) -> Option<Value> {
    if (raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2)
        || (raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2)
    {
        // Quotes are ASCII; still floor so a corrupted input cannot panic.
        let end = crate::utf8::floor_char_boundary(raw, raw.len() - 1);
        let inner = &raw[1..end];
        return Some(Value::String(inner.to_string()));
    }
    if raw == "True" || raw == "true" {
        return Some(Value::Bool(true));
    }
    if raw == "False" || raw == "false" {
        return Some(Value::Bool(false));
    }
    if raw == "None" || raw == "null" {
        return Some(Value::Null);
    }
    if let Ok(n) = raw.parse::<i64>() {
        return Some(json!(n));
    }
    if let Ok(n) = raw.parse::<f64>() {
        return Some(json!(n));
    }
    if (raw.starts_with('{') && raw.ends_with('}')) || (raw.starts_with('[') && raw.ends_with(']'))
    {
        return serde_json::from_str(raw).ok();
    }
    Some(Value::String(raw.to_string()))
}

fn value_looks_like_tool_call(value: &Value) -> bool {
    if let Some(list) = value.get("tool_calls").and_then(|list| list.as_array()) {
        return !list.is_empty() && list.iter().all(value_looks_like_tool_call);
    }
    if let Some(list) = value.as_array() {
        return !list.is_empty() && list.iter().all(value_looks_like_tool_call);
    }
    let name = value
        .pointer("/function/name")
        .and_then(|value| value.as_str())
        .or_else(|| value.get("name").and_then(|value| value.as_str()))
        .unwrap_or("")
        .trim();
    !name.is_empty()
}

/// True when streamed assistant text is likely a textual tool call, not chat.
pub fn content_looks_like_tool_prefix(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed.starts_with("```")
        || trimmed.starts_with("<tool_call")
        || trimmed.starts_with("<function=")
        || trimmed.starts_with("<|python_tag|>")
        || trimmed.starts_with("<|tool_call")
        || trimmed.starts_with("[TOOL_CALLS]")
    {
        return true;
    }
    // Python-style / Gemma tool_code: `editor_state(` or `read_file(`
    if let Some(open) = trimmed.find('(') {
        let name = trimmed[..open].trim();
        if !name.is_empty()
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            && (KNOWN_TOOL_NAMES.contains(&name) || looks_like_tool_alias(name))
            && trimmed[open..].starts_with('(')
            && !name.chars().any(|ch| ch.is_whitespace())
            && trimmed[..open]
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return true;
        }
    }
    false
}

fn push_block_calls(inner: &str, calls: &mut Vec<ToolCall>) {
    let trimmed = inner.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Some(value) = first_json(trimmed) {
            push_value_calls(&value, calls);
            return;
        }
    }
    let mut cursor = 0;
    while let Some(rel) = inner[cursor..].find("<function=") {
        let at = cursor + rel;
        if let Some(end) = push_function(inner, at, calls) {
            cursor = end;
        } else {
            cursor = at + "<function=".len();
        }
    }
}

fn push_value_calls(value: &Value, calls: &mut Vec<ToolCall>) {
    if let Some(list) = value.get("tool_calls").and_then(|list| list.as_array()) {
        for item in list {
            push_value_calls(item, calls);
        }
        return;
    }
    if let Some(list) = value.as_array() {
        for item in list {
            push_value_calls(item, calls);
        }
        return;
    }
    if let Some(call) = tool_call_from_value(value, calls.len()) {
        calls.push(call);
    }
}

fn first_json(text: &str) -> Option<Value> {
    first_json_span(text).map(|(value, _)| value)
}

fn first_json_span(text: &str) -> Option<(Value, usize)> {
    let start = text.find(['{', '['])?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    let raw = &text[start..end];
                    let value = serde_json::from_str(raw).ok()?;
                    return Some((value, end));
                }
            }
            _ => {}
        }
    }
    None
}

/// Parses one `<function=name>...</function>` block. The usize is the index just after it.
fn push_function(text: &str, at: usize, calls: &mut Vec<ToolCall>) -> Option<usize> {
    let rest = text.get(at..)?;
    if !rest.starts_with("<function=") {
        return None;
    }
    let name_start = "<function=".len();
    let name_end = rest[name_start..].find('>')?;
    let name = rest[name_start..name_start + name_end].trim().to_string();
    if name.is_empty() {
        return None;
    }
    let body_start = at + name_start + name_end + 1;
    let body_rel = text[body_start..].find("</function>")?;
    let body = &text[body_start..body_start + body_rel];
    let mut params = serde_json::Map::new();
    let mut scan = 0;
    while let Some(rel) = body[scan..].find("<parameter") {
        let tag = scan + rel;
        let head_end = tag + body[tag..].find('>')?;
        let head = &body[tag..head_end];
        let key = if let Some(eq) = head.find("name=") {
            head[eq + "name=".len()..]
                .trim()
                .trim_matches('"')
                .to_string()
        } else if let Some(eq) = head.find('=') {
            head[eq + 1..].trim().trim_matches('"').to_string()
        } else {
            return None;
        };
        if key.is_empty() {
            return None;
        }
        let value_start = head_end + 1;
        let close = body[value_start..].find("</parameter>")?;
        let raw = &body[value_start..value_start + close];
        params.insert(key, Value::String(trim_param(raw)));
        scan = value_start + close + "</parameter>".len();
    }
    calls.push(ToolCall {
        id: format!("call-{}", calls.len()),
        name,
        arguments: Value::Object(params).to_string(),
    });
    Some(body_start + body_rel + "</function>".len())
}

fn trim_param(raw: &str) -> String {
    let raw = raw
        .strip_prefix("\r\n")
        .or_else(|| raw.strip_prefix('\n'))
        .unwrap_or(raw);
    raw.strip_suffix("\r\n")
        .or_else(|| raw.strip_suffix('\n'))
        .unwrap_or(raw)
        .to_string()
}

/// Accumulates streamed `delta` objects into content and tool_calls.
#[derive(Debug, Default)]
pub struct StreamAssembler {
    content: String,
    thinking: String,
    calls: Vec<PartialCall>,
}

#[derive(Debug, Default, Clone)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

impl StreamAssembler {
    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn push_delta(&mut self, delta: &Value) {
        if let Some(text) = delta.get("content").and_then(|value| value.as_str()) {
            self.content.push_str(text);
        }
        for key in ["reasoning", "reasoning_content", "thinking"] {
            if let Some(text) = delta.get(key).and_then(|value| value.as_str()) {
                if !text.is_empty() {
                    self.thinking.push_str(text);
                }
            }
        }
        let Some(list) = delta.get("tool_calls").and_then(|value| value.as_array()) else {
            return;
        };
        for item in list {
            let index = item.get("index").and_then(|value| value.as_u64()).unwrap_or(0) as usize;
            while self.calls.len() <= index {
                self.calls.push(PartialCall::default());
            }
            let slot = &mut self.calls[index];
            if let Some(id) = item.get("id").and_then(|value| value.as_str()) {
                if !id.is_empty() {
                    slot.id = id.to_string();
                }
            }
            if let Some(name) = item
                .pointer("/function/name")
                .and_then(|value| value.as_str())
                .or_else(|| item.get("name").and_then(|value| value.as_str()))
            {
                if !name.is_empty() {
                    slot.name.push_str(name);
                }
            }
            if let Some(args) = item
                .pointer("/function/arguments")
                .and_then(|value| value.as_str())
                .or_else(|| item.get("arguments").and_then(|value| value.as_str()))
            {
                slot.arguments.push_str(args);
            }
        }
    }

    pub fn into_reply(self) -> Result<Reply, String> {
        let mut content = self.content;
        let mut calls = Vec::new();
        for (index, partial) in self.calls.into_iter().enumerate() {
            let name = partial.name.trim();
            if name.is_empty() {
                if !partial.arguments.is_empty() || !partial.id.is_empty() {
                    return Err("Streamed tool call is missing a name".to_string());
                }
                continue;
            }
            let id = if partial.id.is_empty() {
                format!("call-{index}")
            } else {
                partial.id
            };
            calls.push(ToolCall {
                id,
                name: name.to_string(),
                arguments: normalize_tool_arguments(&partial.arguments),
            });
        }
        let (visible, from_tags) = split_think_tags(&content);
        content = visible;
        let mut thinking = self.thinking;
        if !from_tags.is_empty() {
            if !thinking.is_empty() {
                thinking.push('\n');
            }
            thinking.push_str(&from_tags);
        }
        if calls.is_empty() {
            let (found, stripped) = extract_textual(&content);
            if !found.is_empty() {
                calls = found;
                content = stripped;
            }
        }
        if calls.is_empty() {
            let (found, stripped) = extract_textual(&thinking);
            if !found.is_empty() {
                calls = found;
                thinking = stripped;
            }
        }
        Ok(reply_from_parts(content, thinking, calls))
    }
}

/// Why a stream stopped. `Fallback` retries the same turn without streaming;
/// `Fatal` is the answer (a bad key, no credits, a rate limit, an outage, or an
/// error the server sent inside the stream) and a second request would not help.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StreamFailure {
    Fallback(String),
    Fatal(String),
}

impl From<String> for StreamFailure {
    fn from(text: String) -> Self {
        Self::Fallback(text)
    }
}

/// An error object sent inside a stream: OpenRouter's `{"error":{"message":..}}`,
/// Ollama's `{"error":"..."}`, or a choice that finished with `finish_reason: "error"`.
fn stream_error(value: &Value) -> Option<String> {
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        let code = error
            .get("code")
            .filter(|code| !code.is_null())
            .map(|code| match code {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            });
        return Some(match code {
            Some(code) => format!("Stream error {code}: {message}"),
            None => format!("Stream error: {message}"),
        });
    }
    (value
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        == Some("error"))
    .then(|| "Stream error: the provider ended the answer with an error".to_string())
}

/// Parses one SSE payload line (`data: ...`) into a delta value when present.
/// An error object in the stream ends it as [`StreamFailure::Fatal`].
fn sse_event(line: &str) -> Result<Option<Value>, StreamFailure> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return Ok(None);
    }
    let Some(data) = trimmed.strip_prefix("data:") else {
        return Ok(None);
    };
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(data)
        .map_err(|err| StreamFailure::Fallback(format!("Bad stream chunk: {err}")))?;
    if let Some(error) = stream_error(&value) {
        return Err(StreamFailure::Fatal(error));
    }
    Ok(value.pointer("/choices/0/delta").cloned())
}

/// A stream whose request failed with one of these will not do better without streaming.
fn fatal_stream_status(status: reqwest::StatusCode) -> bool {
    !capability_status(status) && !status.is_success()
}

/// Asks for a summary of the conversation in `transcript` and returns its text.
/// One plain request: no tools, no streaming, no thinking. Ollama native gets
/// `num_ctx`; everything else gets the usual OpenAI body. Used by auto compact.
pub async fn summarize(
    client: &reqwest::Client,
    server: &str,
    model: &str,
    transcript: &str,
    options: CompleteOptions<'_>,
) -> Result<String, String> {
    let messages = vec![
        json!({"role": "system", "content": SUMMARY_PROMPT}),
        json!({"role": "user", "content": transcript}),
    ];
    let native = options.ollama_native;
    let url = if native {
        ollama_chat_url(server)
    } else {
        completions_url(server)
    };
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "temperature": 0.2,
    });
    if native {
        let mut extra = serde_json::Map::new();
        if let Some(num_ctx) = options.num_ctx {
            extra.insert("num_ctx".into(), json!(num_ctx));
        }
        extra.insert("temperature".into(), json!(0.2));
        body["options"] = Value::Object(extra);
        body.as_object_mut()
            .map(|object| object.remove("temperature"));
        body["think"] = json!(false);
    } else if let Some(max) = options.tuning.max_tokens {
        body["max_tokens"] = json!(max.min(SUMMARY_MAX_TOKENS));
    }
    let tuning = options.tuning;
    let api_key = options.api_key;
    let response = send_with_retry(
        || prepare(client.post(&url).json(&body), api_key, tuning),
        &url,
        tuning,
    )
    .await?;
    let status = response.status();
    let text = response.text().await.map_err(|err| http_err(&url, err))?;
    if !status.is_success() {
        return Err(status_error(status, &text));
    }
    let value: Value =
        serde_json::from_str(&text).map_err(|err| format!("Bad summary reply: {err}"))?;
    if let Some(problem) = stream_error(&value) {
        return Err(problem);
    }
    let content = value
        .pointer("/choices/0/message/content")
        .or_else(|| value.pointer("/message/content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let (content, _) = split_think_tags(content);
    if content.trim().is_empty() {
        return Err("The summary came back empty".to_string());
    }
    Ok(content.trim().to_string())
}

/// What the model is asked to keep when older turns are folded into a summary.
pub const SUMMARY_PROMPT: &str = "You compact a coding assistant's conversation so it can continue with less context. Write a short summary of the conversation below: the user's goals and constraints, decisions made, files read or changed (with paths), tool results that still matter, errors seen, and what was about to happen next. Plain text or short bullets, no preamble, at most 400 words. Do not call tools.";

/// Upper bound for a summary answer on providers that take `max_tokens`.
const SUMMARY_MAX_TOKENS: u32 = 2048;

pub async fn complete(
    client: &reqwest::Client,
    server: &str,
    model: &str,
    messages: &[Value],
    options: CompleteOptions<'_>,
) -> Result<Reply, String> {
    if options.stream {
        match complete_stream(client, server, model, messages, options, |_, _| {}).await {
            Ok(reply) => return Ok(reply),
            Err(StreamFailure::Fatal(text)) => return Err(text),
            Err(StreamFailure::Fallback(_)) => {}
        }
    }
    let mut options = options;
    options.stream = false;
    complete_once(client, server, model, messages, options, true).await
}

pub async fn complete_with_delta<F>(
    client: &reqwest::Client,
    server: &str,
    model: &str,
    messages: &[Value],
    options: CompleteOptions<'_>,
    mut on_delta: F,
) -> Result<Reply, String>
where
    F: FnMut(DeltaKind, &str),
{
    if options.stream {
        match complete_stream(client, server, model, messages, options, &mut on_delta).await {
            Ok(reply) => return Ok(reply),
            Err(StreamFailure::Fatal(text)) => return Err(text),
            Err(StreamFailure::Fallback(_)) => {}
        }
    }
    let mut options = options;
    options.stream = false;
    complete_once(client, server, model, messages, options, true).await
}

async fn complete_once(
    client: &reqwest::Client,
    server: &str,
    model: &str,
    messages: &[Value],
    options: CompleteOptions<'_>,
    allow_tools: bool,
) -> Result<Reply, String> {
    let native = options.ollama_native;
    let url = if native {
        ollama_chat_url(server)
    } else {
        completions_url(server)
    };
    let api_key = options.api_key;
    let tuning = options.tuning;
    let stream = options.stream;
    let key = capability_key(&url, model);
    let mut allow_tools = allow_tools;
    let mut allow_think = native && options.think && !model_skips_think(&key);
    let mut flattened = false;
    let mut working: Vec<Value> = messages.iter().map(sanitize_tool_message).collect();
    loop {
        let use_tools = allow_tools && !model_skips_tools(&key);
        let use_think = allow_think && !model_skips_think(&key);
        let tools = if use_tools {
            tool_specs()
        } else {
            Vec::new()
        };
        let owned;
        let msgs = if use_tools {
            working.as_slice()
        } else {
            owned = messages_with_tool_prompt(&working);
            owned.as_slice()
        };
        let ollama_msgs;
        let body = if native {
            ollama_msgs = messages_for_ollama_native(msgs);
            ollama_request_body_with(
                model,
                &ollama_msgs,
                &tools,
                stream,
                options.num_ctx,
                use_think,
                tuning.max_tokens,
            )
        } else {
            request_body_with(model, msgs, &tools, stream, tuning.max_tokens)
        };
        let response = send_with_retry(
            || prepare(client.post(&url), api_key, tuning).json(&body),
            &url,
            tuning,
        )
        .await?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| format!("Cannot read the response: {err}"))?;
        if !status.is_success() {
            if use_think && think_not_supported(status, &text) {
                remember_model_without_think(&key);
                allow_think = false;
                continue;
            }
            if use_tools && tools_not_supported(status, &text) {
                remember_model_without_tools(&key);
                allow_tools = false;
                continue;
            }
            if !flattened && invalid_tool_call_arguments(status, &text) {
                working = flatten_tool_history(&working);
                flattened = true;
                continue;
            }
            return Err(status_error(status, &text));
        }
        if let Ok(value) = serde_json::from_str::<Value>(&text)
            && let Some(error) = stream_error(&value)
        {
            return Err(error.replacen("Stream error", "Server error", 1));
        }
        let mut reply = if native {
            parse_ollama_reply(&text)?
        } else {
            parse_reply(&text)?
        };
        reply.history_flattened = flattened;
        reply.usage = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| usage_of(&value));
        return Ok(reply);
    }
}

async fn complete_stream<F>(
    client: &reqwest::Client,
    server: &str,
    model: &str,
    messages: &[Value],
    options: CompleteOptions<'_>,
    mut on_delta: F,
) -> Result<Reply, StreamFailure>
where
    F: FnMut(DeltaKind, &str),
{
    if options.ollama_native {
        return complete_ollama_stream(client, server, model, messages, options, on_delta).await;
    }
    let url = completions_url(server);
    let api_key = options.api_key;
    let tuning = options.tuning;
    let key = capability_key(&url, model);
    let mut allow_tools = true;
    let mut flattened = false;
    let mut working: Vec<Value> = messages.iter().map(sanitize_tool_message).collect();
    let response = loop {
        let use_tools = allow_tools && !model_skips_tools(&key);
        let tools = if use_tools {
            tool_specs()
        } else {
            Vec::new()
        };
        let owned;
        let msgs = if use_tools {
            working.as_slice()
        } else {
            owned = messages_with_tool_prompt(&working);
            owned.as_slice()
        };
        let body = request_body_with(model, msgs, &tools, true, tuning.max_tokens);
        let response = send_with_retry(
            || {
                prepare(client.post(&url), api_key, tuning)
                    .header("Accept", "text/event-stream")
                    .json(&body)
            },
            &url,
            tuning,
        )
        .await?;
        let status = response.status();
        if status.is_success() {
            break response;
        }
        let text = response.text().await.unwrap_or_default();
        if use_tools && tools_not_supported(status, &text) {
            remember_model_without_tools(&key);
            allow_tools = false;
            continue;
        }
        if !flattened && invalid_tool_call_arguments(status, &text) {
            working = flatten_tool_history(&working);
            flattened = true;
            continue;
        }
        let error = status_error(status, &text);
        return Err(if fatal_stream_status(status) {
            StreamFailure::Fatal(error)
        } else {
            StreamFailure::Fallback(error)
        });
    };
    // Stash flattened flag for after assembly.
    let history_flattened = flattened;
    let mut usage: Option<u32> = None;
    let mut assembler = StreamAssembler::default();
    let mut think_split = ContentThinkSplit::default();
    // Byte buffer so a multi-byte UTF-8 char split across TCP chunks is not lossy-decoded.
    let mut byte_buf: Vec<u8> = Vec::new();
    let mut line_buf = String::new();
    let mut saw_event = false;
    let mut hold_deltas = false;
    let mut held = String::new();
    let mut stream = response.bytes_stream();

    let handle_delta = |delta: &Value,
                            assembler: &mut StreamAssembler,
                            think_split: &mut ContentThinkSplit,
                            hold_deltas: &mut bool,
                            held: &mut String,
                            on_delta: &mut F| {
        assembler.push_delta(delta);
        for key in ["reasoning", "reasoning_content", "thinking"] {
            if let Some(text) = delta.get(key).and_then(|value| value.as_str()) {
                if !text.is_empty() {
                    on_delta(DeltaKind::Thinking, text);
                }
            }
        }
        if let Some(text) = delta.get("content").and_then(|value| value.as_str()) {
            if text.is_empty() {
                return;
            }
            if !*hold_deltas && content_looks_like_tool_prefix(assembler.content()) {
                *hold_deltas = true;
            }
            think_split.push(text, |kind, piece| match kind {
                DeltaKind::Thinking => on_delta(DeltaKind::Thinking, piece),
                DeltaKind::Content => {
                    if *hold_deltas {
                        held.push_str(piece);
                    } else {
                        on_delta(DeltaKind::Content, piece);
                    }
                }
            });
        }
    };

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| {
            if err.is_timeout() {
                format!("Stream timed out reading from the model (large tool args / slow local gen). Retry or raise the HTTP timeout.")
            } else {
                format!("Stream read failed: {err}")
            }
        })?;
        byte_buf.extend_from_slice(&chunk);
        let _ = append_valid_utf8(&mut line_buf, &mut byte_buf);
        while let Some(newline) = line_buf.find('\n') {
            let mut line = line_buf[..newline].to_string();
            line_buf.drain(..=newline);
            if line.ends_with('\r') {
                line.pop();
            }
            if let Some(tokens) = sse_usage(&line) {
                usage = Some(tokens);
            }
            let Some(delta) = sse_event(&line)? else {
                continue;
            };
            saw_event = true;
            handle_delta(
                &delta,
                &mut assembler,
                &mut think_split,
                &mut hold_deltas,
                &mut held,
                &mut on_delta,
            );
        }
    }
    if !byte_buf.is_empty() {
        // Flush any remaining bytes with replacement only at end-of-stream.
        line_buf.push_str(&String::from_utf8_lossy(&byte_buf));
        byte_buf.clear();
    }
    if !line_buf.trim().is_empty() {
        if let Some(delta) = sse_event(line_buf.trim())? {
            saw_event = true;
            handle_delta(
                &delta,
                &mut assembler,
                &mut think_split,
                &mut hold_deltas,
                &mut held,
                &mut on_delta,
            );
        }
    }
    think_split.finish(|kind, piece| match kind {
        DeltaKind::Thinking => on_delta(DeltaKind::Thinking, piece),
        DeltaKind::Content => {
            if hold_deltas {
                held.push_str(piece);
            } else {
                on_delta(DeltaKind::Content, piece);
            }
        }
    });
    if !saw_event {
        return Err("Stream produced no events".to_string().into());
    }
    let mut reply = assembler.into_reply()?;
    reply.history_flattened = history_flattened;
    reply.usage = usage;
    if hold_deltas {
        if reply.calls.is_empty() {
            if !held.is_empty() {
                on_delta(DeltaKind::Content, &held);
            }
        } else if !reply.content.trim().is_empty() {
            on_delta(DeltaKind::Content, &reply.content);
        }
    }
    Ok(reply)
}

async fn complete_ollama_stream<F>(
    client: &reqwest::Client,
    server: &str,
    model: &str,
    messages: &[Value],
    options: CompleteOptions<'_>,
    mut on_delta: F,
) -> Result<Reply, StreamFailure>
where
    F: FnMut(DeltaKind, &str),
{
    let url = ollama_chat_url(server);
    let tuning = options.tuning;
    let key = capability_key(&url, model);
    let mut allow_tools = true;
    let mut allow_think = options.think && !model_skips_think(&key);
    let mut flattened = false;
    let mut working: Vec<Value> = messages.iter().map(sanitize_tool_message).collect();
    let response = loop {
        let use_tools = allow_tools && !model_skips_tools(&key);
        let use_think = allow_think && !model_skips_think(&key);
        let tools = if use_tools {
            tool_specs()
        } else {
            Vec::new()
        };
        let owned;
        let msgs = if use_tools {
            working.as_slice()
        } else {
            owned = messages_with_tool_prompt(&working);
            owned.as_slice()
        };
        let ollama_msgs = messages_for_ollama_native(msgs);
        let body = ollama_request_body_with(
            model,
            &ollama_msgs,
            &tools,
            true,
            options.num_ctx,
            use_think,
            tuning.max_tokens,
        );
        let response = send_with_retry(
            || prepare(client.post(&url), options.api_key, tuning).json(&body),
            &url,
            tuning,
        )
        .await?;
        let status = response.status();
        if status.is_success() {
            break response;
        }
        let text = response.text().await.unwrap_or_default();
        if use_think && think_not_supported(status, &text) {
            remember_model_without_think(&key);
            allow_think = false;
            continue;
        }
        if use_tools && tools_not_supported(status, &text) {
            remember_model_without_tools(&key);
            allow_tools = false;
            continue;
        }
        if !flattened && invalid_tool_call_arguments(status, &text) {
            working = flatten_tool_history(&working);
            flattened = true;
            continue;
        }
        let error = status_error(status, &text);
        return Err(if fatal_stream_status(status) {
            StreamFailure::Fatal(error)
        } else {
            StreamFailure::Fallback(error)
        });
    };
    let history_flattened = flattened;
    let mut usage: Option<u32> = None;
    let mut content = String::new();
    let mut thinking = String::new();
    let mut tool_calls = Value::Null;
    let mut think_split = ContentThinkSplit::default();
    let mut byte_buf: Vec<u8> = Vec::new();
    let mut line_buf = String::new();
    let mut saw_event = false;
    let mut hold_deltas = false;
    let mut held = String::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| {
            if err.is_timeout() {
                format!("Stream timed out reading from the model (large tool args / slow local gen). Retry or raise the HTTP timeout.")
            } else {
                format!("Stream read failed: {err}")
            }
        })?;
        byte_buf.extend_from_slice(&chunk);
        let _ = append_valid_utf8(&mut line_buf, &mut byte_buf);
        while let Some(newline) = line_buf.find('\n') {
            let mut line = line_buf[..newline].to_string();
            line_buf.drain(..=newline);
            if line.ends_with('\r') {
                line.pop();
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(error) = stream_error(&value) {
                return Err(StreamFailure::Fatal(error));
            }
            saw_event = true;
            if let Some(tokens) = usage_of(&value) {
                usage = Some(tokens);
            }
            if let Some(message) = value.get("message") {
                for key in ["thinking", "reasoning", "reasoning_content"] {
                    if let Some(piece) = message.get(key).and_then(|v| v.as_str()) {
                        if !piece.is_empty() {
                            thinking.push_str(piece);
                            on_delta(DeltaKind::Thinking, piece);
                        }
                    }
                }
                if let Some(piece) = message.get("content").and_then(|v| v.as_str()) {
                    if !piece.is_empty() {
                        content.push_str(piece);
                        if !hold_deltas && content_looks_like_tool_prefix(&content) {
                            hold_deltas = true;
                        }
                        think_split.push(piece, |kind, text| match kind {
                            DeltaKind::Thinking => {
                                thinking.push_str(text);
                                on_delta(DeltaKind::Thinking, text);
                            }
                            DeltaKind::Content => {
                                if hold_deltas {
                                    held.push_str(text);
                                } else {
                                    on_delta(DeltaKind::Content, text);
                                }
                            }
                        });
                    }
                }
                if let Some(calls) = message.get("tool_calls") {
                    if !calls.is_null() {
                        tool_calls = calls.clone();
                    }
                }
            }
        }
    }
    think_split.finish(|kind, text| match kind {
        DeltaKind::Thinking => {
            thinking.push_str(text);
            on_delta(DeltaKind::Thinking, text);
        }
        DeltaKind::Content => {
            if hold_deltas {
                held.push_str(text);
            } else {
                on_delta(DeltaKind::Content, text);
            }
        }
    });
    if !saw_event {
        return Err("Stream produced no events".to_string().into());
    }
    // Tags were already routed into `thinking` via ContentThinkSplit; strip only.
    let (visible, _) = split_think_tags(&content);
    content = visible;
    let mut raw = json!({"role": "assistant", "content": content});
    if !tool_calls.is_null() {
        raw["tool_calls"] = tool_calls;
    }
    let mut calls = structured_calls(&raw);
    if calls.is_empty() {
        let (found, stripped) = extract_textual(&content);
        if !found.is_empty() {
            calls = found;
            content = stripped;
        }
    }
    let mut reply = reply_from_parts(content, thinking, calls);
    reply.history_flattened = history_flattened;
    reply.usage = usage;
    if hold_deltas {
        if reply.calls.is_empty() {
            if !held.is_empty() {
                on_delta(DeltaKind::Content, &held);
            }
        } else if !reply.content.trim().is_empty() {
            on_delta(DeltaKind::Content, &reply.content);
        }
    }
    Ok(reply)
}

/// Move complete UTF-8 sequences from `bytes` into `out`. Incomplete trailing bytes stay in `bytes`.
fn append_valid_utf8(out: &mut String, bytes: &mut Vec<u8>) -> usize {
    let mut valid_up_to = bytes.len();
    while valid_up_to > 0 {
        match std::str::from_utf8(&bytes[..valid_up_to]) {
            Ok(_) => break,
            Err(err) => {
                if err.error_len().is_some() {
                    // Invalid sequence — skip one byte to avoid hanging.
                    let start = err.valid_up_to();
                    if start > 0 {
                        out.push_str(std::str::from_utf8(&bytes[..start]).unwrap_or(""));
                        bytes.drain(..start + 1);
                        valid_up_to = bytes.len();
                        continue;
                    }
                    bytes.drain(..1);
                    valid_up_to = bytes.len();
                    continue;
                }
                valid_up_to = err.valid_up_to();
            }
        }
    }
    if valid_up_to == 0 {
        return 0;
    }
    out.push_str(std::str::from_utf8(&bytes[..valid_up_to]).unwrap());
    bytes.drain(..valid_up_to);
    valid_up_to
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn request_names_the_tools() {
        let body = request_body("qwen", &[], &tool_specs(), false);
        assert_eq!(body["max_tokens"], 16384);
        assert!(body.get("stream").is_none());
        let streamed = request_body("qwen", &[], &tool_specs(), true);
        assert_eq!(streamed["stream"], true);
        let names: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["function"]["name"].as_str())
            .collect();
        for name in [
            "read_file",
            "list_dir",
            "editor_state",
            "diagnostics",
            "apply_edit",
            "create_file",
            "open_file",
            "save",
            "goto",
        ] {
            assert!(names.contains(&name), "{name}");
        }
    }

    #[test]
    fn fixture_tool_call_parses() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.rs\"}"}}]}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "read_file");
        assert_eq!(reply.calls[0].arguments, "{\"path\":\"a.rs\"}");
        assert!(reply.message["content"].is_null());
    }

    #[test]
    fn reasoning_stays_out_of_content_when_content_is_empty() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"","reasoning":"Use index.html"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert!(reply.calls.is_empty());
        assert!(reply.content.trim().is_empty(), "{}", reply.content);
        assert_eq!(reply.thinking, "Use index.html");
    }

    #[test]
    fn think_tags_are_stripped_from_content() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"<think>plan</think>Hello"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.content, "Hello");
        assert_eq!(reply.thinking, "plan");
        assert!(reply.calls.is_empty());
    }

    #[test]
    fn bare_json_content_tool_call_is_detected() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"{\"name\": \"editor_state\", \"arguments\": {}}"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1, "{reply:?}");
        assert_eq!(reply.calls[0].name, "editor_state");
        assert!(reply.content.trim().is_empty(), "{}", reply.content);
        assert!(reply.message["content"].is_null());
    }

    #[test]
    fn fenced_json_content_tool_call_is_detected() {
        let raw = "{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"```json\\n{\\\"name\\\":\\\"list_dir\\\",\\\"parameters\\\":{\\\"path\\\":\\\".\\\"}}\\n```\"}}]}";
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "list_dir");
        assert!(
            reply.calls[0].arguments.contains('.'),
            "{}",
            reply.calls[0].arguments
        );
        assert!(reply.content.trim().is_empty(), "{}", reply.content);
    }

    #[test]
    fn json_array_of_tool_calls_in_content() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"[{\"name\":\"editor_state\",\"arguments\":{}},{\"name\":\"list_dir\",\"arguments\":{\"path\":\".\"}}]"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 2);
        assert_eq!(reply.calls[0].name, "editor_state");
        assert_eq!(reply.calls[1].name, "list_dir");
    }

    #[test]
    fn arguments_as_json_string_in_content_object() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "read_file");
        assert_eq!(reply.calls[0].arguments, "{\"path\":\"a.rs\"}");
    }

    #[test]
    fn streamed_assembler_parses_bare_json_content() {
        let mut assembler = StreamAssembler::default();
        for piece in ["{\"name\": ", "\"editor_state\", ", "\"arguments\": {}}"] {
            assembler.push_delta(&json!({ "content": piece }));
        }
        let reply = assembler.into_reply().unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "editor_state");
        assert!(reply.content.trim().is_empty(), "{}", reply.content);
    }

    #[test]
    fn a_text_tool_call_becomes_a_call() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"<tool_call>\n{\"name\":\"write_file\",\"arguments\":{\"path\":\"index.html\",\"content\":\"<p>hi</p>\"}}\n</tool_call>"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "write_file");
        assert!(
            reply.calls[0].arguments.contains("index.html"),
            "{}",
            reply.calls[0].arguments
        );
        assert!(reply.content.trim().is_empty(), "{}", reply.content);
        assert!(reply.message["content"].is_null());
    }

    #[test]
    fn a_function_tag_keeps_the_file_text() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"<function=write_file>\n<parameter=path>\nindex.html\n</parameter>\n<parameter=content>\n<p>hi</p>\n</parameter>\n</function>"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "write_file");
        let args: Value = serde_json::from_str(&reply.calls[0].arguments).unwrap();
        assert_eq!(args["path"], "index.html");
        assert_eq!(args["content"], "<p>hi</p>");
    }

    #[test]
    fn a_tool_call_name_at_the_top_level_is_kept() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"1","name":"list_dir","arguments":{"path":"."}}]}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "list_dir");
        assert!(
            reply.calls[0].arguments.contains('.'),
            "{}",
            reply.calls[0].arguments
        );
    }

    #[test]
    fn streamed_tool_call_deltas_assemble() {
        let mut assembler = StreamAssembler::default();
        assembler.push_delta(&json!({
            "tool_calls": [{
                "index": 0,
                "id": "call-1",
                "function": { "name": "read_file", "arguments": "" }
            }]
        }));
        assembler.push_delta(&json!({
            "tool_calls": [{
                "index": 0,
                "function": { "arguments": "{\"path\":" }
            }]
        }));
        assembler.push_delta(&json!({
            "tool_calls": [{
                "index": 0,
                "function": { "arguments": "\"a.rs\"}" }
            }]
        }));
        let reply = assembler.into_reply().unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].id, "call-1");
        assert_eq!(reply.calls[0].name, "read_file");
        assert_eq!(reply.calls[0].arguments, "{\"path\":\"a.rs\"}");
    }

    fn spawn_mock<F>(handler: F) -> (String, thread::JoinHandle<()>)
    where
        F: FnOnce(String) -> String + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 16384];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = handler(request);
            stream.write_all(body.as_bytes()).unwrap();
        });
        (format!("http://{addr}/v1"), server)
    }

    #[tokio::test]
    async fn posts_an_openai_compatible_chat_completion() {
        let (server_url, server) = spawn_mock(|request| {
            assert!(request.starts_with("POST /v1/chat/completions"), "{request}");
            assert!(request.contains("\"model\":\"qwen\""), "{request}");
            assert!(request.contains("\"tools\":"), "{request}");
            assert!(request.contains("read_file"), "{request}");
            assert!(!request.contains("\"stream\":true"), "{request}");
            assert!(
                !request.to_ascii_lowercase().contains("authorization:"),
                "{request}"
            );
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"ok"}}]}"#;
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
        });
        let client = reqwest::Client::new();
        let messages = [serde_json::json!({"role":"user","content":"hi"})];
        let reply = complete(
            &client,
            &server_url,
            "qwen",
            &messages,
            CompleteOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(reply.content, "ok");
        assert!(reply.calls.is_empty());
        server.join().unwrap();
    }

    #[tokio::test]
    async fn sends_bearer_token_only_when_set() {
        let (server_url, server) = spawn_mock(|request| {
            assert!(
                request.to_ascii_lowercase().contains("authorization: bearer secret-key"),
                "{request}"
            );
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"authed"}}]}"#;
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
        });
        let client = reqwest::Client::new();
        let messages = [serde_json::json!({"role":"user","content":"hi"})];
        let reply = complete(
            &client,
            &server_url,
            "qwen",
            &messages,
            CompleteOptions {
                api_key: "secret-key",
                stream: false,
                num_ctx: None,
                ollama_native: false,
                think: true,
                tuning: &DEFAULT_TUNING,
            },
        )
        .await
        .unwrap();
        assert_eq!(reply.content, "authed");
        server.join().unwrap();
    }

    #[tokio::test]
    async fn streams_sse_deltas_and_assembles_tool_calls() {
        let (server_url, server) = spawn_mock(|request| {
            assert!(request.contains("\"stream\":true"), "{request}");
            let events = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"there\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"list_dir\",\"arguments\":\"\"}}]}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\\\".\\\"}\"}}]}}]}\n\n",
                "data: [DONE]\n\n"
            );
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                events.len(),
                events
            )
        });
        let client = reqwest::Client::new();
        let messages = [serde_json::json!({"role":"user","content":"hi"})];
        let mut pieces = Vec::new();
        let reply = complete_with_delta(
            &client,
            &server_url,
            "qwen",
            &messages,
            CompleteOptions {
                api_key: "",
                stream: true,
                num_ctx: None,
                ollama_native: false,
                think: true,
                tuning: &DEFAULT_TUNING,
            },
            |kind, text| {
                if kind == DeltaKind::Content {
                    pieces.push(text.to_string());
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(pieces, vec!["Hi ".to_string(), "there".to_string()]);
        assert_eq!(reply.content, "Hi there");
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "list_dir");
        assert!(reply.calls[0].arguments.contains('.'), "{}", reply.calls[0].arguments);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn streams_qwen_style_tool_json_in_content_without_flashing_deltas() {
        let (server_url, server) = spawn_mock(|request| {
            assert!(request.contains("\"stream\":true"), "{request}");
            let events = concat!(
                "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"{\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"\\\"name\\\": \\\"editor_state\\\", \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"\\\"arguments\\\": {}}\"}}]}\n\n",
                "data: [DONE]\n\n"
            );
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                events.len(),
                events
            )
        });
        let client = reqwest::Client::new();
        let messages = [serde_json::json!({"role":"user","content":"สวัสดี"})];
        let mut pieces = Vec::new();
        let reply = complete_with_delta(
            &client,
            &server_url,
            "qwen2.5-coder:7b",
            &messages,
            CompleteOptions {
                api_key: "",
                stream: true,
                num_ctx: None,
                ollama_native: false,
                think: true,
                tuning: &DEFAULT_TUNING,
            },
            |kind, text| {
                if kind == DeltaKind::Content {
                    pieces.push(text.to_string());
                }
            },
        )
        .await
        .unwrap();
        assert!(
            pieces.is_empty(),
            "raw tool JSON must not stream into the UI: {pieces:?}"
        );
        assert_eq!(reply.calls.len(), 1, "{reply:?}");
        assert_eq!(reply.calls[0].name, "editor_state");
        assert!(reply.content.trim().is_empty(), "{}", reply.content);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn non_stream_qwen_style_tool_json_in_content() {
        let (server_url, server) = spawn_mock(|_request| {
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"{\"name\": \"editor_state\", \"arguments\": {}}" }}]}"#;
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
        });
        let client = reqwest::Client::new();
        let messages = [serde_json::json!({"role":"user","content":"สวัสดี"})];
        let reply = complete(
            &client,
            &server_url,
            "qwen2.5-coder:7b",
            &messages,
            CompleteOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "editor_state");
        assert!(reply.content.trim().is_empty(), "{}", reply.content);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn stream_failure_falls_back_to_non_streaming() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for round in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 16384];
                let n = stream.read(&mut buf).unwrap();
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                if round == 0 {
                    assert!(request.contains("\"stream\":true"), "{request}");
                    let body = "broken";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                } else {
                    assert!(!request.contains("\"stream\":true"), "{request}");
                    let body = r#"{"choices":[{"message":{"role":"assistant","content":"fallback"}}]}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                }
            }
        });
        let client = reqwest::Client::new();
        let server_url = format!("http://{addr}/v1");
        let messages = [serde_json::json!({"role":"user","content":"hi"})];
        let reply = complete(
            &client,
            &server_url,
            "qwen",
            &messages,
            CompleteOptions {
                api_key: "",
                stream: true,
                num_ctx: None,
                ollama_native: false,
                think: true,
                tuning: &DEFAULT_TUNING,
            },
        )
        .await
        .unwrap();
        assert_eq!(reply.content, "fallback");
        server.join().unwrap();
    }

    #[test]
    fn python_style_tool_call_is_detected() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"read_file(path=\"a.rs\", start_line=1)"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1, "{reply:?}");
        assert_eq!(reply.calls[0].name, "read_file");
        let args: Value = serde_json::from_str(&reply.calls[0].arguments).unwrap();
        assert_eq!(args["path"], "a.rs");
        assert_eq!(args["start_line"], 1);
        assert!(reply.content.trim().is_empty(), "{}", reply.content);
    }

    #[test]
    fn gemma_tool_code_fence_is_detected() {
        let raw = "{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"```tool_code\\neditor_state()\\n```\"}}]}";
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1, "{reply:?}");
        assert_eq!(reply.calls[0].name, "editor_state");
        assert!(reply.content.trim().is_empty(), "{}", reply.content);
    }

    #[test]
    fn mistral_tool_calls_marker_then_json() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"[TOOL_CALLS]{\"name\":\"list_dir\",\"arguments\":{\"path\":\".\"}}"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "list_dir");
    }

    #[test]
    fn llama_python_tag_then_call() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"<|python_tag|>editor_state()"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "editor_state");
    }

    #[test]
    fn unknown_text_shape_stays_as_reply_not_hang() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"Here is help: call the read tool somehow {{not json"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert!(reply.calls.is_empty());
        assert!(reply.content.contains("Here is help"));
    }

    #[test]
    fn incomplete_fence_stays_as_reply() {
        let raw = "{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"```json\\n{\\\"name\\\":\\\"list_dir\\\"\"}}]}";
        let reply = parse_reply(raw).unwrap();
        assert!(reply.calls.is_empty());
        assert!(reply.content.contains("```json"));
    }

    #[test]
    fn ollama_chat_url_strips_v1() {
        assert_eq!(
            ollama_chat_url("http://127.0.0.1:11434/v1"),
            "http://127.0.0.1:11434/api/chat"
        );
        assert_eq!(
            ollama_chat_url("http://127.0.0.1:11434/v1/"),
            "http://127.0.0.1:11434/api/chat"
        );
    }

    #[test]
    fn ollama_request_body_sets_num_ctx() {
        let body = ollama_request_body("qwen", &[], &[], false, Some(16384), false);
        assert_eq!(body["options"]["num_ctx"], 16384);
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn messages_for_ollama_native_objectifies_arguments() {
        let args = "{\"path\":\"a.rs\"}";
        let messages = [json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "c1",
                "type": "function",
                "function": { "name": "read_file", "arguments": args }
            }]
        })];
        let out = messages_for_ollama_native(&messages);
        assert!(out[0]["tool_calls"][0]["function"]["arguments"].is_object());
        assert_eq!(out[0]["tool_calls"][0]["function"]["arguments"]["path"], "a.rs");
    }

    #[test]
        fn request_body_omits_tools_when_empty() {
        let body = request_body("tiny", &[], &[], false);
        assert!(body.get("tools").is_none(), "{body}");
    }

    #[test]
    fn messages_with_tool_prompt_appends_catalog() {
        let messages = [json!({"role":"system","content":"Be brief."}), json!({"role":"user","content":"hi"})];
        let out = messages_with_tool_prompt(&messages);
        assert_eq!(out.len(), 2);
        let sys = out[0]["content"].as_str().unwrap();
        assert!(sys.contains("Be brief."));
        assert!(sys.contains("read_file"));
        assert!(sys.contains("Available tools"));
    }

    #[test]
    fn content_prefix_detects_python_style() {
        assert!(content_looks_like_tool_prefix("editor_state("));
        assert!(content_looks_like_tool_prefix("read_file(path="));
        assert!(!content_looks_like_tool_prefix("Hello editor_state("));
        assert!(!content_looks_like_tool_prefix("Sure, I can help."));
    }

    #[test]
    fn append_valid_utf8_holds_incomplete_trailing_bytes() {
        let mut out = String::new();
        let mut bytes = Vec::new();
        // Thai ส = E0 B8 AA — split after first byte
        bytes.extend_from_slice(&[0xE0]);
        let n = append_valid_utf8(&mut out, &mut bytes);
        assert_eq!(n, 0);
        assert!(out.is_empty());
        assert_eq!(bytes, vec![0xE0]);
        bytes.extend_from_slice(&[0xB8, 0xAA, b'x']);
        let n = append_valid_utf8(&mut out, &mut bytes);
        assert_eq!(n, 4);
        assert_eq!(out, "สx");
        assert!(bytes.is_empty());
    }

    #[test]
    fn append_valid_utf8_skips_invalid_byte_without_hanging() {
        let mut out = String::new();
        let mut bytes = vec![0xFF, b'a'];
        append_valid_utf8(&mut out, &mut bytes);
        assert_eq!(out, "a");
        assert!(bytes.is_empty());
    }

    #[tokio::test]
    async fn tools_unsupported_retries_without_tools_and_remembers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let model = format!("no-tools-model-{}", addr.port());
        let model_for_server = model.clone();
        let server = thread::spawn(move || {
            for round in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 65536];
                let n = stream.read(&mut buf).unwrap();
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                if round == 0 {
                    assert!(request.contains("\"tools\":"), "{request}");
                    let body = r#"{"error":{"message":"registry.ollama.ai/library/gemma2 does not support tools"}}"#;
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                } else if round == 1 {
                    assert!(!request.contains("\"tools\":"), "retry must omit tools: {request}");
                    assert!(request.contains("Available tools"), "{request}");
                    assert!(request.contains(&model_for_server), "{request}");
                    let body = r#"{"choices":[{"message":{"role":"assistant","content":"{\"name\":\"editor_state\",\"arguments\":{}}" }}]}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                } else {
                    // Third call (new complete) must skip tools immediately because model was remembered.
                    assert!(!request.contains("\"tools\":"), "remembered model must skip tools: {request}");
                    let body = r#"{"choices":[{"message":{"role":"assistant","content":"ok"}}]}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                }
            }
        });
        let client = reqwest::Client::new();
        let server_url = format!("http://{addr}/v1");
        let messages = [json!({"role":"system","content":"Be helpful."}), json!({"role":"user","content":"hi"})];
        let reply = complete(
            &client,
            &server_url,
            &model,
            &messages,
            CompleteOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "editor_state");
        let reply2 = complete(
            &client,
            &server_url,
            &model,
            &messages,
            CompleteOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(reply2.content, "ok");
        server.join().unwrap();
    }

    
    #[test]
    fn normalize_tool_arguments_empty_and_invalid_become_object() {
        assert_eq!(normalize_tool_arguments(""), "{}");
        assert_eq!(normalize_tool_arguments("   "), "{}");
        assert_eq!(normalize_tool_arguments("{\"a\":"), "{}");
        assert_eq!(normalize_tool_arguments("{}"), "{}");
        assert_eq!(normalize_tool_arguments("{\"path\":\"a.rs\"}"), "{\"path\":\"a.rs\"}");
    }

    #[test]
    fn truncated_create_file_args_are_salvaged() {
        // Mid-stream abort / max_tokens cut inside the HTML body (em dash included).
        let raw = "{\"path\":\"index.html\",\"content\":\"<!DOCTYPE html>\\n<!-- — Tailwind CSS";
        let got = normalize_tool_arguments(raw);
        let value: Value = serde_json::from_str(&got).expect(&got);
        assert_eq!(value["path"], "index.html");
        let body = value["content"]
            .as_str()
            .or_else(|| value["text"].as_str())
            .unwrap();
        assert!(body.contains("DOCTYPE"), "{body}");
        assert!(body.contains('—'), "{body}");
    }

    #[test]
    fn tool_call_inside_reasoning_content_is_recovered() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"","reasoning_content":"Let me create the file.\n{\"name\":\"create_file\",\"arguments\":{\"path\":\"index.html\",\"text\":\"<p>hi—สวัสดี👋</p>\"}}"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1, "{reply:?}");
        assert_eq!(reply.calls[0].name, "create_file");
        assert!(
            reply.calls[0].arguments.contains("index.html"),
            "{}",
            reply.calls[0].arguments
        );
    }

    #[test]
    fn think_tags_then_textual_tool_call_with_multibyte() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"<think>plan — สวัสดี 👋</think>\n{\"name\":\"create_file\",\"arguments\":{\"path\":\"i.html\",\"text\":\"ok\"}}"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.thinking, "plan — สวัสดี 👋");
        assert_eq!(reply.calls.len(), 1, "{reply:?}");
        assert_eq!(reply.calls[0].name, "create_file");
    }

    #[test]
    fn stream_assembler_recovers_tool_call_from_reasoning_deltas() {
        let mut assembler = StreamAssembler::default();
        assembler.push_delta(&json!({
            "reasoning_content": "Let me create the complete index.html file — "
        }));
        assembler.push_delta(&json!({
            "reasoning_content": "now.\n{\"name\":\"create_file\",\"arguments\":{\"path\":\"index.html\",\"text\":\"x\"}}"
        }));
        let reply = assembler.into_reply().unwrap();
        assert_eq!(reply.calls.len(), 1, "{reply:?}");
        assert_eq!(reply.calls[0].name, "create_file");
        assert!(reply.content.trim().is_empty());
    }

    #[test]
    fn content_think_split_handles_multibyte_chunks_before_close() {
        let mut split = ContentThinkSplit::default();
        let mut thinking = String::new();
        let mut content = String::new();
        let mut emit = |kind: DeltaKind, piece: &str| match kind {
            DeltaKind::Thinking => thinking.push_str(piece),
            DeltaKind::Content => content.push_str(piece),
        };
        // Split tag / multibyte across pushes the way SSE chunks do.
        split.push("<thi", &mut emit);
        split.push("nk>plan — ", &mut emit);
        split.push("สวัสดี", &mut emit);
        split.push("</thi", &mut emit);
        split.push(
            "nk>{\"name\":\"list_dir\",\"arguments\":{\"path\":\".\"}}",
            &mut emit,
        );
        split.finish(&mut emit);
        assert!(thinking.contains("plan"), "{thinking}");
        assert!(thinking.contains('—'), "{thinking}");
        assert!(thinking.contains("สวัสดี"), "{thinking}");
        assert!(content.contains("list_dir"), "{content}");
    }

    #[test]
    fn json_calls_always_emit_string_arguments() {
        let formats = [
            parse_reply(r#"{"choices":[{"message":{"role":"assistant","content":"{\"name\":\"editor_state\",\"arguments\":{}}"}}]}"#).unwrap(),
            parse_reply(r#"{"choices":[{"message":{"role":"assistant","content":"read_file(path=\"a.rs\")"}}]}"#).unwrap(),
            parse_reply("{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"```tool_code\\neditor_state()\\n```\"}}]}").unwrap(),
            parse_reply(r#"{"choices":[{"message":{"role":"assistant","content":"<tool_call>\n{\"name\":\"list_dir\",\"arguments\":{\"path\":\".\"}}\n</tool_call>"}}]}"#).unwrap(),
            parse_reply(r#"{"choices":[{"message":{"role":"assistant","content":"[TOOL_CALLS]{\"name\":\"list_dir\",\"arguments\":{\"path\":\".\"}}"}}]}"#).unwrap(),
            parse_reply(r#"{"choices":[{"message":{"role":"assistant","content":"<|python_tag|>editor_state()"}}]}"#).unwrap(),
            parse_reply(r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"1","type":"function","function":{"name":"editor_state","arguments":""}}]}}]}"#).unwrap(),
        ];
        for reply in formats {
            assert!(!reply.calls.is_empty(), "{reply:?}");
            let wire = json_calls(&reply.calls);
            for call in wire.as_array().unwrap() {
                assert_eq!(call["type"], "function");
                assert!(call["id"].as_str().is_some_and(|s| !s.is_empty()));
                let args = call["function"]["arguments"].as_str().expect("arguments must be a JSON string");
                let parsed: Value = serde_json::from_str(args).expect(args);
                assert!(parsed.is_object(), "{args}");
            }
            // Message embedded for history must match the same shape.
            let msg_calls = reply.message["tool_calls"].as_array().unwrap();
            for call in msg_calls {
                let args = call["function"]["arguments"].as_str().unwrap();
                assert!(serde_json::from_str::<Value>(args).unwrap().is_object());
            }
        }
    }

    #[test]
    fn sanitize_repairs_empty_and_object_arguments() {
        let bad = json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "",
                "function": { "name": "editor_state", "arguments": "" }
            }, {
                "name": "list_dir",
                "arguments": {"path": "."}
            }]
        });
        let fixed = sanitize_tool_message(&bad);
        let calls = fixed["tool_calls"].as_array().unwrap();
        assert_eq!(calls[0]["id"], "call-0");
        assert_eq!(calls[0]["type"], "function");
        assert_eq!(calls[0]["function"]["arguments"], "{}");
        assert_eq!(calls[1]["function"]["arguments"].as_str().unwrap(), "{\"path\":\".\"}");
    }

    #[test]
    fn flatten_tool_history_removes_tool_calls() {
        let messages = vec![
            json!({"role":"user","content":"hi"}),
            json!({
                "role":"assistant",
                "content": null,
                "tool_calls":[{"id":"c1","type":"function","function":{"name":"editor_state","arguments":""}}]
            }),
            json!({"role":"tool","tool_call_id":"c1","content":"ok"}),
            json!({"role":"user","content":"next"}),
        ];
        let flat = flatten_tool_history(&messages);
        assert!(flat.iter().all(|m| m.get("tool_calls").is_none()));
        assert!(flat.iter().all(|m| m["role"] != "tool"));
        assert!(flat.iter().any(|m| m["content"].as_str().unwrap_or("").contains("Called editor_state")));
    }

    #[test]
    fn real_ollama_qwen_content_json_fixture() {
        // Exact non-stream body shape from local Ollama + qwen2.5-coder:0.5b (greeting).
        let raw = r#"{"id":"chatcmpl-337","choices":[{"message":{"role":"assistant","content":"{\"name\": \"editor_state\", \"arguments\": {\"file\": \"(none)\", \"line\": 1, \"col\": 1, \"selection\": \"none\"}}"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].name, "editor_state");
        assert!(reply.content.trim().is_empty(), "{}", reply.content);
        let args: Value = serde_json::from_str(&reply.calls[0].arguments).unwrap();
        assert_eq!(args["file"], "(none)");
    }


    #[tokio::test]
    async fn live_ollama_rejects_empty_args_but_sanitize_recovers() {
        let client = reqwest::Client::new();
        let ok = client
            .get("http://127.0.0.1:11434/api/tags")
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        // Poisoned history shape that Ollama returns as "invalid tool call arguments".
        let poisoned = vec![
            json!({"role": "user", "content": "hi"}),
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": { "name": "editor_state", "arguments": "" }
                }]
            }),
            json!({"role": "tool", "tool_call_id": "call-1", "content": "file=(none)"}),
            json!({"role": "user", "content": "say ready"}),
        ];
        let reply = complete(
            &client,
            "http://127.0.0.1:11434/v1",
            "qwen2.5-coder:0.5b",
            &poisoned,
            CompleteOptions::default(),
        )
        .await;
        assert!(
            reply.is_ok(),
            "sanitize/flatten must recover from empty tool arguments: {reply:?}"
        );
        // Sanitized outbound path: empty args become "{}".
        let sanitized: Vec<_> = poisoned.iter().map(sanitize_tool_message).collect();
        let args = sanitized[1]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert_eq!(args, "{}");
    }

    #[tokio::test]
    async fn live_ollama_qwen3_thinking_stream() {
        let client = reqwest::Client::new();
        let ok = client
            .get("http://127.0.0.1:11434/api/tags")
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        let messages = [serde_json::json!({
            "role": "user",
            "content": "Reply with exactly the word hi and nothing else."
        })];
        let mut thinking = String::new();
        let mut content = String::new();
        let reply = complete_with_delta(
            &client,
            "http://127.0.0.1:11434/v1",
            "qwen3:0.6b",
            &messages,
            CompleteOptions {
                api_key: "",
                stream: true,
                num_ctx: Some(2048),
                ollama_native: true,
                think: true,
                tuning: &DEFAULT_TUNING,
            },
            |kind, text| match kind {
                DeltaKind::Thinking => thinking.push_str(text),
                DeltaKind::Content => content.push_str(text),
            },
        )
        .await;
        let Ok(reply) = reply else {
            eprintln!("live thinking request failed: {reply:?}");
            return;
        };
        eprintln!(
            "live thinking_len={} content_len={} reply.thinking_len={} reply.content={:?}",
            thinking.len(),
            content.len(),
            reply.thinking.len(),
            reply.content.chars().take(80).collect::<String>()
        );
        assert!(
            !thinking.is_empty() || !reply.thinking.is_empty() || !reply.content.trim().is_empty(),
            "expected thinking and/or content from qwen3: {reply:?}"
        );
        assert!(
            !reply.content.contains("<think>") && !reply.content.contains("</think>"),
            "think tags leaked into content: {}",
            reply.content
        );
    }

    #[tokio::test]
    async fn live_ollama_stream_content_json_if_available() {
        let client = reqwest::Client::new();
        let ok = client
            .get("http://127.0.0.1:11434/api/tags")
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        let messages = [serde_json::json!({
            "role": "user",
            "content": "สวัสดี\n\neditor_state: file=(none) line=1 col=1 selection=none"
        })];
        let mut pieces = Vec::new();
        let reply = complete_with_delta(
            &client,
            "http://127.0.0.1:11434/v1",
            "qwen2.5-coder:0.5b",
            &messages,
            CompleteOptions {
                api_key: "",
                stream: true,
                num_ctx: None,
                ollama_native: false,
                think: true,
                tuning: &DEFAULT_TUNING,
            },
            |kind, text| {
                if kind == DeltaKind::Content {
                    pieces.push(text.to_string());
                }
            },
        )
        .await;
        let Ok(reply) = reply else {
            return;
        };
        // Tiny Qwen often emits editor_state as content JSON; we must not leave it as chat text.
        if reply.calls.is_empty() {
            assert!(
                !reply.content.trim_start().starts_with('{'),
                "raw tool JSON leaked as content: {}",
                reply.content
            );
        } else {
            assert!(
                pieces.is_empty() || !pieces.join("").trim_start().starts_with('{'),
                "stream flashed tool JSON: {pieces:?}"
            );
            assert!(
                reply.calls.iter().any(|c| c.name.contains("editor_state") || !c.name.is_empty()),
                "{reply:?}"
            );
        }
    }

    /// Accepts `rounds` connections in order. `handler(round, request)` writes the answer.
    /// The join handle returns every request it saw.
    fn spawn_rounds<F>(rounds: usize, handler: F) -> (String, thread::JoinHandle<Vec<String>>)
    where
        F: Fn(usize, &str) -> String + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut seen = Vec::new();
            for round in 0..rounds {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                let answer = handler(round, &request);
                stream.write_all(answer.as_bytes()).unwrap();
                seen.push(request);
            }
            seen
        });
        (format!("http://{addr}/v1"), server)
    }

    /// Headers plus the whole `Content-Length` body, however many reads that takes.
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 16384];
        loop {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&bytes);
            if let Some(head_end) = text.find("\r\n\r\n") {
                let length = text[..head_end]
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if bytes.len() >= head_end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&bytes).to_string()
    }

    fn http(status: &str, content_type: &str, extra: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn ok_json(content: &str) -> String {
        let body =
            json!({"choices":[{"message":{"role":"assistant","content":content}}]}).to_string();
        http("200 OK", "application/json", "", &body)
    }

    fn body_of(request: &str) -> Value {
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        serde_json::from_str(body).unwrap()
    }

    fn quick(retries: u32) -> Tuning {
        Tuning {
            retries,
            backoff: Duration::from_millis(1),
            ..Tuning::default()
        }
    }

    #[test]
    fn default_tuning_keeps_the_lm_studio_and_ollama_bodies_byte_for_byte() {
        let messages = [json!({"role":"user","content":"hi"})];
        // The exact bytes the client sent before Max tokens existed.
        assert_eq!(
            request_body_with("qwen", &messages, &[], true, DEFAULT_TUNING.max_tokens).to_string(),
            r#"{"max_tokens":16384,"messages":[{"content":"hi","role":"user"}],"model":"qwen","stream":true,"temperature":0.2}"#
        );
        assert_eq!(
            ollama_request_body_with(
                "qwen",
                &messages,
                &[],
                true,
                Some(16384),
                true,
                DEFAULT_TUNING.max_tokens
            )
            .to_string(),
            r#"{"messages":[{"content":"hi","role":"user"}],"model":"qwen","options":{"num_ctx":16384,"num_predict":16384,"temperature":0.2},"stream":true,"think":true}"#
        );
        assert_eq!(
            request_body("qwen", &messages, &[], false),
            request_body_with("qwen", &messages, &[], false, Some(16384))
        );
        let omitted = request_body_with("qwen", &messages, &[], false, None);
        assert!(omitted.get("max_tokens").is_none(), "{omitted}");
        let omitted = ollama_request_body_with("qwen", &messages, &[], false, None, false, None);
        assert!(omitted["options"].get("num_predict").is_none(), "{omitted}");
    }

    #[tokio::test]
    async fn default_tuning_sends_no_extra_headers() {
        let (url, server) = spawn_rounds(1, |_, _| ok_json("ok"));
        let client = reqwest::Client::new();
        let messages = [json!({"role":"user","content":"hi"})];
        let reply = complete(&client, &url, "qwen", &messages, CompleteOptions::default())
            .await
            .unwrap();
        assert_eq!(reply.content, "ok");
        let request = server.join().unwrap().remove(0).to_ascii_lowercase();
        assert!(!request.contains("http-referer"), "{request}");
        assert!(!request.contains("x-title"), "{request}");
        assert!(!request.contains("authorization"), "{request}");
        assert_eq!(body_of(&request)["max_tokens"], json!(16384));
    }

    #[tokio::test]
    async fn openrouter_tuning_sends_its_headers_and_leaves_out_max_tokens() {
        let (url, server) = spawn_rounds(1, |_, _| ok_json("routed"));
        let client = reqwest::Client::new();
        let tuning = Tuning {
            max_tokens: None,
            headers: vec![
                (
                    "HTTP-Referer".into(),
                    "https://github.com/Manuree/duckx-code".into(),
                ),
                ("X-Title".into(), "duckx-code".into()),
            ],
            ..quick(0)
        };
        let options = CompleteOptions {
            api_key: "sk-or-test",
            tuning: &tuning,
            ..CompleteOptions::default()
        };
        let messages = [json!({"role":"user","content":"hi"})];
        let reply = complete(
            &client,
            &url,
            "anthropic/claude-sonnet-4",
            &messages,
            options,
        )
        .await
        .unwrap();
        assert_eq!(reply.content, "routed");
        let request = server.join().unwrap().remove(0);
        let lower = request.to_ascii_lowercase();
        assert!(
            lower.contains("authorization: bearer sk-or-test"),
            "{request}"
        );
        assert!(
            lower.contains("http-referer: https://github.com/manuree/duckx-code"),
            "{request}"
        );
        assert!(lower.contains("x-title: duckx-code"), "{request}");
        let body = body_of(&request);
        assert!(body.get("max_tokens").is_none(), "{body}");
        assert_eq!(body["model"], json!("anthropic/claude-sonnet-4"));
    }

    #[tokio::test]
    async fn a_429_is_retried_after_retry_after_and_then_succeeds() {
        let (url, server) = spawn_rounds(3, |round, _| match round {
            0 => http(
                "429 Too Many Requests",
                "application/json",
                "Retry-After: 0\r\n",
                r#"{"error":{"message":"slow down"}}"#,
            ),
            1 => http("503 Service Unavailable", "text/plain", "", "busy"),
            _ => ok_json("third time"),
        });
        let client = reqwest::Client::new();
        let tuning = quick(2);
        let options = CompleteOptions {
            tuning: &tuning,
            ..CompleteOptions::default()
        };
        let messages = [json!({"role":"user","content":"hi"})];
        let reply = complete(&client, &url, "retry-model", &messages, options)
            .await
            .unwrap();
        assert_eq!(reply.content, "third time");
        let seen = server.join().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(
            body_of(&seen[0]),
            body_of(&seen[2]),
            "a retry resends the same body"
        );
    }

    #[tokio::test]
    async fn retries_stop_at_the_setting_and_report_the_last_status() {
        let (url, server) = spawn_rounds(2, |_, _| {
            http(
                "500 Internal Server Error",
                "application/json",
                "",
                r#"{"error":{"message":"upstream exploded"}}"#,
            )
        });
        let client = reqwest::Client::new();
        let tuning = quick(1);
        let options = CompleteOptions {
            tuning: &tuning,
            ..CompleteOptions::default()
        };
        let messages = [json!({"role":"user","content":"hi"})];
        let err = complete(&client, &url, "flaky", &messages, options)
            .await
            .unwrap_err();
        assert!(err.contains("500"), "{err}");
        assert!(err.contains("upstream exploded"), "{err}");
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn auth_credit_rate_and_server_errors_never_mean_a_model_lacks_tools_or_think() {
        let body = "tool use is not supported; think is not supported; unknown field tools";
        for code in [401u16, 402, 403, 408, 429, 500, 502, 503] {
            let status = reqwest::StatusCode::from_u16(code).unwrap();
            assert!(!tools_not_supported(status, body), "{code}");
            assert!(!think_not_supported(status, body), "{code}");
        }
        for code in [400u16, 404, 422] {
            let status = reqwest::StatusCode::from_u16(code).unwrap();
            assert!(tools_not_supported(status, body), "{code}");
            assert!(think_not_supported(status, body), "{code}");
        }
        let openrouter =
            r#"{"error":{"message":"No endpoints found that support tool use.","code":404}}"#;
        assert!(tools_not_supported(
            reqwest::StatusCode::NOT_FOUND,
            openrouter
        ));
    }

    #[tokio::test]
    async fn a_401_with_tool_words_keeps_tools_for_the_next_turn() {
        let (url, server) = spawn_rounds(2, |round, _| match round {
            0 => http(
                "401 Unauthorized",
                "application/json",
                "",
                r#"{"error":{"message":"invalid key: tool use is not supported for anonymous users"}}"#,
            ),
            _ => ok_json("ok"),
        });
        let client = reqwest::Client::new();
        let messages = [json!({"role":"user","content":"hi"})];
        let model = "cloud-401-model";
        let err = complete(&client, &url, model, &messages, CompleteOptions::default())
            .await
            .unwrap_err();
        assert!(err.contains("401"), "{err}");
        assert!(err.contains("API key"), "{err}");
        assert!(!model_skips_tools(&capability_key(
            &completions_url(&url),
            model
        )));
        complete(&client, &url, model, &messages, CompleteOptions::default())
            .await
            .unwrap();
        let seen = server.join().unwrap();
        assert!(
            body_of(&seen[1]).get("tools").is_some(),
            "tools must still be sent"
        );
    }

    #[tokio::test]
    async fn a_streamed_402_is_final_and_does_not_fall_back() {
        let (url, server) = spawn_rounds(1, |_, request| {
            assert!(request.contains("\"stream\":true"), "{request}");
            http(
                "402 Payment Required",
                "application/json",
                "",
                r#"{"error":{"message":"Insufficient credits"}}"#,
            )
        });
        let client = reqwest::Client::new();
        let options = CompleteOptions {
            stream: true,
            ..CompleteOptions::default()
        };
        let messages = [json!({"role":"user","content":"hi"})];
        let err = complete_with_delta(&client, &url, "paid", &messages, options, |_, _| {})
            .await
            .unwrap_err();
        assert!(err.contains("402"), "{err}");
        assert!(err.contains("Insufficient credits"), "{err}");
        assert_eq!(
            server.join().unwrap().len(),
            1,
            "no second, non-streaming request"
        );
    }

    #[tokio::test]
    async fn an_error_event_inside_the_stream_is_reported() {
        let (url, server) = spawn_rounds(1, |_, _| {
            let events = concat!(
                ": OPENROUTER PROCESSING\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
                "data: {\"error\":{\"message\":\"Provider returned error\",\"code\":502}}\n\n",
            );
            http("200 OK", "text/event-stream", "", events)
        });
        let client = reqwest::Client::new();
        let options = CompleteOptions {
            stream: true,
            ..CompleteOptions::default()
        };
        let messages = [json!({"role":"user","content":"hi"})];
        let mut shown = String::new();
        let err = complete_with_delta(
            &client,
            &url,
            "mid-stream",
            &messages,
            options,
            |_, text| shown.push_str(text),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "Stream error 502: Provider returned error");
        assert_eq!(shown, "Hel");
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_ollama_error_line_ends_the_native_stream() {
        let (url, server) = spawn_rounds(1, |_, request| {
            assert!(request.starts_with("POST /api/chat"), "{request}");
            http(
                "200 OK",
                "application/x-ndjson",
                "",
                "{\"error\":\"model runner has unexpectedly stopped\"}\n",
            )
        });
        let client = reqwest::Client::new();
        let options = CompleteOptions {
            stream: true,
            ollama_native: true,
            ..CompleteOptions::default()
        };
        let messages = [json!({"role":"user","content":"hi"})];
        let err = complete_with_delta(&client, &url, "ollama-crash", &messages, options, |_, _| {})
            .await
            .unwrap_err();
        assert!(
            err.contains("model runner has unexpectedly stopped"),
            "{err}"
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn a_200_with_an_error_object_is_an_error() {
        let (url, server) = spawn_rounds(1, |_, _| {
            http(
                "200 OK",
                "application/json",
                "",
                r#"{"error":{"message":"Model is overloaded","code":503}}"#,
            )
        });
        let client = reqwest::Client::new();
        let messages = [json!({"role":"user","content":"hi"})];
        let err = complete(
            &client,
            &url,
            "overloaded",
            &messages,
            CompleteOptions::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "Server error 503: Model is overloaded");
        server.join().unwrap();
    }

    #[tokio::test]
    async fn openrouter_no_tool_endpoint_falls_back_to_the_tool_prompt() {
        let (url, server) = spawn_rounds(2, |round, _| match round {
            0 => http(
                "404 Not Found",
                "application/json",
                "",
                r#"{"error":{"message":"No endpoints found that support tool use.","code":404}}"#,
            ),
            _ => ok_json("plain"),
        });
        let client = reqwest::Client::new();
        let messages = [
            json!({"role":"system","content":"sys"}),
            json!({"role":"user","content":"hi"}),
        ];
        let reply = complete(
            &client,
            &url,
            "no-tools/model",
            &messages,
            CompleteOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(reply.content, "plain");
        let seen = server.join().unwrap();
        let second = body_of(&seen[1]);
        assert!(second.get("tools").is_none(), "{second}");
        assert!(
            second["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Available tools:")
        );
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        let tuning = Tuning {
            backoff: Duration::from_secs(1),
            ..Tuning::default()
        };
        assert_eq!(backoff_for(&tuning, 0), Duration::from_secs(1));
        assert_eq!(backoff_for(&tuning, 2), Duration::from_secs(4));
        assert_eq!(backoff_for(&tuning, 20), MAX_BACKOFF);
    }

    #[test]
    fn models_url_uses_api_tags_only_for_ollama() {
        assert_eq!(
            models_url("http://127.0.0.1:11434/v1/", true),
            "http://127.0.0.1:11434/api/tags"
        );
        assert_eq!(
            models_url("https://openrouter.ai/api/v1", false),
            "https://openrouter.ai/api/v1/models"
        );
        assert_eq!(
            parse_model_names(r#"{"data":[{"id":"openai/gpt-4o"},{"id":"x/y"}]}"#),
            ["openai/gpt-4o", "x/y"]
        );
        assert_eq!(
            parse_model_names(r#"{"models":[{"name":"qwen3:8b"}]}"#),
            ["qwen3:8b"]
        );
        assert!(parse_model_names("not json").is_empty());
    }

    #[test]
    fn listing_models_sends_the_key_and_reports_a_401() {
        let (url, server) = spawn_rounds(2, |round, request| {
            assert!(request.starts_with("GET /v1/models"), "{request}");
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer sk-list"),
                "{request}"
            );
            assert!(
                request.to_ascii_lowercase().contains("x-title: duckx-code"),
                "{request}"
            );
            match round {
                0 => ok_json_raw(r#"{"data":[{"id":"openai/gpt-4o-mini"}]}"#),
                _ => http(
                    "401 Unauthorized",
                    "application/json",
                    "",
                    r#"{"error":{"message":"No auth credentials found"}}"#,
                ),
            }
        });
        let models = models_url(&url, false);
        let headers = vec![("X-Title".to_string(), "duckx-code".to_string())];
        assert_eq!(
            list_models(&models, "sk-list", &headers).unwrap(),
            ["openai/gpt-4o-mini"]
        );
        let err = list_models(&models, "sk-list", &headers).unwrap_err();
        assert!(
            err.contains("401") && err.contains("No auth credentials found"),
            "{err}"
        );
        server.join().unwrap();
    }

    fn ok_json_raw(body: &str) -> String {
        http("200 OK", "application/json", "", body)
    }

    #[test]
    fn usage_reads_openai_and_ollama_counts() {
        assert_eq!(
            usage_of(&json!({"usage": {"prompt_tokens": 900, "completion_tokens": 100}})),
            Some(1000)
        );
        assert_eq!(usage_of(&json!({"usage": {"total_tokens": 42}})), Some(42));
        assert_eq!(
            usage_of(&json!({"prompt_eval_count": 300, "eval_count": 20, "done": true})),
            Some(320)
        );
        assert_eq!(usage_of(&json!({"choices": []})), None);
        assert_eq!(
            sse_usage(r#"data: {"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3}}"#),
            Some(10)
        );
        assert_eq!(sse_usage(r#"data: {"choices":[{"delta":{}}]}"#), None);
    }

    #[tokio::test]
    async fn a_completion_carries_the_usage_the_server_reported() {
        let (url, server) = spawn_rounds(1, |_, _| {
            ok_json_raw(
                r#"{"choices":[{"message":{"role":"assistant","content":"hi"}}],"usage":{"prompt_tokens":120,"completion_tokens":5}}"#,
            )
        });
        let client = reqwest::Client::new();
        let messages = [json!({"role":"user","content":"hi"})];
        let reply = complete(&client, &url, "m", &messages, CompleteOptions::default())
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(reply.usage, Some(125));
    }

    #[tokio::test]
    async fn summarize_sends_one_plain_request_and_returns_the_text() {
        let (url, server) = spawn_rounds(1, |_, _| {
            ok_json("<think>hm</think>User is fixing the parser.")
        });
        let client = reqwest::Client::new();
        let summary = summarize(
            &client,
            &url,
            "m",
            "user: fix the parser",
            CompleteOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(summary, "User is fixing the parser.");
        let seen = server.join().unwrap();
        let body = body_of(&seen[0]);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "user: fix the parser");
        assert!(body.get("tools").is_none(), "{body}");
        assert_ne!(body["stream"], json!(true), "{body}");
    }

    #[tokio::test]
    async fn summarize_reports_a_server_error_instead_of_an_empty_summary() {
        let (url, server) = spawn_rounds(1, |_, _| {
            http(
                "400 Bad Request",
                "application/json",
                "",
                r#"{"error":{"message":"context too long"}}"#,
            )
        });
        let client = reqwest::Client::new();
        let tuning = quick(0);
        let options = CompleteOptions {
            tuning: &tuning,
            ..CompleteOptions::default()
        };
        let err = summarize(&client, &url, "m", "user: hi", options)
            .await
            .unwrap_err();
        server.join().unwrap();
        assert!(err.contains("context too long"), "{err}");
    }
}
