//! Mouse clicks, drags, and the wheel.
//!
//! The screen is laid out again with [`crate::ui::frame_layout`], the same rectangles
//! the last frame drew, and the event goes to whatever is on top at that cell: an open
//! list, the assistant panel, the sidebar, then a window's tabs, gutter, or text.

use std::time::{Duration, Instant};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use super::{App, SCROLLOFF};
use crate::action::{CommandBarAction, Focus, SemanticAction, SidebarAction};
use crate::app::EditorId;
use crate::layout::Surface;

/// Clicks on the same cell closer together than this count as a double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);
/// Lines (or tree rows) one wheel notch scrolls.
const WHEEL_LINES: usize = 3;

#[derive(Debug, Default)]
pub(super) struct MouseState {
    /// The window a left-button drag started in.
    drag: Option<EditorId>,
    /// When and where the last left click landed, and how many in a row.
    last: Option<(Instant, u16, u16, u8)>,
}

impl App {
    /// Handles one mouse event. Returns true when something changed and should redraw.
    pub fn handle_mouse(&mut self, event: MouseEvent) -> bool {
        self.handle_mouse_at(event, Instant::now())
    }

    /// [`handle_mouse`](Self::handle_mouse) with the time of the event, for double-clicks.
    pub fn handle_mouse_at(&mut self, event: MouseEvent, now: Instant) -> bool {
        if !self.editor_prefs.mouse || self.pane_select.is_some() {
            self.mouse = MouseState::default();
            return false;
        }
        let at = Position::new(event.column, event.row);
        let shift = event.modifiers.contains(KeyModifiers::SHIFT);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let count = self.click_count(at, now);
                self.mouse.drag = None;
                self.press(at, count, shift)
            }
            MouseEventKind::Drag(MouseButton::Left) => self.drag_to(at),
            MouseEventKind::Up(MouseButton::Left) => {
                self.mouse.drag = None;
                false
            }
            MouseEventKind::ScrollDown => self.wheel(at, true),
            MouseEventKind::ScrollUp => self.wheel(at, false),
            _ => false,
        }
    }

    fn click_count(&mut self, at: Position, now: Instant) -> u8 {
        let count = match self.mouse.last {
            Some((when, x, y, count))
                if x == at.x && y == at.y && now.saturating_duration_since(when) <= MULTI_CLICK =>
            {
                count % 3 + 1
            }
            _ => 1,
        };
        self.mouse.last = Some((now, at.x, at.y, count));
        count
    }

    fn screen_layout(&self) -> crate::ui::EditorLayout {
        crate::ui::frame_layout(self, Rect::new(0, 0, self.screen_width, self.screen_height))
    }

    /// Routes a key as if it was pressed, for the wheel in lists and closing popups.
    fn press_key(&mut self, code: KeyCode) {
        let event = Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
        let mut burst = crate::agent::PasteBurst::default();
        if let Some(action) = crate::input::route_flags(
            &event,
            self.focus(),
            self.explorer.selecting,
            self.filtering(),
            self.keys(),
            self.capturing(),
            self.drafting(),
            &mut burst,
        ) {
            self.handle(action);
        }
    }

    fn press(&mut self, at: Position, count: u8, shift: bool) -> bool {
        match self.focus() {
            // Full-screen pages and dialogs keep their keyboard flow; clicks are ignored.
            Focus::Prompt
            | Focus::Settings
            | Focus::About
            | Focus::FilePicker
            | Focus::PreviewSide => {
                return false;
            }
            // A click closes these lists, then lands where it was aimed.
            Focus::CompletionPopup | Focus::ActionMenu | Focus::LayoutMenu => {
                self.press_key(KeyCode::Esc);
            }
            Focus::CommandBar => {
                let layout = self.screen_layout();
                if layout.command.contains(at) {
                    return self.click_command_row(layout.command, at, count);
                }
                if layout.status.contains(at) {
                    return false;
                }
                self.handle(SemanticAction::CommandBar(CommandBarAction::Close));
            }
            _ => {}
        }
        let layout = self.screen_layout();
        if layout.agent.height > 0 && layout.agent.contains(at) {
            self.focus_surface(Surface::Agent);
            self.agent.reading = false;
            self.agent.scroll = 0;
            return true;
        }
        if layout.sidebar.width > 0 && layout.sidebar.contains(at) {
            return self.click_sidebar(layout.sidebar, at, count, shift);
        }
        let Some(pane) = layout.panes.iter().find(|pane| {
            pane.title.contains(at) || pane.gutter.contains(at) || pane.text.contains(at)
        }) else {
            return false;
        };
        let id = pane.id;
        if pane.title.contains(at) {
            let tab = crate::ui::tab_at(self.editor(id), pane.title, at.x);
            self.focus_surface(Surface::Editor(id));
            if let Some(index) = tab
                && let Some(editor) = self.editors.iter_mut().find(|editor| editor.id == id)
            {
                editor.active = index;
            }
            return true;
        }
        self.focus_surface(Surface::Editor(id));
        if self.preview(id).is_some() {
            return true;
        }
        self.dismiss_lsp_popups();
        self.clear_snippet();
        let pos = self.text_pos(id, pane.text, at);
        let buffer = self.buffer_mut();
        match count {
            _ if shift => buffer.move_to(pos, true),
            2 => {
                let (start, end) = buffer.word_range(pos);
                buffer.select_span(start, end);
            }
            3 => {
                let line = buffer.rope().char_to_line(pos);
                let (start, end) = buffer.line_range(line);
                buffer.select_span(start, end);
            }
            _ => {
                buffer.deselect();
                buffer.move_to(pos, false);
            }
        }
        self.mouse.drag = (count == 1 || shift).then_some(id);
        self.scroll_to_cursor();
        true
    }

    /// The char under `at` in window `id`. The gutter maps to the start of its line, and a
    /// row past the last line to the end of the document.
    fn text_pos(&self, id: EditorId, text: Rect, at: Position) -> usize {
        let tab = self.editor(id).tab();
        let line = tab.view.scroll_y + at.y.saturating_sub(text.y) as usize;
        let vcol = if at.x < text.x {
            0
        } else {
            tab.view.scroll_x + (at.x - text.x) as usize
        };
        tab.buffer.pos_at_visual(line, vcol)
    }

    /// Extends the selection to the cell under the pointer. Above or below the text the
    /// view scrolls one line per move, so dragging past an edge keeps selecting.
    fn drag_to(&mut self, at: Position) -> bool {
        let Some(id) = self.mouse.drag else {
            return false;
        };
        let layout = self.screen_layout();
        let Some(pane) = layout.panes.iter().find(|pane| pane.id == id) else {
            return false;
        };
        if id != self.active {
            return false;
        }
        let text = pane.text;
        let tab = self.editor(id).tab();
        let height = tab.view.height.max(1) as u16;
        // Past an edge, aim just beyond the scroll margin so each move scrolls one line.
        let off = SCROLLOFF.min(tab.view.height.max(1).saturating_sub(1) / 2);
        let line = if at.y < text.y {
            if tab.view.scroll_y == 0 {
                0
            } else {
                (tab.view.scroll_y + off).saturating_sub(1)
            }
        } else if at.y >= text.y + height {
            (tab.view.scroll_y + height as usize).saturating_sub(off)
        } else {
            tab.view.scroll_y + (at.y - text.y) as usize
        };
        let vcol = tab.view.scroll_x + at.x.saturating_sub(text.x) as usize;
        let last = tab.buffer.len_lines().saturating_sub(1);
        let pos = tab.buffer.pos_at_visual(line.min(last), vcol);
        if pos == tab.buffer.selection().head {
            return false;
        }
        self.buffer_mut().move_to(pos, true);
        self.scroll_to_cursor();
        true
    }

    fn click_sidebar(&mut self, area: Rect, at: Position, count: u8, shift: bool) -> bool {
        self.focus_surface(Surface::Sidebar);
        let tree = crate::ui::sidebar_tree_area(area);
        if !tree.contains(at) {
            return true;
        }
        let index = self.explorer.scroll + (at.y - tree.y) as usize;
        if index >= self.explorer.visible_rows().len() {
            return true;
        }
        self.explorer.select_row(index, shift);
        if count == 2 && !shift {
            self.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        }
        true
    }

    /// One click highlights a row of the Commands list; a double click runs it.
    fn click_command_row(&mut self, area: Rect, at: Position, count: u8) -> bool {
        let Some((rows, start)) = crate::ui::command_list_rows(self, area) else {
            return false;
        };
        if !rows.contains(at) {
            return false;
        }
        let index = start + (at.y - rows.y) as usize;
        let len = match self.workspace_choices() {
            Some(choices) => choices.len(),
            None => {
                let bar = self.command_bar.as_ref().expect("the bar is open");
                crate::command::suggestions(
                    &bar.input,
                    self.recent_for(bar.context),
                    bar.context,
                    bar.files,
                )
                .len()
            }
        };
        if index >= len {
            return false;
        }
        let workspace = self.workspace_choices().is_some();
        let recent = self
            .command_bar
            .as_ref()
            .and_then(|bar| self.recent_for(bar.context));
        let Some(bar) = self.command_bar.as_mut() else {
            return false;
        };
        bar.selected = index;
        if count >= 2 && !workspace {
            // Run the clicked command, not a different one whose name was typed in full.
            if let Some(spec) = bar.selected(recent)
                && spec.arg.is_none()
            {
                bar.set_input(spec.name);
                bar.selected = 0;
                self.handle(SemanticAction::CommandBar(CommandBarAction::Submit));
            } else if let Some(spec) = bar.selected(recent) {
                bar.set_input(format!("{} ", spec.name));
                bar.selected = 0;
            }
        }
        true
    }

    fn wheel(&mut self, at: Position, down: bool) -> bool {
        let step = if down { KeyCode::Down } else { KeyCode::Up };
        match self.focus() {
            Focus::Prompt
            | Focus::Settings
            | Focus::About
            | Focus::FilePicker
            | Focus::PreviewSide
            | Focus::LayoutMenu
            | Focus::ActionMenu
            | Focus::CompletionPopup => {
                self.press_key(step);
                return true;
            }
            Focus::CommandBar => {
                let layout = self.screen_layout();
                if layout.command.contains(at) {
                    let action = if down {
                        CommandBarAction::SelectNext
                    } else {
                        CommandBarAction::SelectPrev
                    };
                    self.handle(SemanticAction::CommandBar(action));
                    return true;
                }
            }
            _ => {}
        }
        let layout = self.screen_layout();
        if layout.agent.height > 0 && layout.agent.contains(at) {
            self.agent.reading = true;
            let max = crate::ui::agent_max_scroll(self, layout.agent);
            let from = self.agent.scroll.min(max);
            self.agent.scroll = if down {
                from.saturating_sub(WHEEL_LINES)
            } else {
                (from + WHEEL_LINES).min(max)
            };
            if self.agent.scroll == 0 {
                self.agent.reading = false;
            }
            return true;
        }
        if layout.sidebar.width > 0 && layout.sidebar.contains(at) {
            let before = self.explorer.scroll;
            let delta = WHEEL_LINES as isize;
            self.explorer.scroll_by(if down { delta } else { -delta });
            return self.explorer.scroll != before;
        }
        let Some(pane) = layout
            .panes
            .iter()
            .find(|pane| pane.gutter.contains(at) || pane.text.contains(at))
        else {
            return false;
        };
        if self.preview(pane.id).is_some() {
            return self.wheel_preview(pane.id, down);
        }
        self.wheel_editor(pane.id, down)
    }

    fn wheel_preview(&mut self, id: EditorId, down: bool) -> bool {
        let rows = self.preview_rows(id);
        let Some(preview) = self.previews.iter_mut().find(|p| p.pane == id) else {
            return false;
        };
        let max = rows.saturating_sub(usize::from(preview.height.max(1)));
        let before = preview.scroll;
        preview.scroll = if down {
            (preview.scroll + WHEEL_LINES).min(max)
        } else {
            preview.scroll.saturating_sub(WHEEL_LINES)
        };
        preview.scroll != before
    }

    /// Scrolls window `id` and, like Vim, moves its cursor only as far as needed to stay
    /// on screen (keeping the selection's anchor), so the next frame keeps this scroll.
    fn wheel_editor(&mut self, id: EditorId, down: bool) -> bool {
        let Some(editor) = self.editors.iter_mut().find(|editor| editor.id == id) else {
            return false;
        };
        let tab = editor.tab_mut();
        let height = tab.view.height.max(1);
        let off = SCROLLOFF.min(height.saturating_sub(1) / 2);
        let last = tab.buffer.len_lines().saturating_sub(1);
        let max = last.saturating_sub(off);
        let before = tab.view.scroll_y;
        let top = if down {
            (before + WHEEL_LINES).min(max).max(before)
        } else {
            before.saturating_sub(WHEEL_LINES)
        };
        if top == before {
            return false;
        }
        tab.view.scroll_y = top;
        let (line, col) = tab.buffer.cursor_line_col();
        let low = if top == 0 { 0 } else { top + off };
        let high = (top + height - 1)
            .saturating_sub(off)
            .min(last)
            .max(low.min(last));
        let target = line.clamp(low.min(last), high);
        if target != line {
            let extend = !tab.buffer.selection().is_empty();
            let pos = tab.buffer.pos_at_visual(target, col);
            tab.buffer.move_to(pos, extend);
        }
        let (line, col) = tab.buffer.cursor_line_col();
        tab.view.scroll_to(line, col);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::explorer::{Explorer, FileEntry};
    use std::path::PathBuf;

    const W: u16 = 80;
    const H: u16 = 24;

    /// Lays the screen out the way the main loop does before each frame.
    fn settle(app: &mut App) {
        app.set_screen_size(W, H);
        let layout = app.screen_layout();
        app.resize_panes(&layout.pane_sizes());
        app.explorer.height = crate::ui::sidebar_tree_height(layout.sidebar);
    }

    fn app_with(text: &str) -> App {
        let mut app = App::new(Buffer::from_text(text), None);
        settle(&mut app);
        app
    }

    fn numbered(lines: usize) -> App {
        let text: String = (1..=lines).map(|n| format!("line {n}\n")).collect();
        app_with(&text)
    }

    fn event(kind: MouseEventKind, x: u16, y: u16, modifiers: KeyModifiers) -> MouseEvent {
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers,
        }
    }

    /// Sends `kind` at (x, y) at `ms` after a fixed start, then lays out like a frame.
    fn send(app: &mut App, kind: MouseEventKind, x: u16, y: u16, ms: u64) -> bool {
        send_mod(app, kind, x, y, ms, KeyModifiers::NONE)
    }

    fn send_mod(
        app: &mut App,
        kind: MouseEventKind,
        x: u16,
        y: u16,
        ms: u64,
        modifiers: KeyModifiers,
    ) -> bool {
        thread_local! {
            static START: Instant = Instant::now();
        }
        let now = START.with(|start| *start) + Duration::from_millis(ms);
        let changed = app.handle_mouse_at(event(kind, x, y, modifiers), now);
        settle(app);
        changed
    }

    fn down() -> MouseEventKind {
        MouseEventKind::Down(MouseButton::Left)
    }

    fn drag() -> MouseEventKind {
        MouseEventKind::Drag(MouseButton::Left)
    }

    fn up() -> MouseEventKind {
        MouseEventKind::Up(MouseButton::Left)
    }

    /// The text and gutter rectangles of the first window.
    fn pane(app: &App) -> (Rect, Rect, Rect) {
        let layout = app.screen_layout();
        let pane = &layout.panes[0];
        (pane.title, pane.gutter, pane.text)
    }

    /// Clicks the cell `col` columns into the text area on screen row `row` of the text.
    fn click_text(app: &mut App, col: u16, row: u16, ms: u64) {
        let (_, _, text) = pane(app);
        send(app, down(), text.x + col, text.y + row, ms);
        send(app, up(), text.x + col, text.y + row, ms);
    }

    fn cursor(app: &App) -> usize {
        app.buffer().selection().head
    }

    fn selected(app: &App) -> String {
        app.buffer().selected_text()
    }

    #[test]
    fn a_click_puts_the_cursor_on_that_cell() {
        let mut app = app_with("hello world\nsecond line\n");
        click_text(&mut app, 4, 1, 0);
        assert_eq!(cursor(&app), 12 + 4);
        assert!(app.buffer().selection().is_empty());
        assert_eq!(app.focus(), Focus::Editor);
    }

    #[test]
    fn a_click_past_the_end_of_a_line_or_the_text_lands_at_its_end() {
        let mut app = app_with("ab\ncd");
        click_text(&mut app, 30, 0, 0);
        assert_eq!(cursor(&app), 2);
        click_text(&mut app, 30, 10, 1_000);
        assert_eq!(cursor(&app), 5);
    }

    #[test]
    fn a_click_on_the_gutter_goes_to_the_start_of_that_line() {
        let mut app = app_with("first\n    second\nthird\n");
        let (_, gutter, text) = pane(&app);
        assert!(gutter.width > 0 && gutter.right() <= text.x);
        send(&mut app, down(), gutter.x, gutter.y + 1, 0);
        assert_eq!(cursor(&app), 6, "the line's first char, not its indent");
        send(&mut app, down(), text.x - 1, gutter.y + 2, 1_000);
        assert_eq!(cursor(&app), 6 + 11);
    }

    #[test]
    fn wide_chars_map_both_of_their_cells_to_the_char() {
        // "a界b": a at col 0, 界 over cols 1-2, b at col 3.
        let mut app = app_with("a界b😀c\n");
        click_text(&mut app, 1, 0, 0);
        assert_eq!(cursor(&app), 1);
        click_text(&mut app, 2, 0, 1_000);
        assert_eq!(cursor(&app), 1, "the right half of 界");
        click_text(&mut app, 3, 0, 2_000);
        assert_eq!(cursor(&app), 2);
        click_text(&mut app, 5, 0, 3_000);
        assert_eq!(cursor(&app), 3, "the right half of the emoji");
        click_text(&mut app, 6, 0, 4_000);
        assert_eq!(cursor(&app), 4);
    }

    #[test]
    fn a_tab_covers_every_cell_up_to_the_next_stop() {
        let mut app = app_with("a\tb\n");
        let width = app.buffer().tab_width();
        assert!(width >= 2);
        click_text(&mut app, 1, 0, 0);
        assert_eq!(cursor(&app), 1);
        click_text(&mut app, width as u16 - 1, 0, 1_000);
        assert_eq!(cursor(&app), 1, "still inside the tab");
        click_text(&mut app, width as u16, 0, 2_000);
        assert_eq!(cursor(&app), 2, "b sits on the tab stop");
    }

    #[test]
    fn clicks_in_a_scrolled_view_count_from_the_first_shown_line_and_column() {
        let long = format!("{}xyz\n", "-".repeat(200));
        let mut text: String = (0..100).map(|n| format!("row {n}\n")).collect();
        text.push_str(&long);
        let mut app = app_with(&text);
        app.active_editor_mut().tab_mut().view.scroll_y = 40;
        click_text(&mut app, 0, 2, 0);
        assert_eq!(app.buffer().cursor_line_col(), (42, 0));

        // Scroll right on the long last line.
        let last = app.buffer().len_lines() - 2;
        let view = &mut app.active_editor_mut().tab_mut().view;
        view.scroll_y = last - 5;
        view.scroll_x = 190;
        app.handle_mouse_at(
            event(
                down(),
                pane(&app).2.x + 11,
                pane(&app).2.y + 5,
                KeyModifiers::NONE,
            ),
            Instant::now(),
        );
        assert_eq!(app.buffer().cursor_line_col(), (last, 201));
    }

    #[test]
    fn dragging_selects_the_buffer_text_between_press_and_pointer() {
        let mut app = app_with("alpha beta\ngamma delta\n");
        let (_, _, text) = pane(&app);
        send(&mut app, down(), text.x + 6, text.y, 0);
        send(&mut app, drag(), text.x + 3, text.y + 1, 10);
        send(&mut app, drag(), text.x + 5, text.y + 1, 20);
        send(&mut app, up(), text.x + 5, text.y + 1, 30);
        assert_eq!(selected(&app), "beta\ngamma");

        // Dragging over the gutter selects from the line start, never the numbers.
        send(&mut app, down(), text.x + 5, text.y + 1, 1_000);
        send(&mut app, drag(), 0, text.y, 1_010);
        assert_eq!(selected(&app), "alpha beta\ngamma");
    }

    #[test]
    fn dragging_past_the_bottom_or_top_edge_scrolls_and_keeps_selecting() {
        let mut app = numbered(100);
        let (_, _, text) = pane(&app);
        let height = app.active_editor().tab().view.height;
        send(&mut app, down(), text.x, text.y, 0);
        for step in 0..10 {
            send(
                &mut app,
                drag(),
                text.x,
                text.y + height as u16 + 2,
                10 + step,
            );
        }
        assert_eq!(
            app.active_editor().tab().view.scroll_y,
            10,
            "one line per move past the edge"
        );
        let (line, _) = app.buffer().cursor_line_col();
        assert_eq!(line, 10 + height - 1 - SCROLLOFF);
        assert_eq!(app.buffer().selection().anchor, 0);

        for step in 0..30 {
            send(&mut app, drag(), text.x, 0, 100 + step);
        }
        assert_eq!(app.active_editor().tab().view.scroll_y, 0);
        assert_eq!(cursor(&app), 0);
    }

    #[test]
    fn a_drag_without_a_press_in_the_text_does_nothing() {
        let mut app = app_with("alpha\n");
        let (_, _, text) = pane(&app);
        assert!(!send(&mut app, drag(), text.x + 3, text.y, 0));
        assert!(app.buffer().selection().is_empty());
    }

    #[test]
    fn shift_click_extends_the_selection_from_the_cursor() {
        let mut app = app_with("one two three\n");
        click_text(&mut app, 4, 0, 0);
        let (_, _, text) = pane(&app);
        send_mod(
            &mut app,
            down(),
            text.x + 7,
            text.y,
            1_000,
            KeyModifiers::SHIFT,
        );
        assert_eq!(selected(&app), "two");
        send_mod(
            &mut app,
            down(),
            text.x + 13,
            text.y,
            2_000,
            KeyModifiers::SHIFT,
        );
        assert_eq!(selected(&app), "two three", "the anchor stays put");
    }

    #[test]
    fn double_click_selects_a_word_and_triple_click_its_line() {
        let mut app = app_with("let value = 42;\nnext\n");
        click_text(&mut app, 6, 0, 0);
        click_text(&mut app, 6, 0, 100);
        assert_eq!(selected(&app), "value");
        click_text(&mut app, 6, 0, 200);
        assert_eq!(selected(&app), "let value = 42;\n");

        // Slow clicks are separate single clicks.
        click_text(&mut app, 6, 0, 2_000);
        click_text(&mut app, 6, 0, 3_000);
        assert!(app.buffer().selection().is_empty());
    }

    #[test]
    fn copy_after_a_drag_takes_only_the_buffer_text() {
        let mut app = app_with("    fn main() {}\n");
        let (_, gutter, text) = pane(&app);
        send(&mut app, down(), gutter.x, text.y, 0);
        send(&mut app, drag(), text.x + 9, text.y, 10);
        assert_eq!(selected(&app), "    fn ma");
    }

    #[test]
    fn the_wheel_scrolls_the_text_and_the_cursor_stays_on_screen() {
        let mut app = numbered(100);
        let (_, _, text) = pane(&app);
        for ms in 0..4 {
            send(&mut app, MouseEventKind::ScrollDown, text.x, text.y, ms);
        }
        let view = &app.active_editor().tab().view;
        assert_eq!(
            view.scroll_y, 12,
            "three lines per notch survive the next frame"
        );
        let (line, _) = app.buffer().cursor_line_col();
        assert!(line >= 12 && line < 12 + view.height);

        send(&mut app, MouseEventKind::ScrollUp, text.x, text.y, 10);
        assert_eq!(app.active_editor().tab().view.scroll_y, 9);
    }

    #[test]
    fn the_wheel_keeps_a_selection_anchored() {
        let mut app = numbered(100);
        click_text(&mut app, 2, 0, 0);
        let (_, _, text) = pane(&app);
        send_mod(
            &mut app,
            down(),
            text.x + 4,
            text.y,
            1_000,
            KeyModifiers::SHIFT,
        );
        for ms in 0..5 {
            send(
                &mut app,
                MouseEventKind::ScrollDown,
                text.x,
                text.y,
                2_000 + ms,
            );
        }
        assert_eq!(app.buffer().selection().anchor, 2);
        assert!(selected(&app).len() > 20);
    }

    fn with_tree(app: &mut App) {
        let root = PathBuf::from("/tmp/duckx-mouse");
        app.explorer = Explorer::new(root.clone());
        app.explorer.open = true;
        let file = |name: &str| FileEntry {
            name: name.into(),
            depth: 1,
            path: root.join(name),
            is_dir: false,
        };
        let entries: Vec<FileEntry> = (0..40).map(|n| file(&format!("f{n:02}.rs"))).collect();
        app.explorer.set_entries(entries);
        settle(app);
    }

    #[test]
    fn clicking_the_sidebar_focuses_it_and_selects_the_row() {
        let mut app = app_with("text\n");
        with_tree(&mut app);
        assert_eq!(app.focus(), Focus::Editor);
        let layout = app.screen_layout();
        let tree = crate::ui::sidebar_tree_area(layout.sidebar);
        send(&mut app, down(), tree.x + 2, tree.y + 3, 0);
        assert_eq!(app.focus(), Focus::Sidebar);
        assert_eq!(app.explorer.selected_index(), Some(3));

        // And a click on the text gives the focus back to the editor.
        click_text(&mut app, 1, 0, 1_000);
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(cursor(&app), 1);
    }

    #[test]
    fn the_wheel_scrolls_the_sidebar_under_the_pointer() {
        let mut app = app_with("text\n");
        with_tree(&mut app);
        let layout = app.screen_layout();
        let tree = crate::ui::sidebar_tree_area(layout.sidebar);
        send(&mut app, MouseEventKind::ScrollDown, tree.x, tree.y, 0);
        assert_eq!(app.explorer.scroll, 3);
        send(&mut app, MouseEventKind::ScrollUp, tree.x, tree.y, 1);
        assert_eq!(app.explorer.scroll, 0);
        assert_eq!(
            app.focus(),
            Focus::Editor,
            "scrolling does not move the focus"
        );
    }

    #[test]
    fn clicking_a_tab_switches_to_it() {
        let mut app = app_with("first\n");
        let pane_id = app.active;
        app.push_tab(pane_id);
        settle(&mut app);
        assert_eq!(app.active_editor().active, 1);
        let (title, _, _) = pane(&app);
        // " [No Name]  [No Name]": the first tab spans columns 1-9.
        send(&mut app, down(), title.x + 3, title.y, 0);
        assert_eq!(app.active_editor().active, 0);
        assert_eq!(app.buffer().rope().to_string(), "first\n");
        send(&mut app, down(), title.x + 13, title.y, 1_000);
        assert_eq!(app.active_editor().active, 1);
    }

    #[test]
    fn clicking_another_window_focuses_it() {
        let mut app = app_with("left\n");
        let left = app.active;
        app.split(crate::layout::Axis::Horizontal);
        settle(&mut app);
        assert_ne!(app.active, left);
        let layout = app.screen_layout();
        let target = layout
            .panes
            .iter()
            .find(|pane| pane.id == left)
            .unwrap()
            .text;
        send(&mut app, down(), target.x + 2, target.y, 0);
        assert_eq!(app.active, left);
        assert_eq!(cursor(&app), 2);
    }

    #[test]
    fn the_assistant_panel_takes_focus_and_the_wheel() {
        let mut app = numbered(10);
        app.agent.open = true;
        app.agent.focused = true;
        for n in 0..40 {
            app.agent.transcript.push(crate::agent::TranscriptLine {
                user: n % 2 == 0,
                text: format!("message {n}"),
            });
        }
        settle(&mut app);
        let panel = app.screen_layout().agent;
        assert!(panel.height > 0);
        send(&mut app, down(), panel.x + 2, panel.y + 1, 0);
        assert_eq!(app.focus(), Focus::Agent);

        send(
            &mut app,
            MouseEventKind::ScrollUp,
            panel.x + 2,
            panel.y + 1,
            2_000,
        );
        assert!(app.agent.reading);
        assert_eq!(app.agent.scroll, WHEEL_LINES);
        for ms in 0..100 {
            send(
                &mut app,
                MouseEventKind::ScrollUp,
                panel.x + 2,
                panel.y + 1,
                2_000 + ms,
            );
        }
        let max = crate::ui::agent_max_scroll(&app, panel);
        assert_eq!(app.agent.scroll, max, "stops at the oldest message");
        for ms in 0..(max / WHEEL_LINES) as u64 {
            send(
                &mut app,
                MouseEventKind::ScrollDown,
                panel.x + 2,
                panel.y + 1,
                3_000 + ms,
            );
        }
        send(
            &mut app,
            MouseEventKind::ScrollDown,
            panel.x + 2,
            panel.y + 1,
            2_001,
        );
        assert!(!app.agent.reading);

        click_text(&mut app, 0, 0, 3_000);
        assert_eq!(app.focus(), Focus::Editor);
    }

    #[test]
    fn the_commands_list_takes_clicks_and_a_click_outside_closes_it() {
        let mut app = numbered(30);
        app.handle(SemanticAction::OpenCommandBar);
        settle(&mut app);
        assert_eq!(app.focus(), Focus::CommandBar);
        let layout = app.screen_layout();
        let (rows, start) = crate::ui::command_list_rows(&app, layout.command).unwrap();
        send(&mut app, down(), rows.x + 2, rows.y + 2, 0);
        assert_eq!(app.command_bar.as_ref().unwrap().selected, start + 2);
        assert_eq!(app.focus(), Focus::CommandBar);

        send(
            &mut app,
            MouseEventKind::ScrollDown,
            rows.x + 2,
            rows.y,
            1_000,
        );
        assert_eq!(app.command_bar.as_ref().unwrap().selected, start + 3);

        let (_, _, text) = pane(&app);
        let outside = (text.y..text.bottom())
            .flat_map(|y| (text.x..text.right()).map(move |x| Position::new(x, y)))
            .find(|at| !layout.command.contains(*at))
            .expect("some text shows beside the list");
        send(&mut app, down(), outside.x, outside.y, 2_000);
        assert!(app.command_bar.is_none(), "a click outside closes the list");
        assert_eq!(app.focus(), Focus::Editor);
    }

    #[test]
    fn double_clicking_a_command_runs_it() {
        let mut app = app_with("text\n");
        app.handle(SemanticAction::OpenCommandBar);
        app.command_bar.as_mut().unwrap().set_input("split-right");
        settle(&mut app);
        let layout = app.screen_layout();
        let (rows, _) = crate::ui::command_list_rows(&app, layout.command).unwrap();
        send(&mut app, down(), rows.x + 2, rows.y, 0);
        send(&mut app, down(), rows.x + 2, rows.y, 100);
        assert!(app.command_bar.is_none());
        assert_eq!(app.editors.len(), 2);
    }

    #[test]
    fn with_the_setting_off_mouse_events_are_ignored() {
        let mut app = app_with("hello\n");
        assert!(app.editor_prefs.mouse, "on by default");
        app.editor_prefs.mouse = false;
        let (_, _, text) = pane(&app);
        assert!(!send(&mut app, down(), text.x + 3, text.y, 0));
        assert!(!send(
            &mut app,
            MouseEventKind::ScrollDown,
            text.x,
            text.y,
            1
        ));
        assert_eq!(cursor(&app), 0);
    }

    #[test]
    fn a_click_on_a_full_screen_page_is_ignored() {
        let mut app = app_with("hello\n");
        app.handle(SemanticAction::OpenCommandBar);
        app.command_bar.as_mut().unwrap().set_input("settings");
        app.handle(SemanticAction::CommandBar(CommandBarAction::Submit));
        assert_eq!(app.focus(), Focus::Settings);
        assert!(!send(&mut app, down(), 5, 5, 0));
        assert_eq!(app.focus(), Focus::Settings);
    }
}
