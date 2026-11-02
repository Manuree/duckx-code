//! One non-streaming OpenAI chat completion. Ollama and LM Studio both speak this.

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

pub fn completions_url(server: &str) -> String {
    format!("{}/chat/completions", server.trim().trim_end_matches('/'))
}

pub fn request_body(model: &str, messages: &[Value], tools: &[Value]) -> Value {
    json!({
        "model": model,
        "messages": messages,
        "tools": tools,
        "temperature": 0.2
    })
}

pub fn parse_reply(text: &str) -> Result<Reply, String> {
    let value: Value = serde_json::from_str(text).map_err(|err| format!("Bad response: {err}"))?;
    let message = value
        .pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| "Response has no message".to_string())?;
    let content = match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other
            .as_array()
            .and_then(|parts| {
                parts
                    .iter()
                    .find_map(|part| part["text"].as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default(),
    };
    let mut calls = Vec::new();
    if let Some(list) = message["tool_calls"].as_array() {
        for (index, call) in list.iter().enumerate() {
            let name = call["function"]["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let arguments = match &call["function"]["arguments"] {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            let id = call["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("call-{index}"));
            calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
    }
    Ok(Reply {
        content,
        calls,
        message,
    })
}

pub async fn complete(
    client: &reqwest::Client,
    server: &str,
    model: &str,
    messages: &[Value],
) -> Result<Reply, String> {
    let url = completions_url(server);
    let response = client
        .post(&url)
        .json(&request_body(model, messages, &tool_specs()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_names_the_tools() {
        let body = request_body("qwen", &[], &tool_specs());
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
    }
}
