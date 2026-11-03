//! Assistant turns. Tools run here, on the core, then the worker hears the result.

use serde::Deserialize;

use crate::action::AgentAction;
use crate::agent::{self, AgentCommand, AgentEvent, DiskTask, FileEdit, PendingTool, TranscriptLine};
use crate::jobs::Job;

use super::App;

const HISTORY: usize = 12;
const MEMORY_BUDGET: usize = 6_000;
const MEMORY_TOOL_CLIP: usize = 400;

impl App {
    /// Next frame of the wait spinner. Idle turns stay on the current frame.
    pub fn advance_agent_spin(&mut self) {
        if self.agent.busy {
            self.agent.spin = self.agent.spin.wrapping_add(1);
        }
    }

    pub fn handle_agent(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Reply { id, text } => {
                if id != self.agent.turn || !self.agent.busy {
                    return;
                }
                self.agent.stream_draft = None;
                self.remember_assistant(&text);
                self.push_line(false, text);
                self.agent.busy = false;
                self.agent.status = "ready".to_string();
            }
            AgentEvent::Delta { id, text } => {
                if id != self.agent.turn || !self.agent.busy {
                    return;
                }
                match self.agent.stream_draft.as_mut() {
                    Some(draft) => draft.push_str(&text),
                    None => self.agent.stream_draft = Some(text),
                }
                self.agent.status = "streaming".to_string();
            }
            AgentEvent::Failed { id, text } => {
                if id != self.agent.turn || !self.agent.busy {
                    return;
                }
                self.agent.stream_draft = None;
                if text.to_ascii_lowercase().contains("invalid tool call arguments") {
                    self.agent.memory.clear();
                }
                self.push_line(false, text.clone());
                self.agent.busy = false;
                self.agent.status = clip(&text, 80);
            }
            AgentEvent::ResetMemory { id } => {
                if id != self.agent.turn {
                    return;
                }
                self.agent.memory.clear();
            }
            AgentEvent::Tool {
                id,
                call_id,
                name,
                arguments,
            } => {
                if id != self.agent.turn || !self.agent.busy {
                    return;
                }
                if let Some(draft) = self.agent.stream_draft.take() {
                    // Textual tool JSON may have been streamed before parse; do not show it as chat.
                    if !draft.trim().is_empty()
                        && !agent::content_looks_like_tool_prefix(&draft)
                    {
                        self.remember_assistant(&draft);
                        self.push_line(false, draft);
                    }
                }
                self.remember_tool_call(&call_id, &name, &arguments);
                self.agent.status = format!("running {name}");
                self.run_tool(id, call_id, &name, &arguments);
            }
        }
    }

    pub(super) fn toggle_agent(&mut self) {
        self.dismiss_lsp_popups();
        if self.agent.open && self.agent.focused {
            self.agent.open = false;
            self.agent.focused = false;
            return;
        }
        self.focus_agent();
    }

    /// Open the assistant and put the cursor in its prompt.
    pub(super) fn focus_agent(&mut self) {
        self.agent.open = true;
        self.agent.focused = true;
        self.explorer.blur();
        self.end_file_mode();
        if self.agent.status.is_empty() {
            self.agent.status = "ready".to_string();
        }
    }

    pub(super) fn handle_agent_input(&mut self, action: AgentAction) {
        if self.agent.pending.is_some() {
            match action {
                AgentAction::Submit => self.resolve_pending_tool(true),
                AgentAction::Close => self.resolve_pending_tool(false),
                AgentAction::Insert(text) => {
                    let key = text.trim();
                    if key.eq_ignore_ascii_case("y") {
                        self.resolve_pending_tool(true);
                    } else if key.eq_ignore_ascii_case("n") {
                        self.resolve_pending_tool(false);
                    }
                }
                AgentAction::Above => {
                    self.agent.reading = true;
                    self.agent.scroll = 0;
                }
                AgentAction::Below => {
                    self.agent.reading = false;
                }
                AgentAction::Line { down, .. } if self.agent.reading => {
                    if down {
                        self.agent.scroll = self.agent.scroll.saturating_sub(1);
                    } else {
                        self.agent.scroll = self.agent.scroll.saturating_add(1);
                    }
                }
                AgentAction::Jump { end, .. } if self.agent.reading => {
                    self.agent.scroll = if end { 0 } else { usize::MAX };
                }
                _ => {}
            }
            return;
        }
        match action {
            AgentAction::Insert(text) if !self.agent.reading => self.edit_prompt(&text),
            AgentAction::Backspace if !self.agent.reading => self.delete_prompt(false),
            AgentAction::Delete if !self.agent.reading => self.delete_prompt(true),
            AgentAction::Move { right, extend } if !self.agent.reading => {
                self.slide_prompt(right, extend);
            }
            AgentAction::Insert(_)
            | AgentAction::Backspace
            | AgentAction::Delete
            | AgentAction::Move { .. } => {}
            AgentAction::Jump { end, extend } => {
                if self.agent.reading {
                    self.agent.scroll = if end { 0 } else { usize::MAX };
                    return;
                }
                let at = if end {
                    self.agent.prompt.chars().count()
                } else {
                    0
                };
                self.place_prompt(at, extend);
            }
            AgentAction::Line { down, extend } => {
                if self.agent.reading {
                    if down {
                        self.agent.scroll = self.agent.scroll.saturating_sub(1);
                    } else {
                        self.agent.scroll = self.agent.scroll.saturating_add(1);
                    }
                } else {
                    self.line_prompt(down, extend);
                }
            }
            AgentAction::Above => {
                if self.agent.reading {
                    self.agent.scroll = self.agent.scroll.saturating_add(1);
                } else {
                    self.agent.reading = true;
                    self.agent.scroll = 0;
                }
            }
            AgentAction::Below => {
                self.agent.reading = false;
            }
            AgentAction::Close => {
                if self.agent.busy {
                    self.cancel_agent();
                }
                self.agent.focused = false;
            }
            AgentAction::Submit if !self.agent.reading => self.submit_agent(),
            AgentAction::Submit => {}
        }
    }

    fn submit_agent(&mut self) {
        let prompt = self.agent.prompt.trim().to_string();
        if prompt.is_empty() || self.agent.busy {
            return;
        }
        if !self.ai.enabled {
            self.message = Some("Assistant is off".to_string());
            self.agent.status = "off".to_string();
            return;
        }
        if self.ai.model.trim().is_empty() {
            self.message = Some("Set a model in Settings, under AI".to_string());
            return;
        }
        let Some(tx) = self.agent_tx.clone() else {
            self.message = Some("Assistant is not running".to_string());
            return;
        };
        let history = budget_memory(&self.agent.memory);
        self.agent.turn = self.agent.turn.saturating_add(1);
        let id = self.agent.turn;
        self.push_line(true, prompt.clone());
        let prompt = format!("{prompt}

editor_state:
{}", self.tool_state());
        self.remember_user(&prompt);
        self.agent.prompt.clear();
        self.agent.cursor = 0;
        self.agent.anchor = 0;
        self.agent.busy = true;
        self.agent.stream_draft = None;
        self.agent.status = "waiting".to_string();
        let _ = tx.send(AgentCommand::Turn {
            id,
            server: self.ai.endpoint(),
            model: self.ai.model.trim().to_string(),
            api_key: self.ai.api_key.clone(),
            stream: self.ai.streaming,
            history,
            prompt,
        });
    }

    fn cancel_agent(&mut self) {
        let id = self.agent.turn;
        self.agent.turn = self.agent.turn.saturating_add(1);
        self.agent.busy = false;
        self.agent.pending = None;
        self.agent.stream_draft = None;
        self.agent.status = "ready".to_string();
        if let Some(tx) = &self.agent_tx {
            let _ = tx.send(AgentCommand::Cancel { id });
        }
    }

    fn push_line(&mut self, user: bool, text: String) {
        let text = clip(&text, 4_000);
        self.agent.transcript.push(TranscriptLine { user, text });
        let extra = self.agent.transcript.len().saturating_sub(40);
        if extra > 0 {
            self.agent.transcript.drain(0..extra);
        }
    }

    pub(super) fn reply_tool(&mut self, turn: u64, call_id: String, content: String) {
        if turn != self.agent.turn || !self.agent.busy {
            return;
        }
        self.remember_tool_result(&call_id, &content);
        self.agent.status = "waiting".to_string();
        if let Some(tx) = &self.agent_tx {
            let _ = tx.send(AgentCommand::ToolResult {
                id: turn,
                call_id,
                content,
            });
        }
    }

    fn run_tool(&mut self, turn: u64, call_id: String, name: &str, arguments: &str) {
        let (name, arguments) = agent::normalize_call(name, arguments);
        if agent::needs_approval(&name) && !self.ai.auto_approve {
            self.ask_tool_approval(turn, call_id, name, arguments);
            return;
        }
        self.execute_tool(turn, call_id, &name, &arguments);
    }

    fn ask_tool_approval(&mut self, turn: u64, call_id: String, name: String, arguments: String) {
        let summary = self.tool_approval_summary(&name, &arguments);
        self.agent.status = format!("approve {name}? y/n");
        self.agent.reading = false;
        self.agent.pending = Some(PendingTool {
            turn,
            call_id,
            name,
            arguments,
            summary,
        });
        if !self.agent.open || !self.agent.focused {
            self.focus_agent();
        }
    }

    fn resolve_pending_tool(&mut self, allow: bool) {
        let Some(pending) = self.agent.pending.take() else {
            return;
        };
        if pending.turn != self.agent.turn || !self.agent.busy {
            return;
        }
        if allow {
            self.agent.status = format!("running {}", pending.name);
            self.execute_tool(pending.turn, pending.call_id, &pending.name, &pending.arguments);
        } else {
            self.reply_tool(
                pending.turn,
                pending.call_id,
                format!("User denied {}.", pending.name),
            );
        }
    }

    fn tool_approval_summary(&self, name: &str, arguments: &str) -> String {
        let value = serde_json::from_str::<serde_json::Value>(arguments)
            .unwrap_or(serde_json::Value::Object(Default::default()));
        let path = value
            .get("path")
            .and_then(|path| path.as_str())
            .unwrap_or("(no path)");
        match name {
            "apply_edit" => {
                if let Some(find) = value.get("find").and_then(|v| v.as_str()) {
                    let replace = value
                        .get("replace")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    format!(
                        "Allow apply_edit on {path}?
- find: {}
+ replace: {}
[y] Allow  [n] Deny",
                        clip(find, 120),
                        clip(replace, 120)
                    )
                } else {
                    let start = value
                        .get("start_line")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let end = value
                        .get("end_line")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(start);
                    let text = value.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    format!(
                        "Allow apply_edit on {path} lines {start}-{end}?
{}
[y] Allow  [n] Deny",
                        clip(text, 160)
                    )
                }
            }
            "create_file" => {
                let text = value.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let parent_note = match agent::within(&self.explorer.root, path) {
                    Ok(full) => match agent::create_parent_plan(&full) {
                        Ok(plan) if plan.missing_dirs > 0 => format!(
                            "
Will create {} new folder(s) under {}",
                            plan.missing_dirs,
                            plan.parent.display()
                        ),
                        Ok(_) => "
Parent folder exists".to_string(),
                        Err(err) => format!("
{err}"),
                    },
                    Err(err) => format!("
{err}"),
                };
                format!(
                    "Allow create_file {path}?{parent_note}
{}
[y] Allow  [n] Deny",
                    clip(text, 160)
                )
            }
                        "save" => format!("Allow save {path}?
[y] Allow  [n] Deny"),
            _ => format!("Allow {name} on {path}?
[y] Allow  [n] Deny"),
        }
    }

    fn execute_tool(&mut self, turn: u64, call_id: String, name: &str, arguments: &str) {
        let value = serde_json::from_str::<serde_json::Value>(arguments)
            .unwrap_or(serde_json::Value::Object(Default::default()));
        let reply = match name {
            "shell" => Some(
                "No shell. Create a file with create_file, or change one with apply_edit."
                    .to_string(),
            ),
            "read_file" => self.tool_read(&value, turn, call_id.clone()),
            "list_dir" => self.tool_list(&value, turn, call_id.clone()),
            "editor_state" => Some(self.tool_state()),
            "diagnostics" => Some(self.tool_diagnostics(&value)),
            "apply_edit" => self.tool_edit(&value, turn, call_id.clone()),
            "create_file" => self.tool_create(&value, turn, call_id.clone()),
            "open_file" => Some(self.tool_open(&value)),
            "save" => Some(self.tool_save(&value)),
            "goto" => Some(self.tool_goto(&value)),
            _ => Some(format!("Unknown tool {name}")),
        };
        if let Some(content) = reply {
            self.reply_tool(turn, call_id, content);
        }
    }

    /// `None` when a disk job will answer later.
    fn tool_read(
        &mut self,
        value: &serde_json::Value,
        turn: u64,
        call_id: String,
    ) -> Option<String> {
        let args: ReadArgs = serde_json::from_value(value.clone()).unwrap_or(ReadArgs {
            path: String::new(),
            start_line: 1,
            end_line: 0,
        });
        let path = match agent::within(&self.explorer.root, &args.path) {
            Ok(path) => path,
            Err(err) => return Some(err),
        };
        let start = args.start_line.max(1);
        let end = args.end_line;
        if let Some(tab_id) = self.tab_with_path(&path) {
            return Some(
                self.find_tab(tab_id)
                    .map(|tab| agent::read_lines_rope(tab.buffer.rope(), start, end))
                    .unwrap_or_else(|| Err("File is not open".to_string()))
                    .unwrap_or_else(|err| err),
            );
        }
        self.jobs.push(Job::AgentDisk {
            turn,
            call_id,
            task: DiskTask::Read { path, start, end },
        });
        None
    }

    fn tool_list(
        &mut self,
        value: &serde_json::Value,
        turn: u64,
        call_id: String,
    ) -> Option<String> {
        let path = value
            .get("path")
            .and_then(|path| path.as_str())
            .unwrap_or(".");
        let path = match agent::within(&self.explorer.root, path) {
            Ok(path) => path,
            Err(err) => return Some(err),
        };
        self.jobs.push(Job::AgentDisk {
            turn,
            call_id,
            task: DiskTask::List { path },
        });
        None
    }

    fn tool_state(&self) -> String {
        let tab = self.active_editor().tab();
        let (line, col) = tab.buffer.cursor_line_col();
        let path = tab
            .path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "[No Name]".to_string());
        let selection = clip(&tab.buffer.selected_text(), 500);
        let selection = if selection.is_empty() {
            "(none)".to_string()
        } else {
            selection
        };
        format!(
            "path: {path}\nline: {}\ncolumn: {}\nselection:\n{selection}",
            line + 1,
            col + 1
        )
    }

    fn tool_diagnostics(&self, value: &serde_json::Value) -> String {
        let path = if let Some(raw) = value.get("path").and_then(|path| path.as_str()) {
            match agent::within(&self.explorer.root, raw) {
                Ok(path) => path,
                Err(err) => return err,
            }
        } else if let Some(path) = self.path() {
            path.to_path_buf()
        } else {
            return "No file is focused".to_string();
        };
        let items = self.diagnostics.get(&path);
        if items.is_empty() {
            return "No diagnostics".to_string();
        }
        let mut lines = Vec::new();
        for item in items.iter().take(30) {
            let line = self
                .tab_with_path(&path)
                .and_then(|id| self.find_tab(id))
                .map(|tab| {
                    let rope = tab.buffer.rope();
                    let at = item.start.min(rope.len_bytes());
                    rope.byte_to_line(at) + 1
                });
            let where_ = line
                .map(|line| line.to_string())
                .unwrap_or_else(|| "?:?".to_string());
            lines.push(format!(
                "{where_} {} {}",
                item.severity.letter(),
                clip(&item.message, 160)
            ));
        }
        lines.join("\n")
    }

    fn tool_edit(
        &mut self,
        value: &serde_json::Value,
        turn: u64,
        call_id: String,
    ) -> Option<String> {
        let args: EditArgs = match serde_json::from_value(value.clone()) {
            Ok(args) => args,
            Err(err) => return Some(format!("Cannot read arguments: {err}")),
        };
        let path = match agent::within(&self.explorer.root, &args.path) {
            Ok(path) => path,
            Err(err) => return Some(err),
        };
        let edit = if let Some(find) = args.find.filter(|find| !find.is_empty()) {
            FileEdit::Snippet {
                find,
                replace: args.replace.unwrap_or_default(),
            }
        } else if args.start_line > 0 {
            FileEdit::Lines {
                start: args.start_line,
                end: args.end_line.max(args.start_line),
                text: args.text.unwrap_or_default(),
            }
        } else {
            return Some("apply_edit needs a line range or find".to_string());
        };
        if let Some(tab_id) = self.tab_with_path(&path) {
            let result = self
                .find_tab_mut(tab_id)
                .map(|tab| agent::apply_to_buffer(&mut tab.buffer, &edit))
                .unwrap_or_else(|| Err("File is not open".to_string()));
            return Some(match result {
                Ok(summary) => {
                    self.note_tab_change(tab_id);
                    summary
                }
                Err(err) => err,
            });
        }
        self.jobs.push(Job::AgentDisk {
            turn,
            call_id,
            task: DiskTask::Edit { path, edit },
        });
        None
    }

    fn tool_create(
        &mut self,
        value: &serde_json::Value,
        turn: u64,
        call_id: String,
    ) -> Option<String> {
        let Some(raw) = value.get("path").and_then(|path| path.as_str()) else {
            return Some("Path is empty".to_string());
        };
        let text = value
            .get("text")
            .and_then(|text| text.as_str())
            .unwrap_or("")
            .to_string();
        let path = match agent::within(&self.explorer.root, raw) {
            Ok(path) => path,
            Err(err) => return Some(err),
        };
        if self.tab_with_path(&path).is_some() {
            return Some("That file is already open. Use apply_edit.".to_string());
        }
        self.jobs.push(Job::AgentDisk {
            turn,
            call_id,
            task: DiskTask::Create { path, text },
        });
        None
    }

    fn tool_open(&mut self, value: &serde_json::Value) -> String {
        let Some(raw) = value.get("path").and_then(|path| path.as_str()) else {
            return "Path is empty".to_string();
        };
        match agent::within(&self.explorer.root, raw) {
            Ok(path) => {
                self.open_path(path.clone());
                format!("Opening {}", path.display())
            }
            Err(err) => err,
        }
    }

    fn tool_save(&mut self, value: &serde_json::Value) -> String {
        if let Some(raw) = value.get("path").and_then(|path| path.as_str()) {
            let path = match agent::within(&self.explorer.root, raw) {
                Ok(path) => path,
                Err(err) => return err,
            };
            let Some(tab_id) = self.tab_with_path(&path) else {
                return "That file is not open".to_string();
            };
            self.focus_tab(tab_id);
        }
        if self.path().is_none() {
            return "This file has no path".to_string();
        }
        match self.save_focused(None, None) {
            Ok(()) => "Saving the file".to_string(),
            Err(err) => err,
        }
    }

    fn tool_goto(&mut self, value: &serde_json::Value) -> String {
        let line = value
            .get("line")
            .and_then(|line| line.as_u64())
            .unwrap_or(1) as usize;
        let line = line.max(1);
        if let Some(raw) = value.get("path").and_then(|path| path.as_str()) {
            match agent::within(&self.explorer.root, raw) {
                Ok(path) => {
                    self.open_at(path, Some(line));
                    format!("Line {line}")
                }
                Err(err) => err,
            }
        } else {
            self.buffer_mut().goto_line(line - 1);
            format!("Line {line}")
        }
    }
}

impl App {
    fn edit_prompt(&mut self, text: &str) {
        let mut chars: Vec<char> = self.agent.prompt.chars().collect();
        let (from, to) = self.prompt_span(chars.len());
        chars.drain(from..to);
        let room = 4_000usize.saturating_sub(chars.len());
        let insert: Vec<char> = text.chars().take(room).collect();
        let next = from + insert.len();
        chars.splice(from..from, insert);
        self.store_prompt(chars, next);
    }

    fn delete_prompt(&mut self, forward: bool) {
        let mut chars: Vec<char> = self.agent.prompt.chars().collect();
        let (from, to) = self.prompt_span(chars.len());
        let next = if from != to {
            chars.drain(from..to);
            from
        } else if forward {
            let end = agent::next_boundary(&self.agent.prompt, from);
            if end > from {
                chars.drain(from..end);
            }
            from
        } else {
            let start = agent::prev_boundary(&self.agent.prompt, from);
            if start < from {
                chars.drain(start..from);
            }
            start
        };
        self.store_prompt(chars, next);
    }

    fn slide_prompt(&mut self, right: bool, extend: bool) {
        let len = self.agent.prompt.chars().count();
        let cursor = self.agent.cursor.min(len);
        let anchor = self.agent.anchor.min(len);
        if !extend && cursor != anchor {
            let edge = if right {
                cursor.max(anchor)
            } else {
                cursor.min(anchor)
            };
            self.agent.cursor = edge;
            self.agent.anchor = edge;
            return;
        }
        let next = if right {
            agent::next_boundary(&self.agent.prompt, cursor)
        } else {
            agent::prev_boundary(&self.agent.prompt, cursor)
        };
        self.place_prompt(next, extend);
    }

    fn line_prompt(&mut self, down: bool, extend: bool) {
        let width = self.screen_width.saturating_sub(2).saturating_sub(3).max(1) as usize;
        let rows = agent::wrap_rows(&self.agent.prompt, width);
        let len = self.agent.prompt.chars().count();
        let cursor = self.agent.cursor.min(len);
        let row = rows
            .iter()
            .rposition(|(start, end)| cursor >= *start && (cursor < *end || *start == *end))
            .unwrap_or(0);
        let next = if down {
            row.saturating_add(1)
        } else {
            row.saturating_sub(1)
        };
        if next >= rows.len() || next == row {
            return;
        }
        let column = agent::column_at(&self.agent.prompt, rows[row].0, cursor);
        let at = agent::index_at(&self.agent.prompt, rows[next], column);
        self.place_prompt(at, extend);
    }

    fn place_prompt(&mut self, at: usize, extend: bool) {
        let len = self.agent.prompt.chars().count();
        self.agent.cursor = at.min(len);
        if !extend {
            self.agent.anchor = self.agent.cursor;
        }
    }

    fn prompt_span(&self, len: usize) -> (usize, usize) {
        let cursor = self.agent.cursor.min(len);
        let anchor = self.agent.anchor.min(len);
        (cursor.min(anchor), cursor.max(anchor))
    }

    fn store_prompt(&mut self, chars: Vec<char>, cursor: usize) {
        self.agent.prompt = chars.into_iter().collect();
        self.agent.cursor = cursor;
        self.agent.anchor = cursor;
    }

    fn remember_user(&mut self, content: &str) {
        self.agent.memory.push(serde_json::json!({
            "role": "user",
            "content": clip(content, MEMORY_BUDGET)
        }));
        trim_memory(&mut self.agent.memory);
    }

    fn remember_assistant(&mut self, content: &str) {
        if content.trim().is_empty() {
            return;
        }
        self.agent.memory.push(serde_json::json!({
            "role": "assistant",
            "content": clip(content, MEMORY_TOOL_CLIP * 4)
        }));
        trim_memory(&mut self.agent.memory);
    }

    fn remember_tool_call(&mut self, call_id: &str, name: &str, arguments: &str) {
        let call_id = if call_id.trim().is_empty() {
            format!("call-{}", self.agent.memory.len())
        } else {
            call_id.to_string()
        };
        let args = compact_arguments_for_memory(arguments);
        self.agent.memory.push(serde_json::json!({
            "role": "assistant",
            "content": serde_json::Value::Null,
            "tool_calls": [{
                "id": call_id,
                "type": "function",
                "function": { "name": name, "arguments": args }
            }]
        }));
        trim_memory(&mut self.agent.memory);
    }

    fn remember_tool_result(&mut self, call_id: &str, content: &str) {
        self.agent.memory.push(serde_json::json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": summarize_tool_result(content)
        }));
        trim_memory(&mut self.agent.memory);
    }
}

fn budget_memory(memory: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut out: Vec<_> = memory.iter().map(sanitize_memory_message).collect();
    trim_memory(&mut out);
    let keep = HISTORY.saturating_mul(2).max(4);
    if out.len() > keep {
        // Drop whole messages from the front; never slice inside arguments JSON.
        out.drain(0..out.len() - keep);
    }
    // Keep tool_call / tool result pairs coherent when possible.
    reconcile_tool_pairs(&mut out);
    out
}

fn sanitize_memory_message(message: &serde_json::Value) -> serde_json::Value {
    let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("");
    if role != "assistant" || message.get("tool_calls").is_none() {
        return message.clone();
    }
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
            .unwrap_or(serde_json::Value::String(String::new()));
        let args = match args {
            serde_json::Value::String(s) => compact_arguments_for_memory(&s),
            other => compact_arguments_for_memory(&other.to_string()),
        };
        fixed.push(serde_json::json!({
            "id": id,
            "type": "function",
            "function": { "name": name, "arguments": args }
        }));
    }
    out["tool_calls"] = serde_json::Value::Array(fixed);
    out
}

fn reconcile_tool_pairs(memory: &mut Vec<serde_json::Value>) {
    // Drop leading orphan tool results (no preceding assistant tool_calls).
    while memory
        .first()
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        == Some("tool")
    {
        memory.remove(0);
    }
}

/// Keep arguments a valid JSON object string. Summarize huge string fields instead of
/// truncating mid-JSON (which Ollama rejects as invalid tool call arguments).
fn compact_arguments_for_memory(arguments: &str) -> String {
    let normalized = agent::normalize_tool_arguments(arguments);
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&normalized) else {
        return "{}".to_string();
    };
    if let Some(obj) = value.as_object_mut() {
        for (_key, val) in obj.iter_mut() {
            if let serde_json::Value::String(text) = val {
                if text.chars().count() > MEMORY_TOOL_CLIP {
                    let clipped: String = text.chars().take(MEMORY_TOOL_CLIP).collect();
                    *val = serde_json::Value::String(format!("{clipped}…"));
                }
            }
        }
    }
    value.to_string()
}

fn summarize_tool_result(content: &str) -> String {
    clip(content, MEMORY_TOOL_CLIP)
}

fn trim_memory(memory: &mut Vec<serde_json::Value>) {
    while memory_chars(memory) > MEMORY_BUDGET && !memory.is_empty() {
        memory.remove(0);
    }
    reconcile_tool_pairs(memory);
}

fn memory_chars(memory: &[serde_json::Value]) -> usize {
    memory
        .iter()
        .map(|value| value.to_string().chars().count())
        .sum()
}

fn clip(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

#[derive(Deserialize)]
struct ReadArgs {
    path: String,
    #[serde(default)]
    start_line: usize,
    #[serde(default)]
    end_line: usize,
}

#[derive(Deserialize)]
struct EditArgs {
    path: String,
    #[serde(default)]
    start_line: usize,
    #[serde(default)]
    end_line: usize,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    find: Option<String>,
    #[serde(default)]
    replace: Option<String>,
}
