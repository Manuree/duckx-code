//! Assistant turns. Tools run here, on the core, then the worker hears the result.

use serde::Deserialize;

use crate::action::AgentAction;
use crate::agent::{
    self, AgentCommand, AgentEvent, ApprovalChoice, DiskTask, FileEdit, PendingTool, TranscriptLine,
};
use crate::jobs::Job;

use super::App;

const HISTORY: usize = 8;
const MEMORY_BUDGET: usize = 2_500;
const MEMORY_TOOL_CLIP: usize = 180;
/// Conservative chars/token for Thai/CJK-heavy prompts.
const CHARS_PER_TOKEN: f64 = 3.5;
/// With auto compact on, memory holds this many characters before the oldest
/// messages fall off without a summary. Compaction normally folds them first.
const MEMORY_STORE: usize = 1_000_000;
/// Longest summary kept, in characters.
const SUMMARY_CLIP: usize = 12_000;
/// With auto compact on, a tool result is stored up to this many characters.
const TOOL_RESULT_STORE: usize = 6_000;
/// The newest tool results sent in full; older ones are cut to [`MEMORY_TOOL_CLIP`].
const KEEP_TOOL_RESULTS: usize = 4;
/// Entries `/changes` remembers.
const MAX_CHANGES: usize = 100;
/// How long an edit waits for the language server's diagnostics.
const DIAGNOSTICS_WAIT: std::time::Duration = std::time::Duration::from_secs(4);
/// How long a background load or a language server query may take.
const TOOL_WAIT: std::time::Duration = std::time::Duration::from_secs(15);
/// Lines of context returned around an edit.
const EDIT_CONTEXT: usize = 2;
/// Most lines an edit returns.
const EDIT_ECHO_LINES: usize = 30;

impl App {
    /// Next frame of the wait spinner. Idle turns stay on the current frame.
    pub fn advance_agent_spin(&mut self) {
        if self.agent.busy {
            self.agent.spin = self.agent.spin.wrapping_add(1);
            self.expire_agent_wait();
        }
    }

    pub fn handle_agent(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Reply { id, text } => {
                if id != self.agent.turn || !self.agent.busy {
                    return;
                }
                self.agent.stream_draft = None;
                self.agent.thinking_draft = None;
                self.remember_assistant(&text);
                if !matches!(self.agent.context_used, Some((_, true))) {
                    self.agent.context_used = Some((self.estimate_context(""), false));
                }
                self.push_line(false, text.clone());
                self.note_folded_reply(&text, false);
                self.remember_working_target();
                self.agent.busy = false;
                self.agent.busy_started = None;
                self.agent.status = "ready".to_string();
            }
            AgentEvent::Delta { id, text } => {
                if id != self.agent.turn || !self.agent.busy {
                    return;
                }
                // Visible answer started — drop thinking snippet from the title.
                self.agent.thinking_draft = None;
                match self.agent.stream_draft.as_mut() {
                    Some(draft) => draft.push_str(&text),
                    None => self.agent.stream_draft = Some(text),
                }
                self.agent.status = "streaming".to_string();
            }
            AgentEvent::Thinking { id, text } => {
                if id != self.agent.turn || !self.agent.busy {
                    return;
                }
                match self.agent.thinking_draft.as_mut() {
                    Some(draft) => draft.push_str(&text),
                    None => self.agent.thinking_draft = Some(text),
                }
                self.agent.status = "thinking".to_string();
            }
            AgentEvent::Failed { id, text } => {
                if id != self.agent.turn || !self.agent.busy {
                    return;
                }
                self.agent.stream_draft = None;
                self.agent.thinking_draft = None;
                if text
                    .to_ascii_lowercase()
                    .contains("invalid tool call arguments")
                {
                    self.agent.memory.clear();
                }
                self.push_line(false, text.clone());
                self.note_folded_reply(&text, true);
                self.agent.busy = false;
                self.agent.busy_started = None;
                self.agent.status = clip(&text, 80);
            }
            AgentEvent::Detected {
                key,
                no_tools,
                no_think,
            } => {
                if self.ai.detected.learn(&key, no_tools, no_think) {
                    self.queue_settings_save();
                }
            }
            AgentEvent::ResetMemory { id } => {
                if id != self.agent.turn {
                    return;
                }
                self.agent.memory.clear();
            }
            AgentEvent::Compacted {
                id,
                summary,
                dropped,
            } => {
                if id != self.agent.turn {
                    return;
                }
                match summary {
                    Ok(text) => {
                        let text = clip(text.trim(), SUMMARY_CLIP);
                        let tokens = estimate_tokens(&text);
                        self.agent.summary = Some(text);
                        self.push_line(
                            false,
                            format!(
                                "Compacted {dropped} older messages into a summary (~{tokens} tokens)."
                            ),
                        );
                    }
                    Err(err) => self.push_line(
                        false,
                        format!(
                            "Auto compact could not summarize ({}); dropped {dropped} older messages instead.",
                            clip(&err, 120)
                        ),
                    ),
                }
                if self.agent.compacting.take().is_some() {
                    self.agent.busy = false;
                    self.agent.busy_started = None;
                    self.agent.status = "ready".to_string();
                }
                self.agent.context_used = Some((self.estimate_context(""), false));
            }
            AgentEvent::Usage { id, tokens } => {
                if id != self.agent.turn {
                    return;
                }
                self.agent.context_used = Some((tokens, true));
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
                    if !draft.trim().is_empty() && !agent::content_looks_like_tool_prefix(&draft) {
                        self.remember_assistant(&draft);
                        self.push_line(false, draft);
                    }
                }
                self.agent.thinking_draft = None;
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
        if self.snackbar.as_ref().is_some_and(|bar| bar.assistant) {
            self.snackbar = None;
        }
    }

    /// A reply that lands while the panel is hidden. The user is looking at the
    /// editor, so the answer is also shown above the status bar.
    fn note_folded_reply(&mut self, text: &str, error: bool) {
        if self.focus() == crate::action::Focus::Agent {
            return;
        }
        let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let line = clip(&line, 80);
        self.snackbar = Some(super::Snackbar {
            text: if line.is_empty() {
                "The assistant replied.".to_string()
            } else {
                line
            },
            kind: if error {
                super::NoticeKind::Error
            } else {
                super::NoticeKind::Info
            },
            title: " Assistant · F6 ",
            assistant: true,
            until: std::time::Instant::now() + std::time::Duration::from_secs(6),
        });
    }

    /// When the notice should leave the screen. The main loop sleeps until then.
    pub fn snackbar_deadline(&self) -> Option<std::time::Instant> {
        self.snackbar.as_ref().map(|bar| bar.until)
    }

    /// Drops the notice once its time is up.
    pub fn expire_snackbar(&mut self) {
        if self
            .snackbar
            .as_ref()
            .is_some_and(|bar| bar.until <= std::time::Instant::now())
        {
            self.snackbar = None;
        }
    }

    pub(super) fn handle_agent_input(&mut self, action: AgentAction) {
        if self.agent.pending.is_some() {
            match action {
                AgentAction::NewChat => {
                    self.clear_agent_chat();
                    return;
                }
                AgentAction::Submit => {
                    let choice = self
                        .agent
                        .pending
                        .as_ref()
                        .map(|pending| pending.choice)
                        .unwrap_or_default();
                    self.resolve_pending_tool(choice);
                }
                AgentAction::Close => self.resolve_pending_tool(ApprovalChoice::No),
                AgentAction::Insert(text) => {
                    let key = text.trim();
                    if key.eq_ignore_ascii_case("y") {
                        self.resolve_pending_tool(ApprovalChoice::Yes);
                    } else if key.eq_ignore_ascii_case("a") {
                        // Session approval only where offered; otherwise ignore.
                        if self
                            .agent
                            .pending
                            .as_ref()
                            .is_some_and(|p| p.offers_session())
                        {
                            self.resolve_pending_tool(ApprovalChoice::Session);
                        }
                    } else if key.eq_ignore_ascii_case("n") {
                        self.resolve_pending_tool(ApprovalChoice::No);
                    } else if key == "h" || key == "H" {
                        self.step_pending_choice(false);
                    } else if key == "l" || key == "L" {
                        self.step_pending_choice(true);
                    }
                }
                AgentAction::Move { right, .. } => {
                    self.step_pending_choice(right);
                }
                AgentAction::Jump { end, .. } => {
                    if self.agent.reading {
                        self.agent.scroll = if end { 0 } else { usize::MAX };
                    } else if let Some(pending) = self.agent.pending.as_mut() {
                        let list = pending.choices();
                        pending.choice = if end { list[list.len() - 1] } else { list[0] };
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
                AgentAction::Complete => {}
                _ => {}
            }
            return;
        }

        // Popup navigation takes Up/Down/Tab/Enter/Esc before normal prompt keys.
        if self.agent.popup.is_some() {
            match action {
                AgentAction::Line { down, .. } if !self.agent.reading => {
                    self.nudge_agent_popup(down);
                    return;
                }
                AgentAction::Complete | AgentAction::Submit if !self.agent.reading => {
                    self.accept_agent_popup(matches!(action, AgentAction::Submit));
                    return;
                }
                AgentAction::Close => {
                    self.agent.popup = None;
                    return;
                }
                _ => {}
            }
        }

        match action {
            AgentAction::Insert(text) if !self.agent.reading => {
                self.edit_prompt(&text);
                self.refresh_agent_popup();
            }
            AgentAction::Backspace if !self.agent.reading => {
                self.delete_prompt(false);
                self.refresh_agent_popup();
            }
            AgentAction::Delete if !self.agent.reading => {
                self.delete_prompt(true);
                self.refresh_agent_popup();
            }
            AgentAction::Move { right, extend } if !self.agent.reading => {
                self.slide_prompt(right, extend);
                self.refresh_agent_popup();
            }
            AgentAction::Word { right, extend } if !self.agent.reading => {
                let len = self.agent.prompt.chars().count();
                let cursor = self.agent.cursor.min(len);
                let at = word_stop(&self.agent.prompt, cursor, right);
                self.place_prompt(at, extend);
                self.refresh_agent_popup();
            }
            AgentAction::Insert(_)
            | AgentAction::Backspace
            | AgentAction::Delete
            | AgentAction::Move { .. }
            | AgentAction::Word { .. } => {}
            AgentAction::Jump { end, extend } => {
                if self.agent.reading {
                    self.agent.scroll = if end { 0 } else { usize::MAX };
                    return;
                }
                let width = self.screen_width.saturating_sub(2).saturating_sub(3).max(1) as usize;
                let rows = agent::wrap_rows(&self.agent.prompt, width);
                let len = self.agent.prompt.chars().count();
                let cursor = self.agent.cursor.min(len);
                let row = prompt_row_for(&rows, cursor, len);
                let at = if end { rows[row].1 } else { rows[row].0 };
                self.place_prompt(at, extend);
                self.refresh_agent_popup();
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
                    self.refresh_agent_popup();
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
                if self.agent.popup.take().is_some() {
                    return;
                }
                // Esc only leaves the panel. A running turn keeps going.
                self.agent.focused = false;
            }
            AgentAction::NewChat => self.clear_agent_chat(),
            AgentAction::Complete if !self.agent.reading => {
                if self.agent.popup.is_some() {
                    self.accept_agent_popup(false);
                } else {
                    // Same as former Tab → command bar.
                    self.open_command_bar_from_agent();
                }
            }
            AgentAction::Complete => {}
            AgentAction::Submit if !self.agent.reading => self.submit_agent(),
            AgentAction::Submit => {}
        }
    }

    /// Drop transcript + tool memory so the next turn starts fresh.
    /// A running turn is left alone: only `/stop` ends it.
    pub(super) fn clear_agent_chat(&mut self) {
        if self.agent.busy {
            self.push_line(
                false,
                "The assistant is still working. /stop to stop it.".into(),
            );
            return;
        }
        self.agent.pending = None;
        self.agent.wait = None;
        self.agent.reads.clear();
        self.agent.changes.clear();
        self.agent.session_allow.clear();
        self.agent.memory.clear();
        self.agent.summary = None;
        self.agent.context_used = None;
        self.agent.transcript.clear();
        self.agent.stream_draft = None;
        self.agent.thinking_draft = None;
        self.agent.popup = None;
        self.agent.scroll = 0;
        self.agent.reading = false;
        self.agent.status = "new chat".to_string();
        self.push_line(false, "New chat — prior tool memory cleared.".to_string());
        self.agent.status = "ready".to_string();
        self.focus_agent();
    }

    pub(crate) fn submit_agent(&mut self) {
        let prompt = self.agent.prompt.to_string();
        if prompt.trim().is_empty() {
            return;
        }
        // A running turn ignores a new prompt. /stop is the one command that
        // still runs, and it is the only way to end that turn.
        if self.agent.busy {
            if let Some(parsed) = agent::parse_slash(&prompt) {
                if parsed.name == "stop" {
                    self.agent.popup = None;
                    self.run_slash_command(parsed);
                }
            }
            return;
        }
        self.agent.popup = None;

        // Slash commands: only when `/` is the very first character.
        if let Some(parsed) = agent::parse_slash(&prompt) {
            self.run_slash_command(parsed);
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

        let visible = prompt.clone();
        let attached = self.attach_mentions(&prompt);
        let ctx = self.ai.resolved_context_size();
        self.agent.turn = self.agent.turn.saturating_add(1);
        let id = self.agent.turn;
        self.agent.last_prompt = Some(visible.clone());
        self.push_line(true, visible);
        let mut model_prompt = if attached.is_empty() {
            prompt
        } else {
            format!("{prompt}\n\n{attached}")
        };
        model_prompt = format!("{model_prompt}\n\neditor_state:\n{}", self.tool_state());
        self.remember_user(&model_prompt);
        let (history, compact) = if self.ai.auto_compact {
            let (history, split) = compact_plan(
                &self.agent.memory,
                ctx,
                self.ai.resolved_compact_buffer(),
                self.ai.resolved_compact_keep(),
                &model_prompt,
                self.agent.summary.as_deref(),
            );
            let older = (split > 0).then(|| {
                self.agent
                    .memory
                    .drain(..split)
                    .map(|message| sanitize_memory_message(&message))
                    .collect::<Vec<_>>()
            });
            (history, older)
        } else {
            (budget_memory(&self.agent.memory, ctx, &model_prompt), None)
        };
        let compacting = compact.is_some();
        self.agent.prompt.clear();
        self.agent.cursor = 0;
        self.agent.anchor = 0;
        self.agent.busy = true;
        self.agent.busy_started = Some(std::time::Instant::now());
        self.agent.stream_draft = None;
        self.agent.thinking_draft = None;
        self.agent.status = if compacting {
            "compacting".to_string()
        } else {
            "waiting".to_string()
        };
        self.agent.context_used = Some((
            estimate_request(self.agent.summary.as_deref(), &history, &model_prompt),
            false,
        ));
        let ollama_native = self.ai.provider == crate::settings::AiProvider::Ollama;
        let (api_key, _) = self.ai.resolved_api_key();
        self.agent_target = Some((
            self.ai.provider,
            self.ai.endpoint(),
            self.ai.model.trim().to_string(),
        ));
        let mode = self.agent.mode;
        let mut tuning = ai_tuning(&self.ai);
        tuning.read_only = mode.read_only();
        let workspace =
            (!self.explorer.root.as_os_str().is_empty()).then(|| self.explorer.root.clone());
        let _ = tx.send(AgentCommand::Turn {
            id,
            server: self.ai.endpoint(),
            model: self.ai.model.trim().to_string(),
            api_key,
            stream: self.ai.streaming,
            num_ctx: Some(ctx),
            ollama_native,
            think: self.ai.think,
            tuning,
            history,
            prompt: model_prompt,
            summary: self.agent.summary.clone(),
            compact,
            mode,
            workspace,
        });
    }

    /// `/plan`, `/ask`, `/agent`. Naming the mode already on returns to Agent.
    fn set_agent_mode(&mut self, wanted: agent::AgentMode) {
        let next = if self.agent.mode == wanted {
            agent::AgentMode::Agent
        } else {
            wanted
        };
        self.agent.mode = next;
        let text = match next {
            agent::AgentMode::Agent => {
                "Agent mode: the assistant can edit files again.".to_string()
            }
            agent::AgentMode::Plan => {
                "Plan mode: the assistant reads code and writes a plan. It cannot edit. /agent to carry it out."
                    .to_string()
            }
            agent::AgentMode::Ask => {
                "Ask mode: the assistant reads code and answers. It cannot edit. /agent to edit again."
                    .to_string()
            }
        };
        self.push_line(false, text);
    }

    /// `/changes` lists this chat's edits. `/changes 2` opens the second one.
    fn slash_changes(&mut self, args: &str) {
        if self.agent.changes.is_empty() {
            self.push_line(
                false,
                "The assistant has not changed any file in this chat.".into(),
            );
            return;
        }
        if let Ok(number) = args.trim().parse::<usize>() {
            let Some(change) = number
                .checked_sub(1)
                .and_then(|index| self.agent.changes.get(index))
                .cloned()
            else {
                self.push_line(
                    false,
                    format!(
                        "There are {} changes. Try /changes 1.",
                        self.agent.changes.len()
                    ),
                );
                return;
            };
            self.open_at(change.path.clone(), Some(change.line));
            self.agent.focused = false;
            return;
        }
        let mut lines = vec!["Changes in this chat (Ctrl+Z in a file undoes one):".to_string()];
        for (index, change) in self.agent.changes.iter().enumerate() {
            let path = self.agent_relative(&change.path);
            let unsaved = self
                .tab_with_path(&change.path)
                .and_then(|id| self.find_tab(id))
                .is_some_and(|tab| tab.dirty);
            let mark = if unsaved { " · unsaved" } else { "" };
            lines.push(format!(
                "{}. {path}:{} {}{mark}",
                index + 1,
                change.line,
                change.summary
            ));
        }
        lines.push("/changes 2 opens the second one.".to_string());
        self.push_line(false, lines.join("\n"));
    }

    fn agent_relative(&self, path: &std::path::Path) -> String {
        super::relative_to(&self.explorer.root, path)
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| path.display().to_string())
    }

    pub(super) fn agent_opened_new_file(&mut self, path: &std::path::Path) {
        self.note_agent_change(path, 1, "Created the file");
    }

    fn note_agent_change(&mut self, path: &std::path::Path, line: usize, summary: &str) {
        self.agent.changes.push(agent::AgentChange {
            path: path.to_path_buf(),
            line: line.max(1),
            summary: summary.to_string(),
        });
        if self.agent.changes.len() > MAX_CHANGES {
            self.agent.changes.remove(0);
        }
    }

    /// `(used, limit)` for the `ctx` counter in the assistant panel's bottom border.
    /// `limit` is `None` when unknown.
    pub fn context_gauge(&self) -> (u32, Option<u32>) {
        let used = match self.agent.context_used {
            Some((tokens, _)) => tokens,
            None => self.estimate_context(&self.agent.prompt.to_string()),
        };
        let limit = self
            .ai
            .context_size_known()
            .then(|| self.ai.resolved_context_size());
        (used, limit)
    }

    /// Context tokens the next request would use, by the same estimate auto compact uses.
    pub(crate) fn estimate_context(&self, prompt: &str) -> u32 {
        let mut history: Vec<_> = self
            .agent
            .memory
            .iter()
            .map(sanitize_memory_message)
            .collect();
        reconcile_tool_pairs(&mut history);
        clear_old_tool_results(&mut history, KEEP_TOOL_RESULTS);
        estimate_request(self.agent.summary.as_deref(), &history, prompt)
    }

    /// `/compact`: fold everything before the latest turn into a summary now.
    fn compact_now(&mut self) {
        let before = self.agent.memory.len();
        let Some(split) = self
            .agent
            .memory
            .iter()
            .rposition(|message| message.get("role").and_then(|r| r.as_str()) == Some("user"))
            .filter(|split| *split > 0)
        else {
            self.push_line(false, "Nothing to compact yet.".to_string());
            return;
        };
        let tx = self.agent_tx.clone();
        let Some(tx) = tx.filter(|_| self.ai.enabled && !self.ai.model.trim().is_empty()) else {
            // No model to summarize with: plain truncation, as before.
            self.agent.memory.drain(..split);
            reconcile_tool_pairs(&mut self.agent.memory);
            self.push_line(
                false,
                format!(
                    "Compacted history ({} → {} messages, no model to summarize).",
                    before,
                    self.agent.memory.len()
                ),
            );
            self.agent.context_used = Some((self.estimate_context(""), false));
            return;
        };
        let older: Vec<_> = self
            .agent
            .memory
            .drain(..split)
            .map(|message| sanitize_memory_message(&message))
            .collect();
        reconcile_tool_pairs(&mut self.agent.memory);
        self.agent.turn = self.agent.turn.saturating_add(1);
        self.agent.compacting = Some(older.len());
        self.agent.busy = true;
        self.agent.busy_started = Some(std::time::Instant::now());
        self.agent.status = "compacting".to_string();
        let (api_key, _) = self.ai.resolved_api_key();
        let _ = tx.send(AgentCommand::Compact {
            id: self.agent.turn,
            server: self.ai.endpoint(),
            model: self.ai.model.trim().to_string(),
            api_key,
            num_ctx: Some(self.ai.resolved_context_size()),
            ollama_native: self.ai.provider == crate::settings::AiProvider::Ollama,
            tuning: ai_tuning(&self.ai),
            summary: self.agent.summary.clone(),
            older,
        });
    }

    fn cancel_agent(&mut self) {
        let id = self.agent.turn;
        self.agent.turn = self.agent.turn.saturating_add(1);
        self.agent.busy = false;
        self.agent.busy_started = None;
        self.agent.pending = None;
        self.agent.wait = None;
        self.agent.compacting = None;
        self.agent.stream_draft = None;
        self.agent.thinking_draft = None;
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
        let mode = self.agent.mode;
        if mode.read_only() && !agent::is_read_only_tool(&name) && name != "shell" {
            self.reply_tool(
                turn,
                call_id,
                format!(
                    "{name} is not available in {} mode: files cannot change. Reply with text instead.",
                    mode.label()
                ),
            );
            return;
        }
        if agent::needs_approval(&name)
            && !self.ai.auto_approve
            && !(agent::session_approvable(&name) && self.agent.session_allow.contains(&name))
        {
            self.ask_tool_approval(turn, call_id, name, arguments);
            return;
        }
        self.execute_tool(turn, call_id, &name, &arguments);
    }

    fn ask_tool_approval(&mut self, turn: u64, call_id: String, name: String, arguments: String) {
        let summary = self.tool_approval_summary(&name, &arguments);
        self.agent.status = format!("approve {name} · ←/→ Enter");
        self.agent.reading = false;
        self.agent.pending = Some(PendingTool {
            turn,
            call_id,
            name,
            arguments,
            summary,
            choice: ApprovalChoice::Yes,
        });
        // Someone typing in the editor keeps the keys: a `y`, `n`, or `a` they type must
        // not answer the card. The status line tells them where it waits.
        if !self.agent.focused
            && let Some(pending) = &self.agent.pending
        {
            self.message = Some(format!(
                "The assistant asks to run {}. F6 to answer",
                pending.name
            ));
        }
    }

    fn step_pending_choice(&mut self, right: bool) {
        if let Some(pending) = self.agent.pending.as_mut() {
            pending.step(right);
        }
    }

    /// Tab cycles Yes / Yes, this session / No while a tool waits for approval.
    pub(super) fn toggle_pending_allow(&mut self) {
        if let Some(pending) = self.agent.pending.as_mut() {
            pending.cycle();
        }
    }

    fn resolve_pending_tool(&mut self, choice: ApprovalChoice) {
        let Some(pending) = self.agent.pending.take() else {
            return;
        };
        if pending.turn != self.agent.turn || !self.agent.busy {
            return;
        }
        let choice = if choice == ApprovalChoice::Session && !pending.offers_session() {
            ApprovalChoice::Yes
        } else {
            choice
        };
        if choice == ApprovalChoice::Session {
            self.agent.session_allow.insert(pending.name.clone());
        }
        if choice != ApprovalChoice::No {
            self.agent.status = format!("running {}", pending.name);
            self.execute_tool(
                pending.turn,
                pending.call_id,
                &pending.name,
                &pending.arguments,
            );
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
                    let replace = value.get("replace").and_then(|v| v.as_str()).unwrap_or("");
                    let mut lines = vec![format!("apply_edit · {path}")];
                    lines.extend(diff_preview_lines(find, replace));
                    lines.join("\n")
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
                    let mut lines = vec![format!("apply_edit · {path} · lines {start}-{end}")];
                    lines.extend(plus_preview_lines(text, 6));
                    lines.join("\n")
                }
            }
            "create_file" => {
                let text = value
                    .get("text")
                    .or_else(|| value.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let parent_note = match agent::within(&self.explorer.root, path) {
                    Ok(full) => match agent::create_parent_plan(&full) {
                        Ok(plan) if plan.missing_dirs > 0 => format!(
                            "folders: +{} under {}",
                            plan.missing_dirs,
                            plan.parent.display()
                        ),
                        Ok(_) => "folders: parent exists".to_string(),
                        Err(err) => err,
                    },
                    Err(err) => err,
                };
                let mut lines = vec![format!("create_file · {path}"), parent_note];
                lines.extend(plus_preview_lines(text, 4));
                lines.join("\n")
            }
            "save" => format!("save · {path}"),
            _ => format!("{name} · {path}"),
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
            "search" => self.tool_search(&value, turn, call_id.clone()),
            "lsp" => self.tool_lsp(&value, turn, call_id.clone()),
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
            let Some((result, revision)) = self.find_tab(tab_id).map(|tab| {
                (
                    agent::read_lines_rope(tab.buffer.rope(), start, end),
                    tab.buffer.revision(),
                )
            }) else {
                return Some("File is not open".to_string());
            };
            return Some(match result {
                Ok(lines) => {
                    self.agent
                        .reads
                        .insert(path, agent::ReadMark::Buffer(revision));
                    lines
                }
                Err(err) => err,
            });
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
        let selection = clip(&tab.buffer.highlighted_text(), 500);
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
        let lines = self.diagnostic_lines(&path, false);
        if lines.is_empty() {
            return "No diagnostics".to_string();
        }
        lines.join("\n")
    }

    /// `line severity message` for up to 30 diagnostics. `serious` keeps only
    /// errors and warnings.
    fn diagnostic_lines(&self, path: &std::path::Path, serious: bool) -> Vec<String> {
        let items = self.diagnostics.get(path);
        let rope = self
            .tab_with_path(path)
            .and_then(|id| self.find_tab(id))
            .map(|tab| tab.buffer.rope());
        items
            .iter()
            .filter(|item| {
                !serious
                    || matches!(
                        item.severity,
                        crate::lsp::Severity::Error | crate::lsp::Severity::Warning
                    )
            })
            .take(30)
            .map(|item| {
                let where_ = rope
                    .map(|rope| {
                        (rope.byte_to_line(item.start.min(rope.len_bytes())) + 1).to_string()
                    })
                    .unwrap_or_else(|| "?:?".to_string());
                format!(
                    "{where_} {} {}",
                    item.severity.letter(),
                    clip(&item.message, 160)
                )
            })
            .collect()
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
                all: args.replace_all,
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
            return self.edit_open_tab(turn, call_id, tab_id, &path, &edit);
        }
        if !path.is_file() {
            return Some(format!(
                "{} does not exist. Use create_file for a new file.",
                self.agent_relative(&path)
            ));
        }
        if let Err(err) = agent::check_file_size(&path) {
            return Some(err);
        }
        // A closed file opens in a background tab, so the edit is an undo step and
        // stays unsaved like any other edit. The visible tab does not change.
        let tab = self.open_in_background(path.clone());
        self.agent.wait = Some(agent::ToolWait {
            turn,
            call_id,
            deadline: std::time::Instant::now() + TOOL_WAIT,
            kind: agent::WaitKind::Load {
                tab,
                path,
                then: agent::AfterLoad::Edit(edit),
            },
        });
        None
    }

    /// Applies `edit` to an open tab. `None` when the reply waits for diagnostics.
    fn edit_open_tab(
        &mut self,
        turn: u64,
        call_id: String,
        tab_id: crate::layout::TabId,
        path: &std::path::Path,
        edit: &FileEdit,
    ) -> Option<String> {
        if let FileEdit::Lines { .. } = edit {
            let Some((revision, fingerprint)) = self
                .find_tab(tab_id)
                .map(|tab| (tab.buffer.revision(), tab.buffer.fingerprint()))
            else {
                return Some("File is not open".to_string());
            };
            match self.agent.reads.get(path) {
                Some(agent::ReadMark::Buffer(seen)) if *seen == revision => {}
                Some(agent::ReadMark::Disk(seen)) if *seen == fingerprint => {}
                Some(_) => {
                    return Some(format!(
                        "{} changed since you read it (the user may be typing). read_file those lines again, or use find/replace.",
                        self.agent_relative(path)
                    ));
                }
                None => {
                    return Some(format!(
                        "read_file {} before a line-range edit, or use find/replace.",
                        self.agent_relative(path)
                    ));
                }
            }
        }
        let result = match self.find_tab_mut(tab_id) {
            Some(tab) => {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    agent::apply_to_buffer(&mut tab.buffer, edit)
                })) {
                    Ok(inner) => inner,
                    Err(payload) => Err(format!(
                        "Edit panicked: {}",
                        crate::utf8::panic_message(payload)
                    )),
                }
            }
            None => Err("File is not open".to_string()),
        };
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(err) => return Some(err),
        };
        self.note_tab_change(tab_id);
        let (echo, revision) = match self.find_tab(tab_id) {
            Some(tab) => {
                let from = outcome.first_line.saturating_sub(EDIT_CONTEXT).max(1);
                let to = (outcome.last_line + EDIT_CONTEXT).min(from + EDIT_ECHO_LINES - 1);
                (
                    agent::read_lines_rope(tab.buffer.rope(), from, to).unwrap_or_default(),
                    tab.buffer.revision(),
                )
            }
            None => (String::new(), 0),
        };
        // The model has seen its own edit: a next line-range edit is not stale.
        self.agent
            .reads
            .insert(path.to_path_buf(), agent::ReadMark::Buffer(revision));
        self.note_agent_change(path, outcome.first_line, &outcome.summary);
        let report = format!(
            "{} in {} (unsaved; Ctrl+Z undoes it). Now:\n{echo}",
            outcome.summary,
            self.agent_relative(path)
        );
        match self.lsp_doc_version(tab_id) {
            Some(version) => {
                self.agent.wait = Some(agent::ToolWait {
                    turn,
                    call_id,
                    deadline: std::time::Instant::now() + DIAGNOSTICS_WAIT,
                    kind: agent::WaitKind::Diagnostics {
                        path: path.to_path_buf(),
                        version,
                        report,
                    },
                });
                self.agent.status = "checking".to_string();
                None
            }
            None => Some(report),
        }
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
        // Models often send `content` (OpenAI-ish) instead of our `text` schema key.
        let text = value
            .get("text")
            .or_else(|| value.get("content"))
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
                let name = self.agent_relative(&path);
                if self.agent_may_move_view() {
                    self.open_path(path);
                    format!("Opening {name}")
                } else {
                    if self.tab_with_path(&path).is_none() {
                        self.open_in_background(path);
                    }
                    format!(
                        "Opened {name} in a background tab. The user is typing in the editor, so the view did not change."
                    )
                }
            }
            Err(err) => err,
        }
    }

    /// The visible tab and the cursor may move only while the user is in the
    /// assistant panel. Someone typing in the editor keeps their place.
    pub(super) fn agent_may_move_view(&self) -> bool {
        self.agent.open && self.agent.focused
    }

    /// Opens `path` in a new tab behind the visible one, or finds the open tab.
    /// Neither the visible tab nor the focus changes.
    pub(super) fn open_in_background(&mut self, path: std::path::PathBuf) -> crate::layout::TabId {
        let path = agent::strip_verbatim(&path);
        if let Some(tab) = self.tab_with_path(&path) {
            return tab;
        }
        let pane = self.active;
        let buffer = self.style_buffer(crate::buffer::Buffer::new());
        let tab = self.alloc_tab(buffer, None);
        let id = tab.id;
        if let Some(editor) = self.editor_mut(pane) {
            editor.tabs.push(tab);
        }
        self.start_load(
            pane,
            id,
            path,
            crate::jobs::LoadKind::Open {
                encoding: None,
                place: None,
                focus: false,
            },
            true,
        );
        id
    }

    fn tool_search(
        &mut self,
        value: &serde_json::Value,
        turn: u64,
        call_id: String,
    ) -> Option<String> {
        let query = value
            .get("query")
            .and_then(|query| query.as_str())
            .unwrap_or("")
            .to_string();
        if query.trim().is_empty() {
            return Some("search needs a query".to_string());
        }
        if self.explorer.root.as_os_str().is_empty() {
            return Some("No workspace folder is open".to_string());
        }
        let glob = value
            .get("glob")
            .and_then(|glob| glob.as_str())
            .map(str::to_string);
        let open = self
            .editors
            .iter()
            .flat_map(|editor| editor.tabs.iter())
            .filter_map(|tab| Some((tab.path.clone()?, tab.buffer.rope().clone())))
            .collect();
        self.jobs.push(Job::AgentDisk {
            turn,
            call_id,
            task: DiskTask::Search {
                root: self.explorer.root.clone(),
                query,
                glob,
                open,
                hidden: self.file_prefs.show_hidden,
            },
        });
        None
    }

    fn tool_lsp(
        &mut self,
        value: &serde_json::Value,
        turn: u64,
        call_id: String,
    ) -> Option<String> {
        let Some(op) = value
            .get("op")
            .and_then(|op| op.as_str())
            .and_then(agent::LspOp::parse)
        else {
            return Some("lsp needs op: definition, references, or hover".to_string());
        };
        let line = value
            .get("line")
            .and_then(loose_number)
            .filter(|line| *line > 0);
        let Some(line) = line else {
            return Some("lsp needs a 1-based line".to_string());
        };
        let path = match value.get("path").and_then(|path| path.as_str()) {
            Some(raw) => match agent::within(&self.explorer.root, raw) {
                Ok(path) => path,
                Err(err) => return Some(err),
            },
            None => match self.path() {
                Some(path) => path.to_path_buf(),
                None => return Some("lsp needs a path".to_string()),
            },
        };
        let query = agent::LspQuery {
            op,
            line,
            column: value
                .get("column")
                .and_then(loose_number)
                .filter(|column| *column > 0),
            symbol: value
                .get("symbol")
                .and_then(|symbol| symbol.as_str())
                .map(str::trim)
                .filter(|symbol| !symbol.is_empty())
                .map(str::to_string),
        };
        if let Some(tab) = self.tab_with_path(&path) {
            return self.send_agent_lsp(turn, call_id, tab, &path, &query);
        }
        if !path.is_file() {
            return Some(format!("{} does not exist", self.agent_relative(&path)));
        }
        let tab = self.open_in_background(path.clone());
        self.agent.wait = Some(agent::ToolWait {
            turn,
            call_id,
            deadline: std::time::Instant::now() + TOOL_WAIT,
            kind: agent::WaitKind::Load {
                tab,
                path,
                then: agent::AfterLoad::Lsp(query),
            },
        });
        None
    }

    /// Sends one language server query for an open tab. `None` while it waits.
    fn send_agent_lsp(
        &mut self,
        turn: u64,
        call_id: String,
        tab_id: crate::layout::TabId,
        path: &std::path::Path,
        query: &agent::LspQuery,
    ) -> Option<String> {
        let Some(tab) = self.find_tab(tab_id) else {
            return Some("File is not open".to_string());
        };
        let rope = tab.buffer.rope();
        let lines = rope.len_lines();
        if query.line > lines {
            return Some(format!("Line {} is past the end of the file", query.line));
        }
        let line_start = rope.line_to_char(query.line - 1);
        let text: String = rope.line(query.line - 1).chars().collect();
        let column = match (&query.symbol, query.column) {
            (Some(symbol), _) => match symbol_column(&text, symbol) {
                Some(column) => column,
                None => {
                    return Some(format!(
                        "{symbol} is not on line {}. That line is: {}",
                        query.line,
                        clip(text.trim_end(), 160)
                    ));
                }
            },
            (None, Some(column)) => column - 1,
            (None, None) => text.chars().take_while(|ch| ch.is_whitespace()).count(),
        };
        let line_len = text.trim_end_matches(['\r', '\n']).chars().count();
        let at_char = line_start + column.min(line_len);
        let at = rope.char_to_byte(at_char);
        let kind = match query.op {
            agent::LspOp::Definition => crate::lsp::protocol::QueryKind::Definition,
            agent::LspOp::References => crate::lsp::protocol::QueryKind::References,
            agent::LspOp::Hover => crate::lsp::protocol::QueryKind::Hover,
        };
        match self.agent_lsp_query(path, at, kind) {
            Ok(token) => {
                self.agent.wait = Some(agent::ToolWait {
                    turn,
                    call_id,
                    deadline: std::time::Instant::now() + TOOL_WAIT,
                    kind: agent::WaitKind::Lsp {
                        token,
                        op: query.op,
                    },
                });
                None
            }
            Err(err) => Some(err),
        }
    }

    /// The assistant is waiting for the language server reply with `token`.
    pub(super) fn agent_waits_lsp(&self, token: u64) -> bool {
        matches!(
            &self.agent.wait,
            Some(agent::ToolWait { kind: agent::WaitKind::Lsp { token: wanted, .. }, .. })
                if *wanted == token
        )
    }

    /// A language server reply. `true` when it answered the assistant's query.
    pub(super) fn agent_lsp_answer(&mut self, token: u64, answer: AgentLspAnswer) -> bool {
        let Some(wait) = &self.agent.wait else {
            return false;
        };
        let agent::WaitKind::Lsp { token: wanted, op } = wait.kind else {
            return false;
        };
        if wanted != token {
            return false;
        }
        let Some(wait) = self.agent.wait.take() else {
            return false;
        };
        let text = match answer {
            AgentLspAnswer::Hover(Some(text)) => clip(text.trim(), 2_000),
            AgentLspAnswer::Hover(None) => "No hover information here".to_string(),
            AgentLspAnswer::Locations(locations) if locations.is_empty() => match op {
                agent::LspOp::Definition => "No definition found".to_string(),
                _ => "No references found".to_string(),
            },
            AgentLspAnswer::Locations(locations) => {
                let total = locations.len();
                let mut lines: Vec<String> = locations
                    .iter()
                    .take(40)
                    .map(|target| self.target_label(target))
                    .collect();
                if total > 40 {
                    lines.push(format!("… and {} more", total - 40));
                }
                lines.join("\n")
            }
        };
        self.reply_tool(wait.turn, wait.call_id, text);
        true
    }

    /// Fresh diagnostics arrived for `path` at `version`.
    pub(super) fn agent_diagnostics_in(&mut self, path: &std::path::Path, version: u64) {
        let fresh = matches!(
            &self.agent.wait,
            Some(agent::ToolWait {
                kind: agent::WaitKind::Diagnostics { path: wanted, version: before, .. },
                ..
            }) if super::same_path(wanted, path) && (version > *before || version == 0)
        );
        if fresh {
            self.finish_diagnostics_wait(false);
        }
    }

    fn finish_diagnostics_wait(&mut self, timed_out: bool) {
        let Some(wait) = self.agent.wait.take() else {
            return;
        };
        let agent::WaitKind::Diagnostics { path, report, .. } = wait.kind else {
            self.agent.wait = Some(wait);
            return;
        };
        let found = self.diagnostic_lines(&path, true);
        let tail = match (found.is_empty(), timed_out) {
            (true, false) => "No errors or warnings after the edit.".to_string(),
            (true, true) => "The language server has not reported on the edit yet.".to_string(),
            (false, _) => format!("Errors and warnings now:\n{}", found.join("\n")),
        };
        self.reply_tool(wait.turn, wait.call_id, format!("{report}\n{tail}"));
    }

    /// A background tab finished loading (or failed). Runs the tool waiting on it.
    pub(super) fn agent_tab_loaded(&mut self, tab_id: crate::layout::TabId, error: Option<String>) {
        let waiting = matches!(
            &self.agent.wait,
            Some(agent::ToolWait { kind: agent::WaitKind::Load { tab, .. }, .. }) if *tab == tab_id
        );
        if !waiting {
            return;
        }
        let Some(wait) = self.agent.wait.take() else {
            return;
        };
        let agent::WaitKind::Load { path, then, .. } = wait.kind else {
            return;
        };
        if let Some(error) = error {
            self.reply_tool(wait.turn, wait.call_id, error);
            return;
        }
        let landed = self
            .find_tab(tab_id)
            .and_then(|tab| tab.path.as_deref())
            .is_some_and(|open| super::same_path(open, &path));
        if !landed {
            let name = self.agent_relative(&path);
            self.reply_tool(wait.turn, wait.call_id, format!("Could not open {name}"));
            return;
        }
        let reply = match then {
            agent::AfterLoad::Edit(edit) => {
                if let Some(agent::ReadMark::Disk(seen)) = self.agent.reads.get(&path).copied()
                    && let Some(tab) = self.find_tab(tab_id)
                    && tab.buffer.fingerprint() == seen
                {
                    let revision = tab.buffer.revision();
                    self.agent
                        .reads
                        .insert(path.clone(), agent::ReadMark::Buffer(revision));
                }
                self.edit_open_tab(wait.turn, wait.call_id.clone(), tab_id, &path, &edit)
            }
            agent::AfterLoad::Lsp(query) => {
                self.send_agent_lsp(wait.turn, wait.call_id.clone(), tab_id, &path, &query)
            }
        };
        if let Some(reply) = reply {
            self.reply_tool(wait.turn, wait.call_id, reply);
        }
    }

    /// Ends a wait that took too long. Called on the busy tick.
    fn expire_agent_wait(&mut self) {
        let expired = self
            .agent
            .wait
            .as_ref()
            .is_some_and(|wait| std::time::Instant::now() >= wait.deadline);
        if !expired {
            return;
        }
        if matches!(
            self.agent.wait.as_ref().map(|wait| &wait.kind),
            Some(agent::WaitKind::Diagnostics { .. })
        ) {
            self.finish_diagnostics_wait(true);
            return;
        }
        let Some(wait) = self.agent.wait.take() else {
            return;
        };
        let text = match wait.kind {
            agent::WaitKind::Load { path, .. } => {
                format!("Opening {} took too long", self.agent_relative(&path))
            }
            agent::WaitKind::Lsp { .. } => {
                "The language server did not answer. It may still be starting; try search instead."
                    .to_string()
            }
            agent::WaitKind::Diagnostics { .. } => return,
        };
        self.reply_tool(wait.turn, wait.call_id, text);
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
        let line = value.get("line").and_then(loose_number).unwrap_or(1).max(1);
        let path = match value.get("path").and_then(|path| path.as_str()) {
            Some(raw) => match agent::within(&self.explorer.root, raw) {
                Ok(path) => Some(path),
                Err(err) => return err,
            },
            None => None,
        };
        if !self.agent_may_move_view() {
            let place = match &path {
                Some(path) => format!("{}:{line}", self.agent_relative(path)),
                None => format!("line {line}"),
            };
            return format!(
                "The user is typing in the editor, so the cursor stays where it is. Tell them to look at {place}."
            );
        }
        match path {
            Some(path) => self.open_at(path, Some(line)),
            None => self.buffer_mut().goto_line(line - 1),
        }
        format!("Line {line}")
    }
}

impl App {
    fn open_command_bar_from_agent(&mut self) {
        // Mirror former Tab behavior: leave agent prompt for the command bar.
        self.agent.focused = false;
        self.agent.popup = None;
        self.open_command_bar();
    }

    pub(super) fn refresh_agent_popup(&mut self) {
        if self.agent.pending.is_some() || self.agent.reading {
            self.agent.popup = None;
            return;
        }
        let prompt = self.agent.prompt.clone();
        let cursor = self.agent.cursor;

        if let Some(filter) = agent::slash_filter(&prompt) {
            // Only while the cursor is still inside the `/cmd` token (no args yet, or editing name).
            let name_end = 1 + filter.len();
            if cursor <= name_end && !prompt[1..].chars().any(|c| c.is_whitespace()) {
                let _ = agent::filter_commands(filter); // ensure module linked
                self.agent.popup = Some(crate::agent::AgentPopup::Slash {
                    filter: filter.to_string(),
                    selected: self
                        .agent
                        .popup
                        .as_ref()
                        .and_then(|p| match p {
                            crate::agent::AgentPopup::Slash { selected, .. } => Some(*selected),
                            _ => None,
                        })
                        .unwrap_or(0),
                });
                if let Some(crate::agent::AgentPopup::Slash { selected, filter }) =
                    self.agent.popup.as_mut()
                {
                    let n = agent::filter_commands(filter).len();
                    if n == 0 {
                        // Keep popup to show "no matches" or close? Close when empty filter of unknown.
                    } else if *selected >= n {
                        *selected = n - 1;
                    }
                }
                return;
            }
        }

        if let Some(active) = agent::active_mention(&prompt, cursor) {
            self.ensure_file_index();
            let files = self
                .file_index
                .as_ref()
                .map(|idx| idx.files.as_slice())
                .unwrap_or(&[]);
            let recent = self.recent_files();
            let active_dir = self.active_relative_dir();
            let rows = agent::suggest_files(files, &active.query, &recent, &active_dir);
            let paths: Vec<String> = rows
                .iter()
                .filter_map(|i| files.get(*i as usize).map(|f| f.relative.clone()))
                .collect();
            let selected = self
                .agent
                .popup
                .as_ref()
                .and_then(|p| match p {
                    crate::agent::AgentPopup::Mention { selected, .. } => Some(*selected),
                    _ => None,
                })
                .unwrap_or(0)
                .min(paths.len().saturating_sub(1));
            self.agent.popup = Some(crate::agent::AgentPopup::Mention {
                at: active.at,
                query: active.query,
                selected,
                paths,
            });
            return;
        }

        self.agent.popup = None;
    }

    fn nudge_agent_popup(&mut self, down: bool) {
        match self.agent.popup.as_mut() {
            Some(crate::agent::AgentPopup::Slash { selected, filter }) => {
                let n = agent::filter_commands(filter).len().max(1);
                if down {
                    *selected = (*selected + 1) % n;
                } else {
                    *selected = selected.checked_sub(1).unwrap_or(n - 1);
                }
            }
            Some(crate::agent::AgentPopup::Mention {
                selected, paths, ..
            }) => {
                let n = paths.len().max(1);
                if down {
                    *selected = (*selected + 1) % n;
                } else {
                    *selected = selected.checked_sub(1).unwrap_or(n - 1);
                }
            }
            None => {}
        }
    }

    /// `run_command` is true when Enter was pressed on a slash popup (execute).
    fn accept_agent_popup(&mut self, run_command: bool) {
        let Some(popup) = self.agent.popup.clone() else {
            return;
        };
        match popup {
            crate::agent::AgentPopup::Slash { filter, selected } => {
                let hits = agent::filter_commands(&filter);
                let Some(cmd) = hits.get(selected).copied() else {
                    self.agent.popup = None;
                    return;
                };
                // Replace `/filter` with `/name` (and a trailing space for args).
                let replacement = if run_command
                    && cmd.name != "model"
                    && cmd.name != "provider"
                    && cmd.name != "context"
                    && cmd.name != "stream"
                    && cmd.name != "auto-approve"
                    && cmd.name != "think"
                {
                    format!("/{}", cmd.name)
                } else {
                    format!("/{} ", cmd.name)
                };
                self.agent.prompt = replacement.clone();
                self.agent.cursor = self.agent.prompt.chars().count();
                self.agent.anchor = self.agent.cursor;
                self.agent.popup = None;
                if run_command {
                    // Commands that take no args (or optional) run immediately on Enter.
                    if !matches!(
                        cmd.name,
                        "model" | "provider" | "context" | "stream" | "auto-approve" | "think"
                    ) || self.agent.prompt.split_whitespace().nth(1).is_none()
                        && matches!(cmd.name, "model" | "provider" | "context")
                    {
                        if let Some(parsed) = agent::parse_slash(&self.agent.prompt) {
                            self.run_slash_command(parsed);
                        }
                    } else if matches!(cmd.name, "stream" | "auto-approve" | "think") {
                        // Need on|off — leave prompt for typing.
                        self.refresh_agent_popup();
                    }
                } else {
                    // Tab: just complete the name.
                    self.refresh_agent_popup();
                }
            }
            crate::agent::AgentPopup::Mention {
                at,
                query,
                selected,
                paths,
            } => {
                let Some(path) = paths.get(selected).cloned() else {
                    self.agent.popup = None;
                    return;
                };
                let active = agent::ActiveMention {
                    at,
                    query,
                    end: self.agent.cursor,
                };
                let (next, cursor) = agent::insert_mention(&self.agent.prompt, &active, &path);
                self.agent.prompt = next;
                self.agent.cursor = cursor;
                self.agent.anchor = cursor;
                self.agent.popup = None;
            }
        }
    }

    fn run_slash_command(&mut self, parsed: agent::ParsedSlash) {
        self.agent.prompt.clear();
        self.agent.cursor = 0;
        self.agent.anchor = 0;
        self.agent.popup = None;
        let name = parsed.name.as_str();
        let args = parsed.args.trim();
        if name.is_empty() {
            self.push_line(false, "Type a command after `/`. Try /help.".into());
            return;
        }
        let Some(_) = agent::lookup(name) else {
            self.push_line(false, format!("Unknown command /{name}. Try /help."));
            return;
        };
        match name {
            "new" | "clear" => self.clear_agent_chat(),
            "help" => self.push_line(false, agent::help_text()),
            "stop" => {
                if self.agent.busy {
                    self.cancel_agent();
                    self.push_line(false, "Stopped.".into());
                } else {
                    self.push_line(false, "Nothing is running.".into());
                }
            }
            "retry" => {
                if let Some(last) = self.agent.last_prompt.clone() {
                    self.agent.prompt = last;
                    self.agent.cursor = self.agent.prompt.chars().count();
                    self.agent.anchor = self.agent.cursor;
                    self.submit_agent();
                } else {
                    self.push_line(false, "No previous prompt to retry.".into());
                }
            }
            "compact" => self.compact_now(),
            "plan" => self.set_agent_mode(agent::AgentMode::Plan),
            "ask" => self.set_agent_mode(agent::AgentMode::Ask),
            "agent" => self.set_agent_mode(agent::AgentMode::Agent),
            "changes" => self.slash_changes(args),
            "settings" => {
                self.close_picker();
                self.about = false;
                self.settings = Some(crate::settings::Page::new());
                // Prefer AI section when possible.
                if let Some(page) = self.settings.as_mut() {
                    let sections = crate::settings::sections();
                    if let Some((idx, _)) = sections
                        .iter()
                        .enumerate()
                        .find(|(_, s)| s.name.eq_ignore_ascii_case("AI"))
                    {
                        page.section = idx;
                        page.row = 0;
                    }
                }
                self.agent.focused = false;
                self.push_line(false, "Opened Settings → AI.".into());
            }
            "model" => self.slash_model(args),
            "provider" => self.slash_provider(args),
            "context" => self.slash_context(args),
            "stream" => self.slash_on_off("stream", args, true),
            "auto-approve" => self.slash_on_off("auto-approve", args, false),
            "think" => self.slash_on_off("think", args, true),
            _ => self.push_line(false, format!("/{name} is not implemented.")),
        }
    }

    fn slash_on_off(&mut self, which: &str, args: &str, default_on_label: bool) {
        let _ = default_on_label;
        let token = args
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let set = match token.as_str() {
            "" => None,
            "on" | "true" | "1" => Some(true),
            "off" | "false" | "0" => Some(false),
            _ => {
                self.push_line(false, format!("Usage: /{which} on|off"));
                return;
            }
        };
        match which {
            "stream" => {
                if let Some(v) = set {
                    self.ai.streaming = v;
                    self.persist_ai_prefs();
                }
                self.push_line(false, format!("stream {}", on_off(self.ai.streaming)));
            }
            "auto-approve" => {
                if let Some(v) = set {
                    self.ai.auto_approve = v;
                    self.persist_ai_prefs();
                }
                self.push_line(
                    false,
                    format!("auto-approve {}", on_off(self.ai.auto_approve)),
                );
            }
            "think" => {
                if let Some(v) = set {
                    self.ai.think = v;
                    self.persist_ai_prefs();
                }
                self.push_line(false, format!("think {}", on_off(self.ai.think)));
            }
            _ => {}
        }
    }

    fn slash_provider(&mut self, args: &str) {
        let name = args.trim();
        if !name.is_empty() {
            if self.ai.find_named(name).is_some() {
                self.ai.switch_named(name);
            } else if let Some(provider) = crate::settings::AiProvider::parse(name) {
                self.ai.switch_provider(provider);
            } else {
                let mut names: Vec<String> =
                    ["ollama", "lmstudio", "openai-compatible", "openrouter"]
                        .iter()
                        .map(|name| name.to_string())
                        .collect();
                names.extend(self.ai.named.iter().map(|entry| entry.name.clone()));
                self.push_line(false, format!("Usage: /provider [{}]", names.join("|")));
                return;
            }
            self.persist_ai_prefs();
        }
        let named = match self.ai.active_named() {
            Some(index) => format!("{} · ", self.ai.named[index].name),
            None => String::new(),
        };
        self.push_line(
            false,
            format!(
                "provider {named}{} · server {}",
                self.ai.provider.as_str(),
                self.ai.endpoint()
            ),
        );
    }

    fn slash_context(&mut self, args: &str) {
        let token = args.split_whitespace().next().unwrap_or("");
        if !token.is_empty() {
            if token.eq_ignore_ascii_case("auto") {
                self.ai.context_size = "auto".into();
            } else if let Ok(n) = token.parse::<u32>() {
                if (1024..=262_144).contains(&n) {
                    self.ai.context_size = n.to_string();
                } else {
                    self.push_line(false, "Context size must be 1024..=262144.".into());
                    return;
                }
            } else {
                self.push_line(false, "Usage: /context [n|auto]".into());
                return;
            }
            self.persist_ai_prefs();
        }
        self.push_line(
            false,
            format!(
                "context {} (resolved {})",
                self.ai.context_size,
                self.ai.resolved_context_size()
            ),
        );
    }

    pub(crate) fn slash_model(&mut self, args: &str) {
        let name = args.trim();
        if !name.is_empty() {
            self.ai.model = name.to_string();
            self.ai.remember_current();
            self.persist_ai_prefs();
            self.push_line(false, format!("model set to {name}"));
            return;
        }
        let current = if self.ai.model.trim().is_empty() {
            "(none)".to_string()
        } else {
            self.ai.model.clone()
        };
        // Listing runs off the UI thread; the answer lands in `models_listed`.
        let job = self.list_models_job();
        if let crate::jobs::Job::ListModels { url, .. } = &job {
            self.agent.listing_models = Some(url.clone());
        }
        self.jobs.push(job);
        self.push_line(false, format!("model {current} · listing models…"));
    }

    /// `GET /models` (or Ollama `/api/tags`) with this provider's key and headers.
    pub(crate) fn list_models_job(&self) -> crate::jobs::Job {
        let (api_key, _) = self.ai.resolved_api_key();
        crate::jobs::Job::ListModels {
            url: crate::agent::models_url(
                &self.ai.endpoint(),
                self.ai.provider == crate::settings::AiProvider::Ollama,
            ),
            api_key,
            headers: self.ai.extra_headers(),
        }
    }

    /// The answer to `/model` with no name. A stale answer (another `/model` since) is dropped.
    pub(crate) fn models_listed(&mut self, url: &str, result: Result<Vec<String>, String>) {
        if self.agent.listing_models.as_deref() != Some(url) {
            return;
        }
        self.agent.listing_models = None;
        match result {
            Ok(names) if !names.is_empty() => {
                let mut lines = vec![format!("Available from {url}:")];
                for name in names.iter().take(30) {
                    lines.push(format!("  {name}"));
                }
                if names.len() > 30 {
                    lines.push(format!("  … and {} more", names.len() - 30));
                }
                self.push_line(false, lines.join("\n"));
            }
            Ok(_) => self.push_line(false, format!("{url} listed no models")),
            Err(err) => self.push_line(false, format!("Could not list models: {err}")),
        }
    }

    fn persist_ai_prefs(&mut self) {
        self.queue_settings_save();
    }

    /// A turn answered: its server and model go to the front of the recent lists,
    /// unless the provider, server, or model changed while it ran.
    fn remember_working_target(&mut self) {
        let Some((provider, server, model)) = self.agent_target.take() else {
            return;
        };
        if self.ai.provider != provider
            || self.ai.endpoint() != server
            || self.ai.model.trim() != model
        {
            return;
        }
        if self.ai.remember_current() {
            self.queue_settings_save();
        }
    }

    fn attach_mentions(&self, prompt: &str) -> String {
        let mentions = agent::parse_mentions(prompt);
        if mentions.is_empty() {
            return String::new();
        }
        let mut budget = agent::ATTACH_BUDGET;
        let mut parts = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for mention in mentions {
            if !seen.insert(mention.path.clone()) {
                continue;
            }
            if budget == 0 {
                parts.push(format!(
                    "(attach budget reached; skipped @{})",
                    mention.path
                ));
                break;
            }
            let path = match agent::within(&self.explorer.root, &mention.path) {
                Ok(p) => p,
                Err(err) => {
                    parts.push(format!("@{}: {err}", mention.path));
                    continue;
                }
            };
            match read_attachment(&path, budget.min(agent::ATTACH_FILE_CAP)) {
                Ok((body, truncated)) => {
                    let used = body.chars().count();
                    budget = budget.saturating_sub(used);
                    let note = if truncated { " (truncated)" } else { "" };
                    parts.push(format!(
                        "Attached file @{}{note}:\n```\n{body}\n```",
                        mention.path
                    ));
                }
                Err(err) => parts.push(format!("@{}: {err}", mention.path)),
            }
        }
        parts.join("\n\n")
    }

    fn ensure_file_index(&mut self) {
        if self.file_index.is_none() {
            // Kick the same indexing path the file picker uses.
            self.queue_index();
        }
    }

    fn recent_files(&self) -> Vec<String> {
        // Prefer open tab paths relative to workspace.
        let root = &self.explorer.root;
        let mut out = Vec::new();
        for editor in &self.editors {
            for tab in editor.tabs() {
                let Some(path) = tab.path.as_ref() else {
                    continue;
                };
                let Ok(rel) = path.strip_prefix(root) else {
                    continue;
                };
                let s = rel.to_string_lossy().replace('\\', "/");
                if !s.is_empty() && !out.contains(&s) {
                    out.push(s);
                }
            }
        }
        out
    }

    fn active_relative_dir(&self) -> String {
        let root = &self.explorer.root;
        let Some(path) = self.active_editor().tab().path.as_ref() else {
            return String::new();
        };
        let Ok(rel) = path.strip_prefix(root) else {
            return String::new();
        };
        rel.parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    }
}

fn on_off(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

fn read_attachment(path: &std::path::Path, cap: usize) -> Result<(String, bool), String> {
    crate::agent::check_file_size(path)?;
    let bytes = std::fs::read(path).map_err(|err| format!("Cannot read: {err}"))?;
    let (text, _enc, had_errors) = encoding_rs::UTF_8.decode(&bytes);
    if had_errors {
        // still use lossy
    }
    let mut truncated = false;
    let body: String = if text.chars().count() > cap {
        truncated = true;
        text.chars().take(cap).collect()
    } else {
        text.into_owned()
    };
    Ok((body, truncated))
}

/// Max tokens, OpenRouter headers, timeout, and retries for one turn.
pub(crate) fn ai_tuning(ai: &crate::settings::AiPrefs) -> crate::agent::Tuning {
    crate::agent::Tuning {
        max_tokens: ai.resolved_max_tokens(),
        headers: ai.extra_headers(),
        timeout: Some(std::time::Duration::from_secs(u64::from(
            ai.resolved_timeout(),
        ))),
        retries: ai.resolved_retries(),
        tools: ai.model_tools_mode(),
        ..crate::agent::Tuning::default()
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
        let row = prompt_row_for(&rows, cursor, len);
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
        self.refresh_agent_popup();
    }

    /// Characters memory keeps: small without auto compact, as before; large with it.
    fn memory_cap(&self) -> usize {
        if self.ai.auto_compact {
            MEMORY_STORE
        } else {
            MEMORY_BUDGET
        }
    }

    fn remember_user(&mut self, content: &str) {
        self.agent.memory.push(serde_json::json!({
            "role": "user",
            "content": clip(content, MEMORY_BUDGET)
        }));
        let cap = self.memory_cap();
        trim_memory(&mut self.agent.memory, cap);
    }

    fn remember_assistant(&mut self, content: &str) {
        if content.trim().is_empty() {
            return;
        }
        self.agent.memory.push(serde_json::json!({
            "role": "assistant",
            "content": clip(content, MEMORY_TOOL_CLIP * 4)
        }));
        let cap = self.memory_cap();
        trim_memory(&mut self.agent.memory, cap);
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
        let cap = self.memory_cap();
        trim_memory(&mut self.agent.memory, cap);
    }

    fn remember_tool_result(&mut self, call_id: &str, content: &str) {
        // With auto compact the newest results go out whole (older ones are cut when
        // sent); without it only a short trail fits, so each result is cut now.
        let stored = if self.ai.auto_compact {
            clip(content.trim(), TOOL_RESULT_STORE)
        } else {
            summarize_tool_result(content)
        };
        self.agent.memory.push(serde_json::json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": stored
        }));
        let cap = self.memory_cap();
        trim_memory(&mut self.agent.memory, cap);
    }
}

fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count() as f64;
    (chars / CHARS_PER_TOKEN).ceil() as usize
}

fn estimate_message_tokens(message: &serde_json::Value) -> usize {
    estimate_tokens(&message.to_string())
}

/// Keep system+tools+current user outside; drop oldest history/tool-trail first.
fn budget_memory(
    memory: &[serde_json::Value],
    context_tokens: u32,
    current_prompt: &str,
) -> Vec<serde_json::Value> {
    let mut out: Vec<_> = memory.iter().map(sanitize_memory_message).collect();
    clear_old_tool_results(&mut out, KEEP_TOOL_RESULTS);
    // Current user message is appended by the worker; drop a trailing duplicate user if present.
    if out
        .last()
        .and_then(|last| last.get("role"))
        .and_then(|r| r.as_str())
        == Some("user")
    {
        out.pop();
    }
    trim_memory(&mut out, MEMORY_BUDGET);
    let keep = HISTORY.saturating_mul(2).max(4);
    if out.len() > keep {
        out.drain(0..out.len() - keep);
    }
    reconcile_tool_pairs(&mut out);

    let system_reserve = estimate_tokens(agent::SYSTEM) + 900; // tools + reply margin
    let prompt_tokens = estimate_tokens(current_prompt);
    let history_budget = (context_tokens as usize)
        .saturating_sub(system_reserve)
        .saturating_sub(prompt_tokens)
        .max(256);
    while !out.is_empty() && out.iter().map(estimate_message_tokens).sum::<usize>() > history_budget
    {
        out.remove(0);
        reconcile_tool_pairs(&mut out);
    }
    out
}

/// Tokens one request takes: system prompt (with summary), tool and reply margin,
/// history, and the prompt.
fn estimate_request(summary: Option<&str>, history: &[serde_json::Value], prompt: &str) -> u32 {
    let tokens = estimate_tokens(agent::SYSTEM)
        + 900
        + summary.map_or(0, estimate_tokens)
        + history.iter().map(estimate_message_tokens).sum::<usize>()
        + estimate_tokens(prompt);
    u32::try_from(tokens).unwrap_or(u32::MAX)
}

/// Auto compact: the history to send and how many leading memory messages to fold
/// into the summary (0 = none). Nothing is folded while the request fits in the
/// context less `buffer_percent`. Past that, the newest messages up to `keep`
/// tokens (at most half the budget) stay verbatim, and the cut moves forward to
/// the next user message, so a tool call is never separated from its result.
fn compact_plan(
    memory: &[serde_json::Value],
    context_tokens: u32,
    buffer_percent: u32,
    keep: u32,
    current_prompt: &str,
    summary: Option<&str>,
) -> (Vec<serde_json::Value>, usize) {
    let mut out: Vec<_> = memory.iter().map(sanitize_memory_message).collect();
    clear_old_tool_results(&mut out, KEEP_TOOL_RESULTS);
    if out
        .last()
        .and_then(|last| last.get("role"))
        .and_then(|r| r.as_str())
        == Some("user")
    {
        out.pop();
    }
    let limit = context_tokens as usize * (100 - buffer_percent.min(90) as usize) / 100;
    let reserve = estimate_tokens(agent::SYSTEM)
        + 900
        + summary.map_or(0, estimate_tokens)
        + estimate_tokens(current_prompt);
    let budget = limit.saturating_sub(reserve).max(256);
    let total: usize = out.iter().map(estimate_message_tokens).sum();
    if total <= budget {
        reconcile_tool_pairs(&mut out);
        return (out, 0);
    }
    let keep = (keep as usize).min(budget / 2);
    let mut kept = 0;
    let mut start = out.len();
    while start > 0 {
        let tokens = estimate_message_tokens(&out[start - 1]);
        if kept + tokens > keep {
            break;
        }
        kept += tokens;
        start -= 1;
    }
    while start < out.len() && out[start].get("role").and_then(|r| r.as_str()) != Some("user") {
        start += 1;
    }
    let mut history = out.split_off(start);
    reconcile_tool_pairs(&mut history);
    (history, start)
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
    let trimmed = content.trim();
    if trimmed.chars().count() <= MEMORY_TOOL_CLIP {
        return trimmed.to_string();
    }
    // Prefer a short head that still names the outcome.
    let head: String = trimmed
        .chars()
        .take(MEMORY_TOOL_CLIP.saturating_sub(20))
        .collect();
    format!("{head}… (truncated)")
}

fn trim_memory(memory: &mut Vec<serde_json::Value>, cap: usize) {
    while memory_chars(memory) > cap && !memory.is_empty() {
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

fn diff_preview_lines(find: &str, replace: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for line in find.lines().take(4) {
        lines.push(format!("- {}", clip_width(line, 72)));
    }
    if find.lines().count() > 4 {
        lines.push("- …".to_string());
    }
    if find.is_empty() {
        lines.push("- (empty)".to_string());
    }
    for line in replace.lines().take(4) {
        lines.push(format!("+ {}", clip_width(line, 72)));
    }
    if replace.lines().count() > 4 {
        lines.push("+ …".to_string());
    }
    if replace.is_empty() {
        lines.push("+ (empty)".to_string());
    }
    lines
}

fn plus_preview_lines(text: &str, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = text
        .lines()
        .take(max_lines)
        .map(|line| format!("+ {}", clip_width(line, 72)))
        .collect();
    if text.lines().count() > max_lines {
        lines.push("+ …".to_string());
    }
    if text.is_empty() {
        lines.push("+ (empty)".to_string());
    }
    lines
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

fn clip_width(text: &str, max: usize) -> String {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if out.width() + w > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

/// A language server answer for the assistant.
pub(super) enum AgentLspAnswer {
    Hover(Option<String>),
    Locations(Vec<crate::lsp::protocol::JumpTarget>),
}

/// `12` or `"12"`.
fn loose_number(value: &serde_json::Value) -> Option<usize> {
    match value {
        serde_json::Value::Number(number) => number.as_u64().map(|n| n as usize),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Char column of `symbol` on `line`: a whole-word match first, then any match.
fn symbol_column(line: &str, symbol: &str) -> Option<usize> {
    let word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let mut fallback = None;
    for (byte, _) in line.match_indices(symbol) {
        let before = line[..byte].chars().next_back();
        let after = line[byte + symbol.len()..].chars().next();
        let column = line[..byte].chars().count();
        if !before.is_some_and(word) && !after.is_some_and(word) {
            return Some(column);
        }
        fallback.get_or_insert(column);
    }
    fallback
}

/// Old tool results beyond the newest `keep` are cut to a short head, the way
/// microcompaction works: the model still sees what it did, not every byte it read.
fn clear_old_tool_results(history: &mut [serde_json::Value], keep: usize) {
    let tools: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, message)| message.get("role").and_then(|r| r.as_str()) == Some("tool"))
        .map(|(index, _)| index)
        .collect();
    let old = tools.len().saturating_sub(keep);
    for &index in &tools[..old] {
        let Some(content) = history[index].get("content").and_then(|c| c.as_str()) else {
            continue;
        };
        if content.chars().count() <= MEMORY_TOOL_CLIP {
            continue;
        }
        let head: String = content.chars().take(MEMORY_TOOL_CLIP).collect();
        history[index]["content"] =
            serde_json::Value::String(format!("{head}… [older tool result cleared]"));
    }
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
    #[serde(default, deserialize_with = "loose_bool")]
    replace_all: bool,
}

/// `true`, `"true"`, `1`, or `"yes"`; models send all of them.
fn loose_bool<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Bool(flag) => flag,
        serde_json::Value::Number(number) => number.as_u64().unwrap_or(0) != 0,
        serde_json::Value::String(text) => {
            matches!(
                text.trim().to_ascii_lowercase().as_str(),
                "true" | "yes" | "1"
            )
        }
        _ => false,
    })
}

/// Where `Ctrl+Left` / `Ctrl+Right` land in the prompt, the same way as the editor:
/// skip whitespace (line breaks included), then one run of word characters or one
/// run of punctuation. Positions are char indices.
fn word_stop(text: &str, cursor: usize, right: bool) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Space,
        Word,
        Mark,
    }
    let mut spans = Vec::new();
    let mut at = 0;
    for grapheme in text.graphemes(true) {
        let c = grapheme.chars().next().unwrap_or(' ');
        let class = if c.is_whitespace() {
            Class::Space
        } else if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else {
            Class::Mark
        };
        let len = grapheme.chars().count();
        spans.push((at, at + len, class));
        at += len;
    }
    if right {
        let mut i = spans.iter().take_while(|s| s.0 < cursor).count();
        while i < spans.len() && spans[i].2 == Class::Space {
            i += 1;
        }
        if let Some(&(_, _, class)) = spans.get(i) {
            while i < spans.len() && spans[i].2 == class {
                i += 1;
            }
        }
        if i == 0 {
            cursor
        } else {
            spans[i - 1].1.max(cursor)
        }
    } else {
        let mut i = spans.iter().take_while(|s| s.1 <= cursor).count();
        while i > 0 && spans[i - 1].2 == Class::Space {
            i -= 1;
        }
        if i > 0 {
            let class = spans[i - 1].2;
            while i > 0 && spans[i - 1].2 == class {
                i -= 1;
            }
        }
        spans.get(i).map_or(0, |span| span.0).min(cursor)
    }
}

fn prompt_row_for(rows: &[(usize, usize)], cursor: usize, len: usize) -> usize {
    let cursor = cursor.min(len);
    rows.iter()
        .rposition(|(start, end)| {
            if cursor == len {
                *end == len || *start == len
            } else {
                cursor >= *start && cursor < *end || (*start == *end && cursor == *start)
            }
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::settings::{AiPrefs, AiProvider};

    #[test]
    fn local_providers_keep_the_tuning_they_had_before() {
        for provider in [AiProvider::Ollama, AiProvider::LmStudio] {
            let mut ai = AiPrefs::default();
            ai.switch_provider(provider);
            let tuning = ai_tuning(&ai);
            assert_eq!(tuning.max_tokens, Some(16384), "{provider:?}");
            assert!(tuning.headers.is_empty(), "{provider:?}");
            // Same as the worker's client-wide limit, so nothing changes by default.
            assert_eq!(
                tuning.timeout,
                Some(std::time::Duration::from_secs(60 * 30))
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        App, MEMORY_BUDGET, budget_memory, compact_plan, estimate_message_tokens, trim_memory,
    };
    use crate::agent::{AgentCommand, AgentEvent};
    use crate::buffer::Buffer;
    use serde_json::{Value, json};

    fn user(text: &str) -> Value {
        json!({"role": "user", "content": text})
    }

    fn assistant(text: &str) -> Value {
        json!({"role": "assistant", "content": text})
    }

    fn call(id: &str) -> Value {
        json!({"role": "assistant", "content": null, "tool_calls": [{
            "id": id, "type": "function",
            "function": {"name": "read_file", "arguments": "{\"path\":\"a\"}"}
        }]})
    }

    fn result(id: &str, text: &str) -> Value {
        json!({"role": "tool", "tool_call_id": id, "content": text})
    }

    /// `turns` turns of user, tool call, tool result, answer; each about 300 tokens.
    fn long_memory(turns: usize) -> Vec<Value> {
        let filler = "x".repeat(900);
        let mut memory = Vec::new();
        for turn in 0..turns {
            memory.push(user(&format!("question {turn} {filler}")));
            memory.push(call(&format!("c{turn}")));
            memory.push(result(&format!("c{turn}"), "ok"));
            memory.push(assistant(&format!("answer {turn}")));
        }
        memory.push(user("current"));
        memory
    }

    #[test]
    fn nothing_is_folded_while_the_request_fits() {
        let memory = long_memory(3);
        let (history, split) = compact_plan(&memory, 16384, 15, 15_000, "current", None);
        assert_eq!(split, 0);
        assert_eq!(
            history.len(),
            memory.len() - 1,
            "the current prompt is sent apart"
        );
    }

    #[test]
    fn past_the_budget_older_turns_fold_at_a_user_message() {
        let memory = long_memory(40);
        let (history, split) = compact_plan(&memory, 8192, 15, 2_000, "current", None);
        assert!(split > 0);
        assert_eq!(
            memory[split]["role"], "user",
            "the cut lands on a turn start"
        );
        assert_eq!(history[0], memory[split]);
        assert_eq!(history.len(), memory.len() - 1 - split);
        let kept: usize = history.iter().map(estimate_message_tokens).sum();
        assert!(kept <= 2_000, "{kept}");
        // Every tool result still follows its call.
        for (index, message) in history.iter().enumerate() {
            if message["role"] == "tool" {
                assert!(history[index - 1].get("tool_calls").is_some());
            }
        }
    }

    #[test]
    fn keep_is_capped_at_half_the_budget_for_a_small_window() {
        let memory = long_memory(40);
        let (history, split) = compact_plan(&memory, 4096, 15, 200_000, "current", None);
        assert!(split > 0);
        let kept: usize = history.iter().map(estimate_message_tokens).sum();
        assert!(kept <= 4096 / 2, "{kept}");
    }

    #[test]
    fn a_summary_counts_against_the_budget() {
        let memory = long_memory(8);
        let (_, without) = compact_plan(&memory, 4096, 15, 1_000, "current", None);
        let summary = "s".repeat(6_000);
        let (_, with) = compact_plan(&memory, 4096, 15, 1_000, "current", Some(&summary));
        assert!(with >= without);
        assert!(with > 0);
    }

    #[test]
    fn auto_compact_off_keeps_the_old_history_selection() {
        let mut memory = long_memory(6);
        trim_memory(&mut memory, MEMORY_BUDGET);
        let history = budget_memory(&memory, 16384, "current");
        assert!(super::memory_chars(&history) <= MEMORY_BUDGET);
        assert!(history.len() <= 16);
    }

    fn app_with_worker() -> (App, tokio::sync::mpsc::UnboundedReceiver<AgentCommand>) {
        let mut app = App::new(Buffer::new(), None);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        app.agent_tx = Some(tx);
        app.ai.model = "m".into();
        app.agent.open = true;
        app.agent.focused = true;
        (app, rx)
    }

    #[test]
    fn a_long_chat_sends_its_older_turns_to_be_folded() {
        let (mut app, mut rx) = app_with_worker();
        app.ai.context_size = "4096".into();
        app.agent.memory = long_memory(30);
        app.agent.memory.pop();
        app.agent.prompt = "go on".into();
        app.submit_agent();
        let Ok(AgentCommand::Turn {
            compact, history, ..
        }) = rx.try_recv()
        else {
            panic!("no turn sent");
        };
        let older = compact.expect("older turns folded");
        assert!(!older.is_empty());
        assert_eq!(older[0]["role"], "user");
        assert_eq!(
            history.first().map(|m| m["role"].clone()),
            Some(json!("user"))
        );
        assert_eq!(app.agent.status, "compacting");
        // App's memory already dropped what went to the summary.
        assert_eq!(app.agent.memory.len(), history.len() + 1);
    }

    #[test]
    fn with_auto_compact_off_nothing_is_folded() {
        let (mut app, mut rx) = app_with_worker();
        app.ai.auto_compact = false;
        app.ai.context_size = "4096".into();
        app.agent.memory = long_memory(30);
        app.agent.memory.pop();
        app.agent.prompt = "go on".into();
        app.submit_agent();
        let Ok(AgentCommand::Turn {
            compact, history, ..
        }) = rx.try_recv()
        else {
            panic!("no turn sent");
        };
        assert!(compact.is_none());
        assert!(super::memory_chars(&history) <= MEMORY_BUDGET);
    }

    #[test]
    fn a_summary_is_kept_and_sent_with_the_next_turn() {
        let (mut app, mut rx) = app_with_worker();
        app.agent.turn = 3;
        app.agent.busy = true;
        app.handle_agent(AgentEvent::Compacted {
            id: 3,
            summary: Ok("Earlier: renamed foo.".into()),
            dropped: 5,
        });
        assert_eq!(app.agent.summary.as_deref(), Some("Earlier: renamed foo."));
        assert!(app.agent.busy, "auto compact does not end the turn");
        let line = &app.agent.transcript.last().unwrap().text;
        assert!(line.contains("Compacted 5 older messages"), "{line}");
        app.handle_agent(AgentEvent::Reply {
            id: 3,
            text: "done".into(),
        });
        app.agent.prompt = "more".into();
        app.submit_agent();
        let Ok(AgentCommand::Turn { summary, .. }) = rx.try_recv() else {
            panic!("no turn sent");
        };
        assert_eq!(summary.as_deref(), Some("Earlier: renamed foo."));
        app.agent.busy = false;
        app.clear_agent_chat();
        assert_eq!(app.agent.summary, None);
    }

    #[test]
    fn a_failed_summary_says_the_messages_were_dropped() {
        let (mut app, _rx) = app_with_worker();
        app.agent.turn = 2;
        app.handle_agent(AgentEvent::Compacted {
            id: 2,
            summary: Err("Server error 500".into()),
            dropped: 4,
        });
        assert_eq!(app.agent.summary, None);
        let line = &app.agent.transcript.last().unwrap().text;
        assert!(line.contains("dropped 4 older messages"), "{line}");
    }

    #[test]
    fn slash_compact_asks_the_worker_for_a_summary() {
        let (mut app, mut rx) = app_with_worker();
        app.agent.memory = long_memory(3);
        app.agent.memory.push(assistant("last answer"));
        app.agent.prompt = "/compact".into();
        app.submit_agent();
        let Ok(AgentCommand::Compact { older, id, .. }) = rx.try_recv() else {
            panic!("no compact sent");
        };
        assert_eq!(older.len(), 12);
        assert_eq!(
            app.agent.memory,
            vec![user("current"), assistant("last answer")]
        );
        assert!(app.agent.busy);
        app.handle_agent(AgentEvent::Compacted {
            id,
            summary: Ok("sum".into()),
            dropped: older.len(),
        });
        assert!(!app.agent.busy);
        assert_eq!(app.agent.status, "ready");
        assert_eq!(app.agent.summary.as_deref(), Some("sum"));
    }

    #[test]
    fn slash_compact_without_a_model_truncates() {
        let (mut app, mut rx) = app_with_worker();
        app.ai.model.clear();
        app.agent.memory = long_memory(2);
        app.agent.prompt = "/compact".into();
        app.submit_agent();
        assert!(rx.try_recv().is_err());
        assert_eq!(app.agent.memory, vec![user("current")]);
        let line = &app.agent.transcript.last().unwrap().text;
        assert!(line.contains("no model to summarize"), "{line}");
    }

    #[test]
    fn the_gauge_prefers_what_the_server_reported() {
        let (mut app, _rx) = app_with_worker();
        app.ai.context_size = "32768".into();
        let (estimate, limit) = app.context_gauge();
        assert!(estimate > 0, "an estimate before the first turn");
        assert_eq!(limit, Some(32768));
        app.agent.turn = 1;
        app.handle_agent(AgentEvent::Usage {
            id: 1,
            tokens: 12_400,
        });
        assert_eq!(app.context_gauge(), (12_400, Some(32768)));
    }

    #[test]
    fn an_unknown_window_shows_no_limit() {
        let (mut app, _rx) = app_with_worker();
        app.ai
            .switch_provider(crate::settings::AiProvider::OpenRouter);
        app.ai.context_size = "auto".into();
        assert_eq!(app.context_gauge().1, None);
        app.ai.switch_provider(crate::settings::AiProvider::Ollama);
        app.ai.context_size = "auto".into();
        assert_eq!(app.context_gauge().1, Some(16384));
    }
}

#[cfg(test)]
mod editor_first_tests {
    use super::{App, clear_old_tool_results, symbol_column};
    use crate::action::{Focus, SemanticAction};
    use crate::agent::{AgentCommand, AgentEvent, AgentMode};
    use crate::buffer::Buffer;
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};
    use tokio::sync::mpsc::UnboundedReceiver;

    fn workspace(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "duckx-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// `a.txt` is open in the editor and has focus. The assistant panel is open
    /// but not focused, the way it is while the user types.
    fn typing_app(root: &Path, text: &str) -> (App, UnboundedReceiver<AgentCommand>) {
        let file = root.join("a.txt");
        std::fs::write(&file, text).unwrap();
        let mut app = App::new(Buffer::from_text(text), Some(file));
        app.explorer = crate::explorer::Explorer::new(root.to_path_buf());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        app.attach_agent(tx);
        app.ai.model = "m".into();
        app.ai.auto_approve = true;
        app.agent.open = true;
        app.agent.focused = false;
        app.agent.busy = true;
        app.agent.turn = 1;
        (app, rx)
    }

    fn tool(app: &mut App, name: &str, args: Value) {
        app.agent.busy = true;
        app.handle_agent(AgentEvent::Tool {
            id: 1,
            call_id: "c".into(),
            name: name.into(),
            arguments: args.to_string(),
        });
    }

    fn reply(rx: &mut UnboundedReceiver<AgentCommand>) -> String {
        let Ok(AgentCommand::ToolResult { content, .. }) = rx.try_recv() else {
            panic!("no tool result");
        };
        content
    }

    fn run_jobs(app: &mut App) {
        for job in app.take_jobs() {
            let event = crate::jobs::run(job);
            app.handle_event(event);
        }
    }

    fn slash(app: &mut App, text: &str) {
        app.agent.busy = false;
        app.agent.prompt = text.into();
        app.submit_agent();
    }

    #[test]
    fn an_edit_keeps_the_users_cursor_and_focus() {
        let root = workspace("keep-cursor");
        let (mut app, mut rx) = typing_app(&root, "one\ntwo\nthree\n");
        app.buffer_mut().move_to(10, false);
        tool(
            &mut app,
            "apply_edit",
            json!({"path": "a.txt", "find": "one", "replace": "first"}),
        );
        let content = reply(&mut rx);
        assert!(content.contains("unsaved"), "{content}");
        assert!(
            content.contains("first"),
            "the edit echoes its lines: {content}"
        );
        assert_eq!(app.buffer().rope().to_string(), "first\ntwo\nthree\n");
        assert_eq!(
            app.buffer().selection().head,
            12,
            "the cursor stays on `three`"
        );
        assert_eq!(app.focus(), Focus::Editor);
        assert!(!app.agent.focused);
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "one\ntwo\nthree\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_line_edit_needs_a_fresh_read() {
        let root = workspace("stale-read");
        let (mut app, mut rx) = typing_app(&root, "one\ntwo\n");
        let edit = json!({"path": "a.txt", "start_line": 1, "end_line": 1, "text": "uno"});
        tool(&mut app, "apply_edit", edit.clone());
        assert!(reply(&mut rx).contains("read_file"));

        tool(&mut app, "read_file", json!({"path": "a.txt"}));
        reply(&mut rx);
        tool(&mut app, "apply_edit", edit);
        reply(&mut rx);
        assert_eq!(app.buffer().rope().to_string(), "uno\ntwo\n");

        app.handle(SemanticAction::InsertChar('!'));
        tool(
            &mut app,
            "apply_edit",
            json!({"path": "a.txt", "start_line": 2, "end_line": 2, "text": "dos"}),
        );
        let content = reply(&mut rx);
        assert!(content.contains("changed since you read it"), "{content}");
        assert!(app.buffer().rope().to_string().contains("two"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_edit_to_a_closed_file_goes_to_a_background_tab() {
        let root = workspace("background-edit");
        let (mut app, mut rx) = typing_app(&root, "alpha\n");
        let other = root.join("b.txt");
        std::fs::write(&other, "beta\n").unwrap();
        tool(
            &mut app,
            "apply_edit",
            json!({"path": "b.txt", "find": "beta", "replace": "BETA"}),
        );
        assert!(
            rx.try_recv().is_err(),
            "the reply waits for the file to load"
        );
        run_jobs(&mut app);
        let content = reply(&mut rx);
        assert!(content.contains("unsaved"), "{content}");
        assert_eq!(
            app.path(),
            Some(root.join("a.txt").as_path()),
            "the visible tab stays"
        );
        assert_eq!(app.focus(), Focus::Editor);
        let tab = app
            .tab_with_path(&other)
            .and_then(|id| app.find_tab(id))
            .unwrap();
        assert_eq!(tab.buffer.rope().to_string(), "BETA\n");
        assert!(tab.dirty);
        assert_eq!(
            std::fs::read_to_string(&other).unwrap(),
            "beta\n",
            "nothing is written"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_and_ask_modes_refuse_edits_and_toggle_off() {
        let root = workspace("modes");
        let (mut app, mut rx) = typing_app(&root, "one\n");
        slash(&mut app, "/plan");
        assert_eq!(app.agent.mode, AgentMode::Plan);
        tool(
            &mut app,
            "apply_edit",
            json!({"path": "a.txt", "find": "one", "replace": "1"}),
        );
        assert!(reply(&mut rx).contains("not available in plan mode"));
        assert_eq!(app.buffer().rope().to_string(), "one\n");

        slash(&mut app, "/ask");
        assert_eq!(app.agent.mode, AgentMode::Ask);
        slash(&mut app, "/ask");
        assert_eq!(
            app.agent.mode,
            AgentMode::Agent,
            "the same mode again turns it off"
        );
        tool(
            &mut app,
            "apply_edit",
            json!({"path": "a.txt", "find": "one", "replace": "1"}),
        );
        reply(&mut rx);
        assert_eq!(app.buffer().rope().to_string(), "1\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_approval_waits_without_taking_focus() {
        let root = workspace("approval-focus");
        let (mut app, mut rx) = typing_app(&root, "one\n");
        app.ai.auto_approve = false;
        tool(
            &mut app,
            "apply_edit",
            json!({"path": "a.txt", "find": "one", "replace": "1"}),
        );
        assert!(app.agent.pending.is_some());
        assert!(rx.try_recv().is_err());
        assert!(!app.agent.focused);
        assert_eq!(app.focus(), Focus::Editor);
        assert!(
            app.message
                .as_deref()
                .is_some_and(|text| text.contains("F6"))
        );
        app.handle(SemanticAction::InsertChar('y'));
        assert_eq!(
            app.buffer().rope().to_string(),
            "yone\n",
            "typing still goes to the file"
        );
        assert!(app.agent.pending.is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_file_and_goto_do_not_move_the_view_while_the_user_types() {
        let root = workspace("open-behind");
        let (mut app, mut rx) = typing_app(&root, "one\ntwo\n");
        std::fs::write(root.join("b.txt"), "beta\n").unwrap();
        tool(&mut app, "open_file", json!({"path": "b.txt"}));
        assert!(reply(&mut rx).contains("background"));
        assert_eq!(app.path(), Some(root.join("a.txt").as_path()));

        tool(&mut app, "goto", json!({"line": 2}));
        assert!(reply(&mut rx).contains("cursor stays"));
        assert_eq!(app.buffer().selection().head, 0);

        app.agent.focused = true;
        tool(&mut app, "goto", json!({"line": 2}));
        reply(&mut rx);
        assert_eq!(
            app.buffer().selection().head,
            4,
            "in the panel, goto moves the cursor"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn slash_changes_lists_the_edits_of_this_chat() {
        let root = workspace("changes");
        let (mut app, mut rx) = typing_app(&root, "one\ntwo\n");
        slash(&mut app, "/changes");
        assert!(
            app.agent
                .transcript
                .last()
                .unwrap()
                .text
                .contains("has not changed")
        );
        tool(
            &mut app,
            "apply_edit",
            json!({"path": "a.txt", "find": "two", "replace": "2"}),
        );
        reply(&mut rx);
        slash(&mut app, "/changes");
        let text = &app.agent.transcript.last().unwrap().text;
        assert!(text.contains("1. a.txt:2"), "{text}");
        assert!(text.contains("unsaved"), "{text}");
        app.agent.busy = false;
        app.clear_agent_chat();
        assert!(app.agent.changes.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_newest_tool_results_stay_whole() {
        let long = "x".repeat(5000);
        let mut history: Vec<Value> = (0..3)
            .map(|i| json!({"role": "tool", "tool_call_id": format!("c{i}"), "content": long}))
            .collect();
        clear_old_tool_results(&mut history, 2);
        let first = history[0]["content"].as_str().unwrap();
        assert!(first.ends_with("[older tool result cleared]"), "{first}");
        assert!(first.len() < long.len());
        assert_eq!(history[1]["content"].as_str(), Some(long.as_str()));
        assert_eq!(history[2]["content"].as_str(), Some(long.as_str()));
    }

    #[test]
    fn symbol_column_prefers_a_whole_word() {
        assert_eq!(symbol_column("let foobar = foo();", "foo"), Some(13));
        assert_eq!(symbol_column("ไทย foo", "foo"), Some(4));
        assert_eq!(symbol_column("foobar", "foo"), Some(0));
        assert_eq!(symbol_column("bar", "foo"), None);
    }

    #[test]
    fn esc_hides_the_panel_and_only_stop_ends_the_run() {
        let root = workspace("esc-stop");
        let (mut app, mut rx) = typing_app(&root, "one\n");
        app.agent.focused = true;
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Close));
        assert!(app.agent.busy, "Esc does not stop the turn");
        assert!(!app.agent.focused);
        assert!(rx.try_recv().is_err(), "no cancel was sent");

        app.agent.focused = true;
        app.handle(SemanticAction::Agent(crate::action::AgentAction::NewChat));
        assert!(app.agent.busy, "Ctrl+L does not stop a running turn");
        assert!(rx.try_recv().is_err());
        assert!(
            app.agent.transcript.last().unwrap().text.contains("/stop"),
            "{}",
            app.agent.transcript.last().unwrap().text
        );

        app.agent.prompt = "/stop".into();
        app.submit_agent();
        assert!(!app.agent.busy);
        assert_eq!(app.agent.status, "ready");
        match rx.try_recv() {
            Ok(AgentCommand::Cancel { id }) => assert_eq!(id, 1),
            other => panic!("expected a cancel, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_reply_while_the_panel_is_hidden_shows_a_note() {
        let root = workspace("snack");
        let (mut app, _rx) = typing_app(&root, "one\n");
        app.agent.focused = false;
        app.handle_agent(AgentEvent::Reply {
            id: 1,
            text: "Sorted the list.\nTry it.".into(),
        });
        let note = app.snackbar.as_ref().expect("a note");
        assert_eq!(note.text, "Sorted the list. Try it.");
        assert_eq!(note.kind, super::super::NoticeKind::Info);
        assert!(note.assistant);

        app.focus_agent();
        assert!(app.snackbar.is_none(), "opening the panel clears the note");

        app.agent.focused = true;
        app.agent.busy = true;
        app.agent.turn = 2;
        app.handle_agent(AgentEvent::Reply {
            id: 2,
            text: "seen".into(),
        });
        assert!(app.snackbar.is_none(), "an open panel needs no note");

        app.agent.focused = false;
        app.agent.busy = true;
        app.agent.turn = 3;
        app.handle_agent(AgentEvent::Failed {
            id: 3,
            text: "Server error".into(),
        });
        assert!(
            app.snackbar
                .as_ref()
                .is_some_and(|note| note.kind == super::super::NoticeKind::Error)
        );
        app.snackbar.as_mut().unwrap().until = std::time::Instant::now();
        app.expire_snackbar();
        assert!(app.snackbar.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
