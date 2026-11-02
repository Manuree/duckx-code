//! Assistant turns. Tools run here, on the core, then the worker hears the result.

use serde::Deserialize;

use crate::action::AgentAction;
use crate::agent::{self, AgentCommand, AgentEvent, DiskTask, FileEdit, TranscriptLine};
use crate::jobs::Job;

use super::App;

const HISTORY: usize = 12;

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
                self.push_line(false, text);
                self.agent.busy = false;
                self.agent.status = "ready".to_string();
            }
            AgentEvent::Failed { id, text } => {
                if id != self.agent.turn || !self.agent.busy {
                    return;
                }
                self.push_line(false, text.clone());
                self.agent.busy = false;
                self.agent.status = clip(&text, 80);
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
        let history: Vec<(String, String)> = self
            .agent
            .transcript
            .iter()
            .rev()
            .take(HISTORY)
            .map(|line| {
                let role = if line.user { "user" } else { "assistant" };
                (role.to_string(), line.text.clone())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        self.agent.turn = self.agent.turn.saturating_add(1);
        let id = self.agent.turn;
        self.push_line(true, prompt.clone());
        let prompt = format!("{prompt}\n\neditor_state:\n{}", self.tool_state());
        self.agent.prompt.clear();
        self.agent.cursor = 0;
        self.agent.anchor = 0;
        self.agent.busy = true;
        self.agent.status = "waiting".to_string();
        let _ = tx.send(AgentCommand::Turn {
            id,
            server: self.ai.endpoint(),
            model: self.ai.model.trim().to_string(),
            history,
            prompt,
        });
    }

    fn cancel_agent(&mut self) {
        let id = self.agent.turn;
        self.agent.turn = self.agent.turn.saturating_add(1);
        self.agent.busy = false;
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
        let value = serde_json::from_str::<serde_json::Value>(arguments)
            .unwrap_or(serde_json::Value::Object(Default::default()));
        let reply = match name {
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
            let text = self
                .find_tab(tab_id)
                .map(|tab| tab.buffer.rope().to_string())
                .unwrap_or_default();
            return Some(agent::read_lines(&text, start, end).unwrap_or_else(|err| err));
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
        let rope = self
            .tab_with_path(&path)
            .and_then(|id| self.find_tab(id))
            .map(|tab| tab.buffer.rope().clone());
        let mut lines = Vec::new();
        for item in items.iter().take(30) {
            let line = rope.as_ref().map(|rope| {
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
