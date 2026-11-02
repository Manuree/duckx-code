//! HTTP loop. Tool calls come back to the core, which replies on the same channel.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::provider::{self, Reply};
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
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
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
                history,
                prompt,
            } => {
                if let Err(text) = run_turn(
                    &client,
                    &events,
                    &mut commands,
                    id,
                    &server,
                    &model,
                    history,
                    prompt,
                )
                .await
                {
                    let _ = events.send(AgentEvent::Failed { id, text });
                }
            }
            AgentCommand::Cancel { .. } | AgentCommand::ToolResult { .. } => {}
        }
    }
}

async fn run_turn(
    client: &reqwest::Client,
    events: &mpsc::UnboundedSender<AgentEvent>,
    commands: &mut mpsc::UnboundedReceiver<AgentCommand>,
    id: u64,
    server: &str,
    model: &str,
    history: Vec<(String, String)>,
    prompt: String,
) -> Result<(), String> {
    let mut messages = Vec::new();
    messages.push(json!({"role": "system", "content": SYSTEM}));
    for (role, content) in history {
        messages.push(json!({"role": role, "content": content}));
    }
    messages.push(json!({"role": "user", "content": prompt}));

    let mut trail = ToolTrail::default();
    loop {
        let Some(reply) =
            complete_or_cancel(client, commands, id, server, model, &messages).await?
        else {
            return Ok(());
        };
        if reply.calls.is_empty() {
            let text = if reply.content.trim().is_empty() {
                "The model returned nothing.".to_string()
            } else {
                reply.content
            };
            let _ = events.send(AgentEvent::Reply { id, text });
            return Ok(());
        }
        messages.push(reply.message);
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

async fn complete_or_cancel(
    client: &reqwest::Client,
    commands: &mut mpsc::UnboundedReceiver<AgentCommand>,
    id: u64,
    server: &str,
    model: &str,
    messages: &[Value],
) -> Result<Option<Reply>, String> {
    let request = provider::complete(client, server, model, messages);
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
