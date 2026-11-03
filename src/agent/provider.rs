//! OpenAI-compatible chat completions. Ollama and LM Studio both speak this.
//! Streaming uses SSE (`data:` lines); a failed stream falls back to one JSON body.

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
    pub calls: Vec<ToolCall>,
    /// The assistant message, sent back before each tool result.
    pub message: Value,
}

#[derive(Debug, Clone, Copy)]
pub struct CompleteOptions<'a> {
    pub api_key: &'a str,
    pub stream: bool,
}

impl Default for CompleteOptions<'static> {
    fn default() -> Self {
        Self {
            api_key: "",
            stream: false,
        }
    }
}

pub fn completions_url(server: &str) -> String {
    format!("{}/chat/completions", server.trim().trim_end_matches('/'))
}

pub fn request_body(model: &str, messages: &[Value], tools: &[Value], stream: bool) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages,
        "tools": tools,
        "temperature": 0.2,
        "max_tokens": 8192
    });
    if stream {
        body["stream"] = Value::Bool(true);
    }
    body
}

fn apply_auth(request: reqwest::RequestBuilder, api_key: &str) -> reqwest::RequestBuilder {
    let key = api_key.trim();
    if key.is_empty() {
        request
    } else {
        request.header("Authorization", format!("Bearer {key}"))
    }
}

pub fn parse_reply(text: &str) -> Result<Reply, String> {
    let value: Value = serde_json::from_str(text).map_err(|err| format!("Bad response: {err}"))?;
    let raw = value
        .pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| "Response has no message".to_string())?;
    let mut content = message_text(&raw["content"]);
    let mut calls = structured_calls(&raw);
    if calls.is_empty() {
        let (found, stripped) = extract_textual(&content);
        if found.is_empty() {
            let (from_reason, _) = extract_textual(&reasoning_text(&raw));
            if from_reason.is_empty() {
                if content.trim().is_empty() {
                    content = reasoning_text(&raw);
                }
            } else {
                calls = from_reason;
                content.clear();
            }
        } else {
            calls = found;
            content = stripped;
        }
    }
    Ok(reply_from_parts(content, calls))
}

fn reply_from_parts(content: String, calls: Vec<ToolCall>) -> Reply {
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
        calls,
        message,
    }
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
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn json_calls(calls: &[ToolCall]) -> Value {
    Value::Array(
        calls
            .iter()
            .map(|call| {
                json!({
                    "id": call.id,
                    "type": "function",
                    "function": {
                        "name": call.name,
                        "arguments": call.arguments
                    }
                })
            })
            .collect(),
    )
}

/// Tool calls some local models write as text. The string is the reply with those calls removed.
fn extract_textual(text: &str) -> (Vec<ToolCall>, String) {
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
                    let raw = &text[start..start + offset + ch.len_utf8()];
                    return serde_json::from_str(raw).ok();
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
    calls: Vec<PartialCall>,
}

#[derive(Debug, Default, Clone)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

impl StreamAssembler {
    pub fn push_delta(&mut self, delta: &Value) {
        if let Some(text) = delta.get("content").and_then(|value| value.as_str()) {
            self.content.push_str(text);
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
                arguments: partial.arguments,
            });
        }
        if calls.is_empty() {
            let (found, stripped) = extract_textual(&content);
            if !found.is_empty() {
                calls = found;
                content = stripped;
            }
        }
        Ok(reply_from_parts(content, calls))
    }
}

/// Parses one SSE payload line (`data: ...`) into a delta value when present.
pub fn sse_data_delta(line: &str) -> Result<Option<Value>, String> {
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
    let value: Value =
        serde_json::from_str(data).map_err(|err| format!("Bad stream chunk: {err}"))?;
    Ok(value.pointer("/choices/0/delta").cloned())
}

pub async fn complete(
    client: &reqwest::Client,
    server: &str,
    model: &str,
    messages: &[Value],
    options: CompleteOptions<'_>,
) -> Result<Reply, String> {
    if options.stream {
        match complete_stream(client, server, model, messages, options.api_key, |_| {}).await {
            Ok(reply) => return Ok(reply),
            Err(_) => {
                // Fall through to a non-streaming request.
            }
        }
    }
    complete_once(client, server, model, messages, options.api_key, false).await
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
    F: FnMut(&str),
{
    if options.stream {
        match complete_stream(client, server, model, messages, options.api_key, &mut on_delta)
            .await
        {
            Ok(reply) => return Ok(reply),
            Err(_) => {}
        }
    }
    complete_once(client, server, model, messages, options.api_key, false).await
}

async fn complete_once(
    client: &reqwest::Client,
    server: &str,
    model: &str,
    messages: &[Value],
    api_key: &str,
    stream: bool,
) -> Result<Reply, String> {
    let url = completions_url(server);
    let request = apply_auth(client.post(&url), api_key)
        .json(&request_body(model, messages, &tool_specs(), stream));
    let response = request
        .send()
        .await
        .map_err(|err| format!("Cannot reach {url}: {err}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| format!("Cannot read the response: {err}"))?;
    if !status.is_success() {
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("Server returned {status}: {snippet}"));
    }
    parse_reply(&text)
}

async fn complete_stream<F>(
    client: &reqwest::Client,
    server: &str,
    model: &str,
    messages: &[Value],
    api_key: &str,
    mut on_delta: F,
) -> Result<Reply, String>
where
    F: FnMut(&str),
{
    let url = completions_url(server);
    let request = apply_auth(client.post(&url), api_key)
        .header("Accept", "text/event-stream")
        .json(&request_body(model, messages, &tool_specs(), true));
    let response = request
        .send()
        .await
        .map_err(|err| format!("Cannot reach {url}: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        let text = response
            .text()
            .await
            .unwrap_or_default();
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("Server returned {status}: {snippet}"));
    }
    let mut assembler = StreamAssembler::default();
    let mut buffer = String::new();
    let mut saw_event = false;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| format!("Stream read failed: {err}"))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(newline) = buffer.find('\n') {
            let mut line = buffer[..newline].to_string();
            buffer.drain(..=newline);
            if line.ends_with('\r') {
                line.pop();
            }
            let Some(delta) = sse_data_delta(&line)? else {
                continue;
            };
            saw_event = true;
            if let Some(text) = delta.get("content").and_then(|value| value.as_str()) {
                if !text.is_empty() {
                    on_delta(text);
                }
            }
            assembler.push_delta(&delta);
        }
    }
    if !buffer.trim().is_empty() {
        if let Some(delta) = sse_data_delta(buffer.trim())? {
            saw_event = true;
            if let Some(text) = delta.get("content").and_then(|value| value.as_str()) {
                if !text.is_empty() {
                    on_delta(text);
                }
            }
            assembler.push_delta(&delta);
        }
    }
    if !saw_event {
        return Err("Stream produced no events".to_string());
    }
    assembler.into_reply()
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
        assert_eq!(body["max_tokens"], 8192);
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
    fn reasoning_is_the_reply_when_content_is_empty() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"","reasoning":"Use index.html"}}]}"#;
        let reply = parse_reply(raw).unwrap();
        assert!(reply.calls.is_empty());
        assert_eq!(reply.content, "Use index.html");
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
            },
            |text| pieces.push(text.to_string()),
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
            },
        )
        .await
        .unwrap();
        assert_eq!(reply.content, "fallback");
        server.join().unwrap();
    }
}
