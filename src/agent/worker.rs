//! HTTP loop. Tool calls come back to the core, which replies on the same channel.

use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures::FutureExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::provider::{self, CompleteOptions, Reply};
use super::{AgentCommand, AgentEvent, SYSTEM};

/// Same tool and the same arguments, this many times in a row, ends the turn.
const REPEAT: usize = 3;
/// Distinct tool calls still stop here so a turn cannot run forever.
const MAX_CALLS: usize = 32;

pub fn spawn() -> (
    mpsc::UnboundedSender<AgentCommand>,
    mpsc::UnboundedReceiver<AgentEvent>,
) {
    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        run(commands_rx, events_tx).await;
    });
    (commands_tx, events_rx)
}

async fn run(
    mut commands: mpsc::UnboundedReceiver<AgentCommand>,
    events: mpsc::UnboundedSender<AgentEvent>,
) {
    // Streaming a large create_file (full HTML) on a local 27B can take many
    // minutes. A short total timeout makes reqwest drop the socket; LM Studio
    // then logs "Client disconnected" / "Failed to generate a tool call" and
    // returns empty tool_calls with only reasoning_content.
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60 * 30))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            while let Some(command) = commands.recv().await {
                if let AgentCommand::Turn { id, .. } = command {
                    let _ = events.send(AgentEvent::Failed {
                        id,
                        text: format!("Cannot start HTTP: {err}"),
                    });
                }
            }
            return;
        }
    };
    while let Some(command) = commands.recv().await {
        match command {
            AgentCommand::Turn {
                id,
                server,
                model,
                api_key,
                stream,
                num_ctx,
                ollama_native,
                think,
                tuning,
                history,
                prompt,
            } => {
                // Catch panics so a bug in parsing/streaming does not kill the agent task
                // (and leave the TUI unable to start another turn).
                let result = AssertUnwindSafe(run_turn(
                    &client,
                    &events,
                    &mut commands,
                    id,
                    &server,
                    &model,
                    &api_key,
                    stream,
                    num_ctx,
                    ollama_native,
                    think,
                    &tuning,
                    history,
                    prompt,
                ))
                .catch_unwind()
                .await;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(text)) => {
                        if text.to_ascii_lowercase().contains("invalid tool call arguments") {
                            let _ = events.send(AgentEvent::ResetMemory { id });
                        }
                        let _ = events.send(AgentEvent::Failed { id, text });
                    }
                    Err(payload) => {
                        let detail = crate::utf8::panic_message(payload);
                        let _ = events.send(AgentEvent::Failed {
                            id,
                            text: format!("Agent worker panicked: {detail}"),
                        });
                    }
                }
                // Tell App what the server taught us, so it outlives this session.
                let (no_tools, no_think) = provider::detected_for(&server, &model, ollama_native);
                if no_tools || no_think {
                    let _ = events.send(AgentEvent::Detected {
                        key: provider::capability_key_for(&server, &model, ollama_native),
                        no_tools,
                        no_think,
                    });
                }
            }
            AgentCommand::Cancel { .. } | AgentCommand::ToolResult { .. } => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_turn(
    client: &reqwest::Client,
    events: &mpsc::UnboundedSender<AgentEvent>,
    commands: &mut mpsc::UnboundedReceiver<AgentCommand>,
    id: u64,
    server: &str,
    model: &str,
    api_key: &str,
    stream: bool,
    num_ctx: Option<u32>,
    ollama_native: bool,
    think: bool,
    tuning: &provider::Tuning,
    history: Vec<Value>,
    prompt: String,
) -> Result<(), String> {
    let mut messages = Vec::new();
    messages.push(json!({"role": "system", "content": SYSTEM}));
    messages.extend(history);
    messages.push(json!({"role": "user", "content": prompt}));

    let mut trail = ToolTrail::default();
    let mut nudged = false;
    let mut empty_thinking_nudged = false;
    loop {
        let Some(reply) = complete_or_cancel(
            client,
            commands,
            events,
            id,
            server,
            model,
            api_key,
            stream,
            num_ctx,
            ollama_native,
            think,
            tuning,
            &messages,
        )
        .await?
        else {
            return Ok(());
        };
        if reply.history_flattened {
            let _ = events.send(AgentEvent::ResetMemory { id });
        }
        if reply.calls.is_empty() {
            if !nudged && wants_file_action(&prompt) && looks_like_manual_steps(&reply.content) {
                nudged = true;
                messages.push(json!({"role": "assistant", "content": reply.content}));
                messages.push(json!({
                    "role": "user",
                    "content": "Use a tool call now (create_file or apply_edit). Do not give steps or paste file contents as instructions. Approval is handled by the app."
                }));
                continue;
            }
            // Thinking-only reply (empty content): re-prompt once instead of error spam.
            if reply.content.trim().is_empty()
                && !reply.thinking.trim().is_empty()
                && !empty_thinking_nudged
            {
                empty_thinking_nudged = true;
                messages.push(json!({
                    "role": "assistant",
                    "content": ""
                }));
                messages.push(json!({
                    "role": "user",
                    "content": "Your previous reply had reasoning but no tool call and no visible answer (the tool call may have been cut off). Call create_file or apply_edit now with complete JSON arguments. Do not put the file body only inside thinking."
                }));
                continue;
            }
            let text = if reply.content.trim().is_empty() {
                "The model sent an empty reply. Send it again. A thinking model can spend the whole reply before it writes anything, and a local server drops a tool call whose name is not in the tool list.".to_string()
            } else {
                reply.content
            };
            let _ = events.send(AgentEvent::Reply { id, text });
            return Ok(());
        }
        // Ensure wire-format arguments stay valid JSON strings for the next round-trip.
        let mut message = reply.message;
        if let Some(calls) = message.get("tool_calls").cloned() {
            if let Some(list) = calls.as_array() {
                let normalized: Vec<_> = list
                    .iter()
                    .enumerate()
                    .filter_map(|(index, call)| {
                        let name = call
                            .pointer("/function/name")
                            .and_then(|v| v.as_str())
                            .or_else(|| call.get("name").and_then(|v| v.as_str()))?;
                        let args = call
                            .pointer("/function/arguments")
                            .cloned()
                            .or_else(|| call.get("arguments").cloned())
                            .unwrap_or(serde_json::Value::String(String::new()));
                        let args = match args {
                            serde_json::Value::String(s) => {
                                provider::normalize_tool_arguments(&s)
                            }
                            other => provider::normalize_tool_arguments(&other.to_string()),
                        };
                        let call_id = call
                            .get("id")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("call-{index}"));
                        Some(serde_json::json!({
                            "id": call_id,
                            "type": "function",
                            "function": { "name": name, "arguments": args }
                        }))
                    })
                    .collect();
                message["tool_calls"] = serde_json::Value::Array(normalized);
            }
        }
        messages.push(message);
        for call in reply.calls {
            if let Some(text) = trail.note(&call.name, &call.arguments) {
                let _ = events.send(AgentEvent::Reply { id, text });
                return Ok(());
            }
            let _ = events.send(AgentEvent::Tool {
                id,
                call_id: call.id.clone(),
                name: call.name,
                arguments: call.arguments,
            });
            let Some(content) = wait_result(commands, id, &call.id).await else {
                return Ok(());
            };
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": content
            }));
        }
    }
}

#[derive(Default)]
struct ToolTrail {
    recent: std::collections::VecDeque<(String, String)>,
    count: usize,
}

impl ToolTrail {
    /// `Some` when this call must not run. A repeated call is the same name and arguments
    /// three times in a row. A later call past the cap stops the turn as well.
    fn note(&mut self, name: &str, arguments: &str) -> Option<String> {
        self.recent
            .push_back((name.to_string(), arguments.to_string()));
        if self.recent.len() > REPEAT {
            self.recent.pop_front();
        }
        let repeated = self.recent.len() == REPEAT
            && self
                .recent
                .iter()
                .all(|(seen_name, seen_args)| seen_name == name && seen_args == arguments);
        if repeated {
            return Some(format!("Stopped repeating {name} with the same arguments."));
        }
        self.count += 1;
        if self.count > MAX_CALLS {
            return Some(format!("Stopped after {MAX_CALLS} tool calls."));
        }
        None
    }
}

fn wants_file_action(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    const KEYS: &[&str] = &[
        "create",
        "write",
        "edit",
        "make a file",
        "new file",
        "สร้าง",
        "เขียน",
        "แก้",
        "ทำไฟล์",
        "สร้างไฟล์",
        "แก้ไฟล์",
    ];
    KEYS.iter().any(|k| lower.contains(k))
}

fn looks_like_manual_steps(content: &str) -> bool {
    let text = content.trim();
    if text.is_empty() {
        return false;
    }
    if text.contains("```") {
        return true;
    }
    let lower = text.to_lowercase();
    lower.contains("step 1")
        || lower.contains("1.")
        || lower.contains("you can")
        || lower.contains("you should")
        || lower.contains("เปิดไฟล์")
        || lower.contains("สร้างไฟล์เอง")
        || lower.contains("ทำตาม")
        || lower.contains("ดังนี้")
}

#[allow(clippy::too_many_arguments)]
async fn complete_or_cancel(
    client: &reqwest::Client,
    commands: &mut mpsc::UnboundedReceiver<AgentCommand>,
    events: &mpsc::UnboundedSender<AgentEvent>,
    id: u64,
    server: &str,
    model: &str,
    api_key: &str,
    stream: bool,
    num_ctx: Option<u32>,
    ollama_native: bool,
    think: bool,
    tuning: &provider::Tuning,
    messages: &[Value],
) -> Result<Option<Reply>, String> {
    let options = CompleteOptions {
        api_key,
        stream,
        num_ctx,
        ollama_native,
        think,
        tuning,
    };
    let events = events.clone();
    let request = async move {
        if stream {
            provider::complete_with_delta(
                client,
                server,
                model,
                messages,
                options,
                move |kind, text| {
                    match kind {
                        provider::DeltaKind::Content => {
                            let _ = events.send(AgentEvent::Delta {
                                id,
                                text: text.to_string(),
                            });
                        }
                        provider::DeltaKind::Thinking => {
                            let _ = events.send(AgentEvent::Thinking {
                                id,
                                text: text.to_string(),
                            });
                        }
                    }
                },
            )
            .await
        } else {
            provider::complete(client, server, model, messages, options).await
        }
    };
    tokio::pin!(request);
    loop {
        tokio::select! {
            biased;
            command = commands.recv() => match command {
                Some(AgentCommand::Cancel { id: cancel }) if cancel == id => return Ok(None),
                Some(AgentCommand::Cancel { .. }) => {}
                None => return Ok(None),
                Some(_) => return Err("The assistant turn was interrupted".to_string()),
            },
            result = &mut request => return result.map(Some),
        }
    }
}

async fn wait_result(
    commands: &mut mpsc::UnboundedReceiver<AgentCommand>,
    id: u64,
    call_id: &str,
) -> Option<String> {
    loop {
        match commands.recv().await? {
            AgentCommand::ToolResult {
                id: result_id,
                call_id: result_call,
                content,
            } if result_id == id && result_call == call_id => return Some(content),
            AgentCommand::Cancel { id: cancel } if cancel == id => return None,
            AgentCommand::Cancel { .. } | AgentCommand::ToolResult { .. } => {}
            AgentCommand::Turn { .. } => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ToolTrail;

    #[test]
    fn the_third_identical_call_stops_and_a_different_one_continues() {
        let mut trail = ToolTrail::default();
        assert_eq!(trail.note("read_file", "{\"path\":\"a\"}"), None);
        assert_eq!(trail.note("read_file", "{\"path\":\"a\"}"), None);
        let stop = trail.note("read_file", "{\"path\":\"a\"}").unwrap();
        assert!(stop.contains("read_file"), "{stop}");
        assert!(stop.contains("same arguments"), "{stop}");

        let mut trail = ToolTrail::default();
        assert_eq!(trail.note("read_file", "{\"path\":\"a\"}"), None);
        assert_eq!(trail.note("read_file", "{\"path\":\"a\"}"), None);
        assert_eq!(trail.note("read_file", "{\"path\":\"b\"}"), None);
        assert_eq!(trail.note("apply_edit", "{\"path\":\"a\"}"), None);
    }

    #[test]
    fn tool_calls_stop_after_the_cap() {
        let mut trail = ToolTrail::default();
        for index in 0..32 {
            assert_eq!(
                trail.note("read_file", &format!("{{\"path\":\"f{index}\"}}")),
                None
            );
        }
        let stop = trail.note("read_file", "{\"path\":\"last\"}").unwrap();
        assert!(stop.contains("32"), "{stop}");
    }
}
