//! Modal editing in the app: the mode, its commands, and keeping the badge true.
//!
//! Everything here is inert while Settings → Editor → Modeless is on (the default):
//! [`App::modal`] is `None`, keys never reach [`crate::modal::route`], and the hooks
//! below return at once. See `src/modal.rs` for the keys.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::App;
use crate::action::{Focus, SemanticAction};
use crate::buffer::Movement;
use crate::command::CommandId;
use crate::modal::{EditMode, KeyList, KeyListAction, LineSelect, ModalAction, ModalState};
use crate::settings::StartModePref;

/// What [`App::modal_after`] needs to know about the state before an action.
pub(super) struct ModalBefore {
    /// `Pend` and `Count` keep the started sequence; everything else ends it.
    keeps_prefix: bool,
    /// The action set the hint row notice itself.
    keeps_notice: bool,
    /// A search or an undo leaves text selected; in Normal the cursor just goes there.
    collapse: bool,
    /// A Visual Line move already shaped the selection.
    line_motion: bool,
    focus: Focus,
    active: crate::layout::EditorId,
}

impl ModalBefore {
    pub(super) fn set_line_motion(&mut self, done: bool) {
        self.line_motion = done;
    }

    pub(super) fn line_motion(&self) -> bool {
        self.line_motion
    }
}

impl App {
    /// The modal state while Modeless is off; `None` while it is on.
    pub fn modal(&self) -> Option<&ModalState> {
        (!self.editor_prefs.modeless).then_some(&self.modal_state)
    }

    /// The modal state when keys should go to [`crate::modal::route`]: Modeless off,
    /// an editor focused (not a preview), and no PANE mode.
    pub fn modal_input(&self) -> Option<&ModalState> {
        let state = self.modal()?;
        (self.focus() == Focus::Editor && !self.preview_focused() && self.pane_select.is_none())
            .then_some(state)
    }

    /// The mode color for the badge, the cursor, and the cursor line number, while the
    /// editor has the keys.
    pub fn mode_color(&self) -> Option<ratatui::style::Color> {
        let state = self.modal_input()?;
        let theme = self.styles();
        Some(match state.mode {
            EditMode::Normal => theme.status_a_bg,
            EditMode::Insert => theme.status_insert_bg,
            EditMode::Visual { .. } => theme.status_visual_bg,
        })
    }

    /// True when the hint row is drawn above the status bar.
    pub fn mode_hints_shown(&self) -> bool {
        self.editor_prefs.mode_hints && self.modal_input().is_some()
    }

    /// Follows Settings → Editor → Modeless: offer or hide the mode commands, and
    /// start in the Start mode whenever Modeless turns off.
    pub(super) fn sync_modal(&mut self) {
        let on = !self.editor_prefs.modeless;
        crate::command::show_modal_commands(on);
        if on == self.modal_applied {
            return;
        }
        self.modal_applied = on;
        self.modal_state = ModalState {
            mode: start_mode(self.editor_prefs.start_mode),
            ..ModalState::default()
        };
    }

    /// `modeless`: flip Settings → Editor → Modeless and save it.
    pub(super) fn toggle_modeless(&mut self) {
        self.editor_prefs.modeless = !self.editor_prefs.modeless;
        self.queue_settings_save();
        self.sync_modal();
        self.message = Some(if self.editor_prefs.modeless {
            "Modeless on: type anywhere".to_string()
        } else {
            match self.modal_state.mode {
                EditMode::Insert => "Modeless off: INSERT. Esc for Normal".to_string(),
                _ => "Modeless off: NORMAL. Press i to type, ? for keys".to_string(),
            }
        });
    }

    /// `mode-hints`: flip Settings → Editor → Mode hints and save it.
    pub(super) fn toggle_mode_hints(&mut self) {
        self.editor_prefs.mode_hints = !self.editor_prefs.mode_hints;
        self.queue_settings_save();
        self.message = Some(format!(
            "Mode hints {}",
            if self.editor_prefs.mode_hints {
                "on"
            } else {
                "off"
            }
        ));
    }

    pub(super) fn modal_before(&self, action: &SemanticAction) -> Option<ModalBefore> {
        self.modal()?;
        Some(ModalBefore {
            keeps_prefix: matches!(
                action,
                SemanticAction::Modal(ModalAction::Pend(_) | ModalAction::Count(_))
            ),
            keeps_notice: matches!(action, SemanticAction::Modal(ModalAction::Unbound(_))),
            collapse: matches!(
                action,
                SemanticAction::FindNext
                    | SemanticAction::FindPrevious
                    | SemanticAction::Undo
                    | SemanticAction::Redo
                    | SemanticAction::Modal(ModalAction::Run(
                        CommandId::FindNext
                            | CommandId::FindPrevious
                            | CommandId::Undo
                            | CommandId::Redo
                    ))
            ) || matches!(
                action,
                SemanticAction::Modal(ModalAction::Repeat(_, inner))
                    if matches!(**inner, SemanticAction::Modal(ModalAction::Run(
                        CommandId::FindNext | CommandId::FindPrevious | CommandId::Undo | CommandId::Redo
                    )))
            ),
            line_motion: false,
            focus: self.focus(),
            active: self.active_id(),
        })
    }

    /// After every action while Modeless is off: end a finished sequence, and keep the
    /// badge true to what is on screen (a selection in Normal is Visual).
    pub(super) fn modal_after(&mut self, before: Option<ModalBefore>) {
        self.sync_modal();
        let Some(before) = before else {
            return;
        };
        if self.editor_prefs.modeless {
            return;
        }
        if !before.keeps_prefix {
            self.modal_state.pending = None;
            self.modal_state.count = None;
        }
        if !before.keeps_notice {
            self.modal_state.notice = None;
        }
        if self.active_id() != before.active && self.modal_state.mode.is_visual() {
            self.leave_visual(false);
        }
        if self.focus() != Focus::Editor || self.preview_focused() {
            return;
        }
        let selection = self.buffer().selection();
        match self.modal_state.mode {
            EditMode::Normal if !selection.is_empty() => {
                let from_search = matches!(before.focus, Focus::Find | Focus::Replace);
                if before.collapse || from_search {
                    self.buffer_mut().move_to(selection.from(), false);
                    self.buffer_mut().deselect();
                } else {
                    self.modal_state.mode = EditMode::Visual { line: false };
                }
            }
            EditMode::Visual { line: true } if !before.line_motion => {
                // An edit (indent, comment) moved the text: follow the cursor and snap again.
                if let Some(line) = self.modal_state.line {
                    let now = self.buffer().selection();
                    self.snap_visual_line();
                    if self.buffer().selection() == now {
                        // Still the lines it showed (a counted move, a key list run).
                        return;
                    }
                    self.buffer_mut().select_range(now.anchor, now.head);
                    let head = now.head;
                    let anchor_line = line
                        .anchor_line
                        .min(self.buffer().len_lines().saturating_sub(1));
                    self.modal_state.line = Some(LineSelect {
                        anchor_line,
                        cursor: head,
                    });
                    self.snap_visual_line();
                }
            }
            _ => {}
        }
    }

    /// The handle step for one action while Modeless is off: a Visual Line move
    /// moves the line cursor, everything else is the usual handler.
    pub(super) fn modal_step(&mut self, action: SemanticAction) {
        if !self.visual_line_motion(&action) {
            self.handle_inner(action);
        }
    }

    /// A move in Visual Line mode moves the cursor line and selects whole lines from
    /// the line the selection started on. Returns false for anything else.
    pub(super) fn visual_line_motion(&mut self, action: &SemanticAction) -> bool {
        let Some(line) = self.modal_state.line else {
            return false;
        };
        if self.editor_prefs.modeless
            || self.modal_state.mode != (EditMode::Visual { line: true })
            || self.focus() != Focus::Editor
        {
            return false;
        }
        let plain = match action {
            SemanticAction::Move { movement, .. } => SemanticAction::Move {
                movement: *movement,
                extend: false,
            },
            SemanticAction::PageUp { .. } => SemanticAction::PageUp { extend: false },
            SemanticAction::PageDown { .. } => SemanticAction::PageDown { extend: false },
            _ => return false,
        };
        self.buffer_mut().select_range(line.cursor, line.cursor);
        self.handle_inner(plain);
        let cursor = self.buffer().selection().head;
        self.modal_state.line = Some(LineSelect { cursor, ..line });
        self.snap_visual_line();
        true
    }

    /// Selects every line from the anchor line to the cursor line. The cursor sits at
    /// the end of its line going down and at the start going up, so it stays on it.
    fn snap_visual_line(&mut self) {
        let Some(line) = self.modal_state.line else {
            return;
        };
        let buffer = self.buffer();
        let len = buffer.rope().len_chars();
        let cursor_line = buffer.rope().char_to_line(line.cursor.min(len));
        let (anchor_start, anchor_end, _) = buffer.line_bounds(line.anchor_line);
        let (cursor_start, cursor_end, _) = buffer.line_bounds(cursor_line);
        let (anchor, head) = if cursor_line >= line.anchor_line {
            (anchor_start, cursor_end)
        } else {
            (anchor_end, cursor_start)
        };
        self.buffer_mut().select_range(anchor, head);
    }

    /// Before copy or cut in Visual Line: take in the line break after the last line,
    /// so a paste brings back whole lines.
    fn whole_lines_for_clipboard(&mut self) {
        if self.modal_state.mode != (EditMode::Visual { line: true }) {
            return;
        }
        let selection = self.buffer().selection();
        let rope = self.buffer().rope();
        let first = rope.char_to_line(selection.from());
        let last = rope.char_to_line(selection.to());
        let (start, _, _) = self.buffer().line_bounds(first);
        let (_, _, next) = self.buffer().line_bounds(last);
        self.buffer_mut().select_range(start, next);
    }

    fn enter_visual(&mut self, line: bool) {
        let selection = self.buffer().selection();
        if line {
            let anchor_line = self.buffer().rope().char_to_line(selection.anchor);
            self.modal_state.line = Some(LineSelect {
                anchor_line,
                cursor: selection.head,
            });
            self.modal_state.mode = EditMode::Visual { line: true };
            self.snap_visual_line();
        } else {
            self.modal_state.line = None;
            self.modal_state.mode = EditMode::Visual { line: false };
        }
    }

    /// Back to Normal from Visual. `keep_cursor_at_start` puts the cursor where the
    /// selection began, the way a copy leaves it.
    fn leave_visual(&mut self, keep_cursor_at_start: bool) {
        if keep_cursor_at_start {
            let from = self.buffer().selection().from();
            self.buffer_mut().move_to(from, false);
        } else if let Some(line) = self.modal_state.line {
            // Visual Line shows whole lines; the cursor goes back to its own column.
            let len = self.buffer().rope().len_chars();
            self.buffer_mut().move_to(line.cursor.min(len), false);
        }
        self.buffer_mut().deselect();
        self.modal_state.line = None;
        self.modal_state.mode = EditMode::Normal;
    }

    fn enter_insert(&mut self) {
        if self.modal_state.mode.is_visual() {
            self.buffer_mut().deselect();
        }
        self.modal_state.line = None;
        self.modal_state.mode = EditMode::Insert;
    }

    fn enter_normal(&mut self) {
        match self.modal_state.mode {
            EditMode::Visual { .. } => self.leave_visual(false),
            EditMode::Insert => {
                self.buffer_mut().seal_undo();
                self.modal_state.mode = EditMode::Normal;
            }
            EditMode::Normal => {}
        }
    }

    fn move_cursor(&mut self, movement: Movement) {
        self.handle_inner(SemanticAction::Move {
            movement,
            extend: false,
        });
    }

    pub(super) fn handle_modal(&mut self, action: ModalAction) {
        if self.editor_prefs.modeless {
            return;
        }
        match action {
            ModalAction::Run(id) => self.run_modal_command(id),
            ModalAction::Repeat(count, action) => {
                for _ in 0..count.min(crate::modal::MAX_COUNT) {
                    self.modal_step((*action).clone());
                }
            }
            ModalAction::Pend(first) => self.modal_state.pending = Some(first),
            ModalAction::Count(digit) => {
                let count = self.modal_state.count.unwrap_or(0);
                self.modal_state.count =
                    Some((count * 10 + u32::from(digit)).min(crate::modal::MAX_COUNT));
            }
            ModalAction::Cancel => {}
            ModalAction::Escape => self.modal_escape(),
            ModalAction::Unbound(keys) => {
                self.modal_state.notice = Some(match self.modal_state.mode {
                    EditMode::Normal => {
                        format!("No command on {keys}. Press i to type, ? for keys")
                    }
                    _ => format!("No command on {keys}. Esc for Normal, ? for keys"),
                });
            }
            ModalAction::GotoLine(line) => {
                let target = self.buffer().line_bounds(line.saturating_sub(1)).0;
                match self.modal_state.mode {
                    EditMode::Visual { line: true } => {
                        if let Some(select) = self.modal_state.line {
                            self.modal_state.line = Some(LineSelect {
                                cursor: target,
                                ..select
                            });
                            self.snap_visual_line();
                        }
                    }
                    EditMode::Visual { line: false } => self.buffer_mut().move_to(target, true),
                    _ => self.buffer_mut().goto_line(line.saturating_sub(1)),
                }
            }
            ModalAction::EnterInsertThen(action) => {
                self.enter_insert();
                self.handle_inner(*action);
            }
            ModalAction::ThenNormal(action) => {
                if matches!(*action, SemanticAction::Copy | SemanticAction::Cut) {
                    self.whole_lines_for_clipboard();
                }
                let copy = *action == SemanticAction::Copy;
                self.handle_inner(*action);
                if !copy {
                    self.modal_state.line = None;
                }
                if self.modal_state.mode.is_visual() {
                    self.leave_visual(copy);
                }
            }
            ModalAction::KeyList(action) => self.handle_key_list(action),
        }
    }

    /// `Esc` in Insert or Visual: a popup or snippet closes first; then back to Normal.
    /// Either way this `Esc` is not the first of a double `Esc`.
    fn modal_escape(&mut self) {
        let closes = self.nav.snippet.is_some()
            || self.nav.hover.is_some()
            || self.completion_view().is_some()
            || self.action_view().is_some()
            || self.reference_view().is_some();
        if closes {
            self.clear_snippet();
            self.dismiss_lsp_popups();
            return;
        }
        self.enter_normal();
    }

    /// A command from a mode key or the Command Bar, the way that mode means it.
    pub(super) fn run_modal_command(&mut self, id: CommandId) {
        if self.editor_prefs.modeless {
            if let Err(error) = self.run_command(id, None) {
                self.message = Some(error);
            }
            return;
        }
        let mode = self.modal_state.mode;
        let result: Result<(), String> = match id {
            CommandId::ModeNormal => {
                self.enter_normal();
                Ok(())
            }
            CommandId::ModeInsert => {
                self.enter_insert();
                Ok(())
            }
            CommandId::ModeVisual => {
                match mode {
                    EditMode::Visual { line: false } => self.leave_visual(false),
                    EditMode::Visual { line: true } => {
                        self.modal_state.line = None;
                        self.modal_state.mode = EditMode::Visual { line: false };
                    }
                    _ => self.enter_visual(false),
                }
                Ok(())
            }
            CommandId::ModeVisualLine => {
                match mode {
                    EditMode::Visual { line: true } => self.leave_visual(false),
                    _ => self.enter_visual(true),
                }
                Ok(())
            }
            CommandId::InsertAfter => {
                self.enter_normal_from_visual();
                let head = self.buffer().selection().head;
                let rope = self.buffer().rope();
                if head < rope.len_chars() && !matches!(rope.char(head), '\n' | '\r') {
                    self.move_cursor(Movement::Right);
                }
                self.enter_insert();
                Ok(())
            }
            CommandId::InsertLineStart => {
                self.enter_normal_from_visual();
                self.move_cursor(Movement::LineStart);
                self.enter_insert();
                Ok(())
            }
            CommandId::InsertLineEnd => {
                self.enter_normal_from_visual();
                self.move_cursor(Movement::LineEnd);
                self.enter_insert();
                Ok(())
            }
            CommandId::OpenLineBelow => {
                self.enter_normal_from_visual();
                self.move_cursor(Movement::LineEnd);
                self.handle_inner(SemanticAction::InsertNewline);
                self.enter_insert();
                Ok(())
            }
            CommandId::OpenLineAbove => {
                self.enter_normal_from_visual();
                let head = self.buffer().selection().head;
                let line = self.buffer().rope().char_to_line(head);
                if line == 0 {
                    self.buffer_mut().select_range(0, 0);
                    self.handle_inner(SemanticAction::InsertNewline);
                    self.move_cursor(Movement::Up);
                } else {
                    let (_, end, _) = self.buffer().line_bounds(line - 1);
                    self.buffer_mut().select_range(end, end);
                    self.handle_inner(SemanticAction::InsertNewline);
                }
                self.enter_insert();
                Ok(())
            }
            CommandId::SwapSelectionEnds => self.swap_selection_ends(),
            CommandId::FindNext if mode == EditMode::Normal => {
                // The cursor sits on the current match; look past it, as `n` does.
                let head = self.buffer().selection().head;
                let len = self.buffer().rope().len_chars();
                self.buffer_mut()
                    .select_range((head + 1).min(len), (head + 1).min(len));
                let result = self.run_command(id, None);
                if self.buffer().selection().is_empty() {
                    self.buffer_mut().select_range(head, head);
                }
                result
            }
            CommandId::ChangeSelection => {
                if !mode.is_visual() || self.buffer().selection().is_empty() {
                    Err("Select text first (v or V), then c".to_string())
                } else {
                    let result = self.run_command(CommandId::Cut, None);
                    self.enter_insert();
                    result
                }
            }
            CommandId::Keys => {
                if mode == EditMode::Insert {
                    self.enter_normal();
                }
                self.modal_state.key_list = Some(KeyList::default());
                Ok(())
            }
            CommandId::Indent => {
                // `>>` and `>` indent whole lines, never insert at the cursor.
                if mode == EditMode::Insert {
                    self.run_command(id, None)
                } else {
                    self.edit(|buffer| buffer.indent_lines());
                    Ok(())
                }
            }
            CommandId::Copy | CommandId::Cut | CommandId::Paste | CommandId::Comment
                if mode.is_visual() =>
            {
                if matches!(id, CommandId::Copy | CommandId::Cut) {
                    self.whole_lines_for_clipboard();
                }
                let result = self.run_command(id, None);
                if id != CommandId::Copy {
                    // The text changed: the remembered Visual Line cursor is stale.
                    self.modal_state.line = None;
                }
                self.leave_visual(id == CommandId::Copy);
                result
            }
            CommandId::SelectBracket
            | CommandId::SelectInsideQuotes
            | CommandId::SelectInsideParens
            | CommandId::SelectInsideBraces
            | CommandId::SelectInsideBrackets => {
                if mode == (EditMode::Visual { line: true }) {
                    self.modal_state.line = None;
                    self.modal_state.mode = EditMode::Visual { line: false };
                }
                self.run_command(id, None)
            }
            _ => self.run_command(id, None),
        };
        if let Err(error) = result {
            self.message = Some(error);
        }
    }

    fn enter_normal_from_visual(&mut self) {
        if self.modal_state.mode.is_visual() {
            self.leave_visual(false);
        }
    }

    fn swap_selection_ends(&mut self) -> Result<(), String> {
        match self.modal_state.mode {
            EditMode::Visual { line: true } => {
                let Some(line) = self.modal_state.line else {
                    return Ok(());
                };
                let rope = self.buffer().rope();
                let cursor_line = rope.char_to_line(line.cursor.min(rope.len_chars()));
                let (start, _, _) = self.buffer().line_bounds(line.anchor_line);
                self.modal_state.line = Some(LineSelect {
                    anchor_line: cursor_line,
                    cursor: start,
                });
                self.snap_visual_line();
                Ok(())
            }
            EditMode::Visual { line: false } => {
                self.buffer_mut().swap_selection_ends();
                Ok(())
            }
            _ => Err("Select text first (v or V), then o".to_string()),
        }
    }

    fn handle_key_list(&mut self, action: KeyListAction) {
        let Some(mut list) = self.modal_state.key_list.take() else {
            return;
        };
        let mode = self.modal_state.mode;
        let rows = crate::modal::key_list(mode, &list.query);
        match action {
            KeyListAction::Prev => {
                list.selected = super::wrap_index(list.selected, rows.len(), -1);
            }
            KeyListAction::Next => {
                list.selected = super::wrap_index(list.selected, rows.len(), 1);
            }
            KeyListAction::Insert(c) => {
                list.query.push(c);
                list.selected = 0;
            }
            KeyListAction::Backspace => {
                list.query.pop();
                list.selected = 0;
            }
            KeyListAction::Close => return,
            KeyListAction::Run => {
                let Some(entry) = rows.get(list.selected).copied() else {
                    return;
                };
                if let Some(action) = crate::modal::action_for(entry, mode, self.keys()) {
                    self.modal_step(action);
                }
                return;
            }
        }
        self.modal_state.key_list = Some(list);
    }

    /// After a mouse event while Modeless is off: a drag that selects is Visual, and a
    /// click that drops the selection is Normal again.
    pub(super) fn modal_after_mouse(&mut self, event: MouseEvent) {
        if self.modal_input().is_none() {
            return;
        }
        let selection = self.buffer().selection();
        let click = event.kind == MouseEventKind::Down(MouseButton::Left);
        match self.modal_state.mode {
            EditMode::Normal if !selection.is_empty() => {
                self.modal_state.mode = EditMode::Visual { line: false };
            }
            EditMode::Visual { .. } if click && selection.is_empty() => {
                self.modal_state.line = None;
                self.modal_state.mode = EditMode::Normal;
            }
            EditMode::Visual { line: true } if !selection.is_empty() && !click => {
                // Dragging selects characters.
                self.modal_state.line = None;
                self.modal_state.mode = EditMode::Visual { line: false };
            }
            _ => {}
        }
    }
}

fn start_mode(pref: StartModePref) -> EditMode {
    match pref {
        StartModePref::Normal => EditMode::Normal,
        StartModePref::Insert => EditMode::Insert,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::buffer::Buffer;

    /// The main loop's routing: modal keys when an editor has them, else the usual router.
    fn send(app: &mut App, event: Event) {
        let action = match app.modal_input() {
            Some(state) => crate::modal::route(&event, state, app.keys()),
            None => crate::input::route_flags(
                &event,
                app.focus(),
                app.explorer.selecting,
                app.filtering(),
                app.keys(),
                app.capturing(),
                app.drafting(),
                &mut crate::agent::PasteBurst::default(),
            ),
        };
        match action {
            Some(action) => app.handle(action),
            None => app.note_unrouted(&event),
        }
    }

    fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        send(app, Event::Key(KeyEvent::new(code, modifiers)));
    }

    fn keys(app: &mut App, text: &str) {
        for c in text.chars() {
            let modifiers = if c.is_ascii_uppercase() {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            press(app, KeyCode::Char(c), modifiers);
        }
    }

    fn esc(app: &mut App) {
        press(app, KeyCode::Esc, KeyModifiers::NONE);
    }

    fn modal_app(text: &str) -> App {
        let mut app = App::new(Buffer::from_text(text), None);
        app.run_command(CommandId::Modeless, None).unwrap();
        assert_eq!(app.modal().map(|state| state.mode), Some(EditMode::Normal));
        app
    }

    fn mode(app: &App) -> EditMode {
        app.modal().expect("Modeless is off").mode
    }

    fn text(app: &App) -> String {
        app.buffer().rope().to_string()
    }

    fn head(app: &App) -> usize {
        app.buffer().selection().head
    }

    #[test]
    fn modeless_on_by_default_keeps_the_old_routing() {
        let mut app = App::new(Buffer::from_text("hello\n"), None);
        assert!(app.editor_prefs.modeless);
        assert!(app.modal().is_none());
        assert!(app.modal_input().is_none());
        assert!(!app.mode_hints_shown());
        // The mode commands are not offered while Modeless is on.
        for name in ["mode-insert", "mode-visual", "keys", "open-line-below"] {
            assert!(
                crate::command::find_command(name, crate::command::CommandContext::Editor, false)
                    .is_none(),
                "{name}"
            );
        }
        assert!(
            crate::command::find_command("modeless", crate::command::CommandContext::Editor, false)
                .is_some()
        );
        assert!(app.run_command(CommandId::ModeInsert, None).is_err());
        // Letters type, Esc Esc opens the Command Bar, exactly as before.
        keys(&mut app, "ij");
        assert_eq!(text(&app), "ijhello\n");
        esc(&mut app);
        esc(&mut app);
        assert_eq!(app.focus(), Focus::CommandBar);
    }

    #[test]
    fn normal_moves_and_never_types_and_i_types() {
        let mut app = modal_app("one two\nthree\n");
        keys(&mut app, "jlq");
        assert_eq!(text(&app), "one two\nthree\n", "Normal never types");
        assert_eq!(head(&app), 9);
        assert!(
            app.modal()
                .unwrap()
                .notice
                .as_deref()
                .unwrap()
                .contains("No command on q")
        );
        keys(&mut app, "i");
        assert_eq!(mode(&app), EditMode::Insert);
        keys(&mut app, "X");
        assert_eq!(text(&app), "one two\ntXhree\n");
        esc(&mut app);
        assert_eq!(mode(&app), EditMode::Normal);
        keys(&mut app, "u");
        assert_eq!(text(&app), "one two\nthree\n");
        keys(&mut app, "U");
        assert_eq!(text(&app), "one two\ntXhree\n");
    }

    #[test]
    fn insert_commands_place_the_cursor() {
        let mut app = modal_app("    abc\nxyz");
        keys(&mut app, "A!");
        assert_eq!(text(&app), "    abc!\nxyz");
        esc(&mut app);
        keys(&mut app, "I>");
        assert_eq!(text(&app), "    >abc!\nxyz");
        esc(&mut app);
        keys(&mut app, "onew");
        assert_eq!(text(&app), "    >abc!\n    new\nxyz");
        esc(&mut app);
        keys(&mut app, "ggOtop");
        assert_eq!(text(&app), "top\n    >abc!\n    new\nxyz");
        esc(&mut app);
        // `G` is the end of the file, like Ctrl+End.
        keys(&mut app, "Ga.");
        assert!(text(&app).ends_with("xyz."), "{}", text(&app));
    }

    #[test]
    fn counts_repeat_motions_and_commands() {
        let lines: String = (1..=12).map(|n| format!("line {n}\n")).collect();
        let mut app = modal_app(&lines);
        keys(&mut app, "5j");
        assert_eq!(app.buffer().rope().char_to_line(head(&app)), 5);
        keys(&mut app, "3G");
        assert_eq!(app.buffer().rope().char_to_line(head(&app)), 2);
        keys(&mut app, "3dd");
        assert!(
            text(&app).starts_with("line 1\nline 2\nline 6\n"),
            "{}",
            text(&app)
        );
        // A started count shows in the status bar state and Esc drops it.
        keys(&mut app, "4");
        assert_eq!(app.modal().unwrap().count, Some(4));
        esc(&mut app);
        assert_eq!(app.modal().unwrap().count, None);
        assert_eq!(
            app.focus(),
            Focus::Editor,
            "that Esc only cancelled the count"
        );
    }

    #[test]
    fn visual_selects_copies_and_returns_to_normal() {
        let mut app = modal_app("alpha beta\n");
        keys(&mut app, "vllll");
        assert_eq!(mode(&app), EditMode::Visual { line: false });
        assert_eq!(app.buffer().selected_text(), "alph");
        keys(&mut app, "y");
        assert_eq!(mode(&app), EditMode::Normal);
        assert!(app.buffer().selection().is_empty());
        assert_eq!(
            head(&app),
            0,
            "a copy leaves the cursor where the selection began"
        );
        keys(&mut app, "$p");
        assert_eq!(text(&app), "alpha betaalph\n");
        // Visual is exclusive: the cursor sits between characters, as in Modeless.
        keys(&mut app, "0vlld");
        assert_eq!(text(&app), "pha betaalph\n");
        assert_eq!(mode(&app), EditMode::Normal);
        keys(&mut app, "vlc");
        assert_eq!(mode(&app), EditMode::Insert);
        keys(&mut app, "Z");
        assert_eq!(text(&app), "Zha betaalph\n");
    }

    #[test]
    fn visual_line_selects_whole_lines() {
        let mut app = modal_app("one\ntwo\nthree\nfour\n");
        keys(&mut app, "jlVj");
        assert_eq!(mode(&app), EditMode::Visual { line: true });
        assert_eq!(app.buffer().selected_text(), "two\nthree");
        keys(&mut app, "d");
        assert_eq!(text(&app), "one\nfour\n");
        keys(&mut app, "p");
        assert_eq!(
            text(&app),
            "one\ntwo\nthree\nfour\n",
            "the cut took whole lines"
        );
        // Up from the anchor line selects upward; Esc returns to the cursor column.
        let mut app = modal_app("one\ntwo\nthree\n");
        keys(&mut app, "jjlVk");
        assert_eq!(app.buffer().selected_text(), "two\nthree");
        esc(&mut app);
        assert_eq!(mode(&app), EditMode::Normal);
        assert!(app.buffer().selection().is_empty());
        assert_eq!(app.buffer().rope().char_to_line(head(&app)), 1);
        // A counted move keeps the cursor column too.
        let mut app = modal_app("one\ntwo\nthree\nfour\n");
        keys(&mut app, "lV2j");
        assert_eq!(app.buffer().selected_text(), "one\ntwo\nthree");
        esc(&mut app);
        assert_eq!(head(&app), 9, "line 3, column 2");
    }

    #[test]
    fn a_selection_made_in_normal_is_visual() {
        let mut app = modal_app("alpha beta\n");
        press(&mut app, KeyCode::Right, KeyModifiers::SHIFT);
        assert_eq!(mode(&app), EditMode::Visual { line: false });
        esc(&mut app);
        assert_eq!(mode(&app), EditMode::Normal);
        press(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(mode(&app), EditMode::Visual { line: false });
    }

    #[test]
    fn esc_leaving_insert_or_visual_is_not_the_first_of_a_double_esc() {
        let mut app = modal_app("hi\n");
        keys(&mut app, "i");
        esc(&mut app);
        assert_eq!(mode(&app), EditMode::Normal);
        esc(&mut app);
        assert_eq!(app.focus(), Focus::Editor, "Insert's Esc did not count");
        esc(&mut app);
        assert_eq!(app.focus(), Focus::CommandBar, "two Esc in Normal open it");
        let mut app = modal_app("hi\n");
        keys(&mut app, "vl");
        esc(&mut app);
        esc(&mut app);
        assert_eq!(app.focus(), Focus::Editor, "Visual's Esc did not count");
    }

    #[test]
    fn insert_esc_closes_a_popup_first() {
        let mut app = modal_app("hi\n");
        keys(&mut app, "i");
        app.nav.hover = Some("fn hi()".into());
        esc(&mut app);
        assert!(app.nav.hover.is_none());
        assert_eq!(mode(&app), EditMode::Insert);
        esc(&mut app);
        assert_eq!(mode(&app), EditMode::Normal);
    }

    #[test]
    fn the_key_list_filters_and_runs_a_key() {
        let mut app = modal_app("one\ntwo\n");
        keys(&mut app, "?");
        assert!(app.modal().unwrap().key_list.is_some());
        keys(&mut app, "open-line-b");
        let state = app.modal().unwrap();
        let rows = crate::modal::key_list(state.mode, &state.key_list.as_ref().unwrap().query);
        assert_eq!(rows[0].keys, "o");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.modal().unwrap().key_list.is_none());
        assert_eq!(mode(&app), EditMode::Insert);
        assert_eq!(text(&app), "one\n\ntwo\n");
    }

    #[test]
    fn every_mode_command_runs_from_the_command_bar() {
        let mut app = modal_app("one two\nthree\n");
        for name in [
            "mode-visual",
            "swap-selection-ends",
            "mode-normal",
            "mode-visual-line",
            "mode-normal",
            "insert-after",
            "mode-normal",
            "insert-line-start",
            "mode-normal",
            "insert-line-end",
            "mode-normal",
            "open-line-below",
            "mode-normal",
            "open-line-above",
            "mode-normal",
            "keys",
        ] {
            let spec =
                crate::command::find_command(name, crate::command::CommandContext::Editor, false)
                    .unwrap_or_else(|| panic!("{name} is offered with Modeless off"));
            app.run_command(spec.id, None)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        assert!(app.modal().unwrap().key_list.is_some());
        assert!(app.run_command(CommandId::ChangeSelection, None).is_ok());
        assert!(
            app.message
                .as_deref()
                .unwrap_or("")
                .contains("Select text first")
        );
        app.run_command(CommandId::ModeInsert, None).unwrap();
        assert_eq!(mode(&app), EditMode::Insert);
        // Turning Modeless back on hides them again.
        app.run_command(CommandId::Modeless, None).unwrap();
        assert!(app.modal().is_none());
        assert!(
            crate::command::find_command(
                "mode-insert",
                crate::command::CommandContext::Editor,
                false
            )
            .is_none()
        );
    }

    #[test]
    fn find_in_normal_moves_the_cursor_and_stays_normal() {
        let mut app = modal_app("alpha beta beta\n");
        keys(&mut app, "/");
        assert_eq!(app.focus(), Focus::Find);
        for c in "beta".chars() {
            press(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        }
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(mode(&app), EditMode::Normal);
        assert!(app.buffer().selection().is_empty());
        assert_eq!(head(&app), 6);
        keys(&mut app, "n");
        assert_eq!(mode(&app), EditMode::Normal);
        assert_eq!(head(&app), 11);
    }

    #[test]
    fn start_mode_insert_starts_typing() {
        let mut app = App::new(Buffer::from_text(""), None);
        app.editor_prefs.start_mode = StartModePref::Insert;
        app.run_command(CommandId::Modeless, None).unwrap();
        assert_eq!(mode(&app), EditMode::Insert);
        keys(&mut app, "hi");
        assert_eq!(text(&app), "hi");
    }
}
