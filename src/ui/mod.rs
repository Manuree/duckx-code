mod about;
mod agent;
mod command_bar;
mod cursor;
mod icons;
mod picker;
mod popup;
mod prompt;
mod settings;
mod sidebar;
mod status_bar;
mod viewport;

use std::io;
use std::path::Path;

use ratatui::{
    Frame, Terminal,
    backend::Backend,
    layout::{Position, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph},
};

use crate::theme::UiStyles;

use crate::app::App;
use crate::layout::{self, Divider, EditorId, Layout};

/// The only way to create a bordered panel: every border in duckx-code is rounded.
pub(crate) fn panel(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .title(title)
}

pub struct EditorPane {
    pub id: EditorId,
    pub title: Rect,
    pub gutter: Rect,
    pub text: Rect,
}

pub struct EditorLayout {
    /// Zero-width when the sidebar is closed.
    pub sidebar: Rect,
    pub panes: Vec<EditorPane>,
    pub dividers: Vec<Divider>,
    pub command: Rect,
    /// The diagnostic message row. Zero height when the cursor line has none.
    pub diagnostic: Rect,
    pub status: Rect,
    /// Zero height while the assistant panel is closed.
    pub agent: Rect,
}

/// The rectangles the screen draws for `app`. Focus mode keeps only the active
/// buffer: no title, no sidebar, no other window, and no status line unless the
/// command bar, find, or replace is open.
pub fn frame_layout(app: &App, area: Rect) -> EditorLayout {
    let focus = app.focus_mode();
    let sole = Layout::Leaf(app.active_id());
    layout(
        area,
        app.explorer.open && !focus,
        app.sidebar_width(),
        if focus { &sole } else { app.layout() },
        |id| app.editor(id).tab().buffer.len_lines(),
        command_rows(app, area.width, area.height),
        if focus { 0 } else { diagnostic_rows(app) },
        app.show_diff(),
        !focus,
        !focus || app.composer_open(),
        app.agent_rows(),
    )
}

pub fn layout(
    area: Rect,
    sidebar_open: bool,
    sidebar_width: u16,
    tree: &Layout,
    line_count: impl Fn(EditorId) -> usize,
    command_rows: u16,
    diagnostic_rows: u16,
    diff_column: bool,
    show_title: bool,
    show_status: bool,
    agent_rows: u16,
) -> EditorLayout {
    let status_rows = if show_status { area.height.min(1) } else { 0 };
    let command_rows = command_rows.min(area.height.saturating_sub(status_rows));
    let diagnostic_rows =
        diagnostic_rows.min(area.height.saturating_sub(status_rows + command_rows));
    let used = status_rows + command_rows + diagnostic_rows;
    let agent_rows = agent_rows.min(area.height.saturating_sub(used.saturating_add(3)));
    let (sidebar, editor_area) =
        layout::split_above(area, sidebar_open, used + agent_rows, sidebar_width);
    let placed = tree.place(editor_area);
    let panes = placed
        .panes
        .into_iter()
        .map(|(id, area)| {
            let (title, body) = split_title(area, show_title);
            let (gutter, text) = split_gutter(body, line_count(id), diff_column);
            EditorPane {
                id,
                title,
                gutter,
                text,
            }
        })
        .collect();
    EditorLayout {
        sidebar,
        panes,
        dividers: placed.dividers,
        agent: Rect::new(area.x, area.y + sidebar.height, area.width, agent_rows),
        command: Rect::new(
            area.x,
            area.y + sidebar.height + agent_rows,
            area.width,
            command_rows,
        ),
        diagnostic: Rect::new(
            area.x,
            area.y + sidebar.height + agent_rows + command_rows,
            area.width,
            diagnostic_rows,
        ),
        status: Rect::new(
            area.x,
            area.y + sidebar.height + agent_rows + command_rows + diagnostic_rows,
            area.width,
            status_rows,
        ),
    }
}

/// One row when the cursor line has a diagnostic to show above the status bar.
pub fn diagnostic_rows(app: &App) -> u16 {
    u16::from(app.cursor_diagnostic().is_some())
}

/// Rows the command composer takes above the status line. Zero when it is closed.
pub fn command_rows(app: &App, width: u16, height: u16) -> u16 {
    let find = command_bar::mode_hint_rows(app, width, height);
    if find > 0 {
        return find;
    }
    command_bar::reserved_height(app, width, height)
}

fn split_title(area: Rect, show_title: bool) -> (Rect, Rect) {
    let title_height = if show_title {
        layout::title_rows(area.height)
    } else {
        0
    };
    (
        Rect::new(area.x, area.y, area.width, title_height),
        Rect::new(
            area.x,
            area.y + title_height,
            area.width,
            area.height - title_height,
        ),
    )
}

fn split_gutter(area: Rect, line_count: usize, diff_column: bool) -> (Rect, Rect) {
    let gutter_width = (gutter_width(line_count, diff_column) as u16).min(area.width);
    (
        Rect::new(area.x, area.y, gutter_width, area.height),
        Rect::new(
            area.x + gutter_width,
            area.y,
            area.width - gutter_width,
            area.height,
        ),
    )
}

/// Number of tree rows the sidebar shows in `area`, for scrolling and paging.
pub fn sidebar_tree_height(area: Rect) -> usize {
    sidebar::tree_area(area).height as usize
}

fn render_title(
    frame: &mut Frame,
    editor: &crate::app::Editor,
    area: Rect,
    focused: bool,
    theme: &UiStyles,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let active = editor.active_index();
    let tabs = editor.tabs();
    let room = area.width.saturating_sub(1) as usize;
    let (start, end) = visible_tabs(tabs, active, room);
    let mut spans = Vec::new();
    if start > 0 {
        spans.push(Span::styled("‹", theme.dim));
    } else {
        spans.push(Span::styled(" ", title_style(focused, true, theme)));
    }
    for (index, tab) in tabs.iter().enumerate().take(end).skip(start) {
        if index > start {
            spans.push(Span::styled("  ", theme.dim));
        }
        let selected = index == active;
        let style = title_style(focused, selected, theme);
        if let Some((icon, color)) = icons::known_file_icon(tab.path.as_deref()) {
            spans.push(Span::styled(icon, style.fg(color)));
            spans.push(Span::styled(" ", style));
        }
        let mut name = buffer_name(tab.path.as_deref());
        if tab.dirty {
            name.push_str(" [+]");
        }
        spans.push(Span::styled(name, style));
    }
    if end < tabs.len() {
        spans.push(Span::styled(" ›", theme.dim));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.title), area);
}

fn title_style(pane_focused: bool, selected: bool, theme: &UiStyles) -> Style {
    if selected && pane_focused {
        theme.title_active
    } else if selected {
        theme.title
    } else {
        theme.dim
    }
}

/// Inclusive-exclusive range of tabs that fit in `room` columns, always including the active one.
fn visible_tabs(tabs: &[crate::app::Tab], active: usize, room: usize) -> (usize, usize) {
    if tabs.is_empty() {
        return (0, 0);
    }
    let widths: Vec<usize> = tabs.iter().map(tab_width).collect();
    let mut start = 0;
    let mut end = tabs.len();
    loop {
        let right = if end < tabs.len() { 2 } else { 0 };
        let gaps = (end - start).saturating_sub(1) * 2;
        let total = widths[start..end].iter().sum::<usize>() + gaps + right;
        if total <= room || end - start <= 1 {
            break;
        }
        let left_dist = active.saturating_sub(start);
        let right_dist = end.saturating_sub(active + 1);
        if right_dist > left_dist {
            end -= 1;
        } else if start < active {
            start += 1;
        } else {
            end -= 1;
        }
    }
    (start, end)
}

fn tab_width(tab: &crate::app::Tab) -> usize {
    use unicode_width::UnicodeWidthStr;
    let mut width = buffer_name(tab.path.as_deref()).width();
    if tab.dirty {
        width += " [+]".width();
    }
    if icons::known_file_icon(tab.path.as_deref()).is_some() {
        width += 2;
    }
    width
}

fn buffer_name(path: Option<&Path>) -> String {
    path.and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "[No Name]".to_string())
}

fn render_divider(frame: &mut Frame, divider: &Divider, theme: &UiStyles) {
    use ratatui::text::Line;
    use ratatui::widgets::Paragraph;

    let glyph = if divider.vertical { "│" } else { "─" };
    let lines: Vec<Line> = (0..divider.area.height)
        .map(|_| Line::styled(glyph.repeat(divider.area.width as usize), theme.dim))
        .collect();
    frame.render_widget(Paragraph::new(lines), divider.area);
}

fn gutter_width(line_count: usize, diff_column: bool) -> usize {
    line_count.to_string().len().max(3) + 2 + usize::from(diff_column)
}

/// Paint one frame, then show the block cursor on the recorded cell.
///
/// The cursor stays hidden while changed cells are written, so a spinning
/// title does not catch the block.
pub fn draw<B: Backend>(terminal: &mut Terminal<B>, app: &App) -> io::Result<()> {
    terminal.hide_cursor()?;
    terminal.draw(|frame| render(frame, app))?;
    if let Some((x, y)) = cursor::position() {
        terminal.set_cursor_position(Position::new(x, y))?;
        terminal.show_cursor()?;
    }
    Ok(())
}

pub fn render(frame: &mut Frame, app: &App) {
    cursor::clear();
    let area = frame.area();
    let theme = *app.styles();
    frame.render_widget(
        Block::new().style(Style::new().fg(theme.editor_fg).bg(theme.editor_bg)),
        area,
    );
    let layout = frame_layout(app, area);
    let focus = app.focus();
    let editor_focused = focus == crate::action::Focus::Editor
        || focus == crate::action::Focus::CompletionPopup
        || focus == crate::action::Focus::ActionMenu;
    let mut active_text = None;
    for pane in &layout.panes {
        let editor = app.editor(pane.id);
        let focused = editor_focused && editor.id == app.active_id();
        render_title(frame, editor, pane.title, focused, &theme);
        let start = editor.tab().view.scroll_y;
        let end = start + pane.text.height as usize;
        let semantic = editor
            .tab()
            .path
            .as_ref()
            .map(|path| app.semantic.get(path))
            .unwrap_or(&[]);
        let highlights = editor.tab().highlights(start, end, semantic);
        let diagnostics = editor.tab().diagnostic_spans(&app.diagnostics, start, end);
        viewport::render(
            frame,
            &editor.tab().buffer,
            editor.tab().view,
            pane.gutter,
            pane.text,
            focused,
            &theme,
            &app.appearance.syntax,
            &highlights,
            &diagnostics,
            app.show_diff(),
            editor.tab().file_diff(),
        );
        if editor.id == app.active_id() {
            active_text = Some(pane.text);
        }
    }
    popup::render(frame, app, active_text, &theme);
    for divider in &layout.dividers {
        render_divider(frame, divider, &theme);
    }
    if app.explorer.open {
        sidebar::render(frame, &app.explorer, layout.sidebar, &theme);
    }
    if (app.find_mode.is_some() || app.replace_mode.is_some()) && layout.command.height > 0 {
        command_bar::render_mode_hint(frame, app, layout.command);
    } else if layout.command.height > 0 {
        command_bar::render(frame, app, layout.command);
    }
    if layout.agent.height > 0 {
        agent::render(frame, app, layout.agent);
    }
    if layout.diagnostic.height > 0 {
        status_bar::render_diagnostic(frame, app, layout.diagnostic);
    }
    if layout.status.height > 0 {
        status_bar::render(frame, app, layout.status);
    }
    let above_status = Rect {
        height: area.height - layout.status.height,
        ..area
    };
    settings::render(frame, app, above_status);
    about::render(frame, app, above_status);
    picker::render(frame, app, above_status);
    prompt::render(frame, app, above_status);
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use std::path::PathBuf;

    use crate::action::{CommandBarAction, SemanticAction, SidebarAction};
    use crate::buffer::Buffer;
    use crate::explorer::{Explorer, FileEntry};
    use crate::jobs::AppEvent;
    use crate::services::git::GitStatus;

    fn draw(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn assistant_panel_uses_rounded_corners() {
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.agent.open = true;
        app.agent.focused = true;
        app.agent.transcript.push(crate::agent::TranscriptLine {
            user: true,
            text: "rename greet".into(),
        });
        app.set_screen_size(80, 24);
        let screen = draw(&app, 80, 24);
        for corner in ["╭", "╮", "╰", "╯"] {
            assert!(
                screen.contains(corner),
                "missing rounded corner {corner}\n{screen}"
            );
        }
        assert!(screen.contains("Assistant"), "{screen}");
        assert!(screen.contains("fn main"), "{screen}");
    }

    #[test]
    fn assistant_messages_wrap_instead_of_stopping_at_the_edge() {
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.agent.open = true;
        app.agent.focused = true;
        app.set_screen_size(80, 24);
        let body = format!("HEAD{}TAIL", " word".repeat(30));
        app.agent.transcript.push(crate::agent::TranscriptLine {
            user: false,
            text: body,
        });
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("HEAD"), "{screen}");
        assert!(screen.contains("TAIL"), "{screen}");
    }

    #[test]
    fn assistant_approval_shows_buttons_and_diff_card() {
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.agent.open = true;
        app.agent.focused = true;
        app.agent.busy = true;
        app.agent.status = "approve apply_edit · ←/→ Enter".into();
        app.agent.pending = Some(crate::agent::PendingTool {
            turn: 1,
            call_id: "c1".into(),
            name: "apply_edit".into(),
            arguments: r#"{"path":"demo.txt","find":"hello","replace":"hi"}"#.into(),
            summary: "apply_edit · demo.txt\n- hello\n+ hi".into(),
            allow_selected: true,
        });
        app.set_screen_size(80, 24);
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("apply_edit"), "{screen}");
        assert!(screen.contains("- hello"), "{screen}");
        assert!(screen.contains("+ hi"), "{screen}");
        assert!(screen.contains("[ Yes ]"), "{screen}");
        assert!(screen.contains("[ No ]"), "{screen}");
        assert!(screen.contains("> [ Yes ]"), "{screen}");
        app.agent.pending.as_mut().unwrap().allow_selected = false;
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("> [ No ]"), "{screen}");
    }

    #[test]
    fn assistant_panel_hides_outside_agent_mode() {
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.agent.open = true;
        app.agent.focused = true;
        app.handle(SemanticAction::OpenCommandBar);
        let screen = draw(&app, 80, 24);
        assert!(!screen.contains("Assistant"), "{screen}");
        assert!(screen.contains("fn main"), "{screen}");
        assert!(screen.contains("COMMAND"), "{screen}");
    }

    #[test]
    fn assistant_spinner_turns_while_busy() {
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.agent.open = true;
        app.agent.focused = true;
        app.agent.busy = true;
        app.agent.status = "waiting".into();
        app.agent.spin = 0;
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("⠋ waiting"), "{screen}");
        app.advance_agent_spin();
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("⠙ waiting"), "{screen}");
        app.agent.busy = false;
        app.agent.status = "ready".into();
        app.advance_agent_spin();
        assert_eq!(app.agent.spin, 1);
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("ready"), "{screen}");
        assert!(!screen.contains('⠋'), "{screen}");
    }

    #[test]
    fn the_wait_spinner_does_not_keep_the_cursor() {
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.agent.open = true;
        app.agent.focused = true;
        app.agent.busy = true;
        app.agent.status = "waiting".into();
        app.agent.spin = 0;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        super::draw(&mut terminal, &app).unwrap();
        let (x, y) = cursor::position().expect("prompt cursor");
        let pos = terminal.get_cursor_position().unwrap();
        assert_eq!((pos.x, pos.y), (x, y));
        let cursor_cell = terminal.backend().buffer()[(x, y)].symbol();
        assert_ne!(cursor_cell, "⠋");
        let spinner = (0..24).find_map(|row| {
            (0..80).find_map(|col| {
                (terminal.backend().buffer()[(col, row)].symbol() == "⠋").then_some((col, row))
            })
        });
        assert_ne!(spinner, Some((x, y)));
    }

    #[test]
    fn settings_model_draft_places_the_cursor_after_the_text() {
        use crate::settings::{Page, SettingId, TextDraft, sections};

        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        let section = sections()
            .iter()
            .position(|item| item.name == "AI")
            .unwrap();
        let row = sections()[section]
            .items
            .iter()
            .position(|item| item.id == SettingId::AiModel)
            .unwrap();
        let mut page = Page::new();
        page.section = section;
        page.row = row;
        page.inside = true;
        page.text = Some(TextDraft::new("qwen".into()));
        app.settings = Some(page);
        let (x, y) = cursor_at(&app, 80, 24);
        let line = draw(&app, 80, 24);
        let line = line.lines().nth(y as usize).unwrap();
        let name_at = line.find("qwen").expect(line);
        let columns: Vec<char> = line.chars().collect();
        let end = line[..name_at].chars().count() + "qwen".chars().count();
        assert!(end < columns.len(), "{line}");
        assert_eq!(columns[end], ' ', "{line}");
        assert_eq!(x as usize, end, "{line}");
    }

    #[test]
    fn command_bar_overlay_shows_input_and_suggestions() {
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "go".into(),
        )));
        let screen = draw(&app, 80, 20);
        let lines: Vec<&str> = screen.lines().collect();
        let hint = lines
            .iter()
            .copied()
            .find(|line| line.contains("goto <line>"))
            .expect("goto row");
        let status = *lines.last().unwrap();
        assert!(status.contains(" COMMAND "), "{status}");
        assert!(status.contains("❯"), "{status}");
        assert!(status.contains("go"), "{status}");
        assert!(!status.contains("goto"), "{status}");
        assert!(!status.contains("[No Name]"), "{status}");
        assert!(!status.contains("NORMAL"), "{status}");
        assert!(screen.contains("goto <line>"), "{screen}");
        assert!(screen.contains("Go to line number"), "{screen}");
        assert!(hint.contains("↑↓"), "{hint}");
        assert!(hint.contains("tab"), "{hint}");
        assert!(hint.contains("esc"), "{hint}");
        assert!(!screen.contains("╭"), "{screen}");
    }

    #[test]
    fn file_picker_is_a_rounded_modal_of_matching_files() {
        use crate::jobs::{AppEvent, Job};
        use crate::picker::IndexedFile;

        let mut app = App::new(Buffer::from_text("stay\n"), None);
        app.explorer = Explorer::new(PathBuf::from("/work/duckx"));
        app.handle(SemanticAction::OpenFilePicker);
        let _ = app.take_jobs();
        app.handle_event(AppEvent::FilesIndexed {
            root: PathBuf::from("/work/duckx"),
            generation: 1,
            files: vec![
                IndexedFile {
                    relative: "src/app.rs".into(),
                    name_at: 4,
                },
                IndexedFile {
                    relative: "src/app/widget.rs".into(),
                    name_at: 8,
                },
            ],
            truncated: false,
        });
        let jobs = app.take_jobs();
        for job in jobs {
            app.handle_event(crate::jobs::run(job));
        }
        app.handle(SemanticAction::Picker(crate::action::PickerAction::Insert(
            "app".into(),
        )));
        let jobs = app.take_jobs();
        assert!(
            jobs.iter()
                .any(|job| matches!(job, Job::FilterFiles { .. }))
        );
        for job in jobs {
            app.handle_event(crate::jobs::run(job));
        }
        let screen = draw(&app, 80, 24);
        for corner in ["╭", "╮", "╰", "╯"] {
            assert!(screen.contains(corner), "missing rounded corner {corner}");
        }
        for square in ["┌", "┐", "└", "┘"] {
            assert!(!screen.contains(square), "found square corner {square}");
        }
        assert!(screen.contains("src/app.rs"), "{screen}");
        assert!(screen.contains("❯"), "{screen}");
        assert!(screen.contains("OPEN"), "{screen}");
        assert_eq!(app.buffer().rope().to_string(), "stay\n");
    }

    #[test]
    fn file_picker_cursor_follows_left_and_right() {
        let mut app = App::new(Buffer::from_text("stay\n"), None);
        app.handle(SemanticAction::OpenFilePicker);
        let picker = app.file_picker.as_mut().unwrap();
        picker.query = "hello".into();
        picker.cursor = picker.query.len();
        let end = cursor_at(&app, 80, 24);
        app.handle(SemanticAction::Picker(
            crate::action::PickerAction::CursorLeft,
        ));
        let on_o = cursor_at(&app, 80, 24);
        assert_eq!(on_o.1, end.1);
        assert_eq!(on_o.0 + 1, end.0, "left should move the block onto o");
        let symbol = cell_at(&app, 80, 24, on_o.0, on_o.1);
        assert_eq!(symbol, "o");
        app.handle(SemanticAction::Picker(
            crate::action::PickerAction::CursorLeft,
        ));
        let on_l = cursor_at(&app, 80, 24);
        assert_eq!(on_l.0 + 1, on_o.0);
        app.handle(SemanticAction::Picker(
            crate::action::PickerAction::CursorRight,
        ));
        assert_eq!(cursor_at(&app, 80, 24), on_o);
    }

    fn cursor_at(app: &App, width: u16, height: u16) -> (u16, u16) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        cursor::position().expect("cursor")
    }

    fn cell_at(app: &App, width: u16, height: u16, x: u16, y: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal.backend().buffer()[(x, y)].symbol().to_string()
    }

    #[test]
    fn command_bar_draws_a_folder_ghost_and_points_at_right() {
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.explorer = Explorer::new(PathBuf::from("/work/duckx-code"));
        app.explorer.open = true;
        app.explorer.focused = true;
        let dir = |path: &str| {
            let path = PathBuf::from(path);
            FileEntry {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                depth: path.components().count(),
                path,
                is_dir: true,
            }
        };
        app.explorer
            .set_entries(vec![dir("examples"), dir("extra")]);
        app.explorer.enter_select();
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "move ex".into(),
        )));
        let screen = draw(&app, 100, 16);
        let status = screen.lines().last().unwrap();
        assert!(status.contains("extra"), "{status}");
        assert!(screen.contains('→'), "{screen}");
        assert_eq!(app.command_bar.as_ref().unwrap().input, "move ex");
    }

    #[test]
    fn command_hints_stack_downward_above_the_status_line() {
        let mut app = App::new(Buffer::new(), None);
        app.handle(SemanticAction::OpenCommandBar);
        let screen = draw(&app, 100, 16);
        let lines: Vec<&str> = screen.lines().collect();
        let first = lines
            .iter()
            .position(|line| line.contains("file-picker"))
            .expect(&screen);
        assert!(
            lines[first].contains("Open a workspace file by name"),
            "{screen}"
        );
        assert!(lines[first + 1].contains("workspace"), "{screen}");
        assert!(lines[first + 1].contains("Open another folder"), "{screen}");
        assert!(lines[first + 2].contains("find"), "{screen}");
        assert!(
            lines[first + 2].contains("Find text in the file or explorer"),
            "{screen}"
        );
        assert!(lines[first + 3].contains("replace"), "{screen}");
        assert!(lines[first + 4].contains("goto <line>"), "{screen}");
        assert!(first + 4 < lines.len() - 1, "{screen}");
        assert!(!lines.last().unwrap().contains("[No Name]"));
    }

    #[test]
    fn hint_lists_matching_commands_in_order_above_the_input() {
        let mut app = App::new(Buffer::new(), None);
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "spli".into(),
        )));
        let screen = draw(&app, 80, 12);
        let lines: Vec<&str> = screen.lines().collect();
        let status = lines.len() - 1;
        let right = lines[status - 2];
        let down = lines[status - 1];
        assert!(right.contains("split-right"), "{right}");
        assert!(
            right.contains("Open an empty editor to the right"),
            "{right}"
        );
        assert!(down.contains("split-down"), "{down}");
        assert!(down.contains("below"), "{down}");
        assert!(lines[status].contains("spli"), "{screen}");
    }

    #[test]
    fn status_line_matches_lualine_layout() {
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        let mut app = App::new(
            Buffer::from_text(&lines.join("\n")),
            Some("docs/DESIGN.md".into()),
        );
        app.branch = Some("master".into());
        app.buffer_mut().goto_line(7);

        let screen = draw(&app, 100, 10);
        let status = screen.lines().last().unwrap();
        for expected in [
            " NORMAL ",
            "\u{e0a0} master",
            "\u{e73e}",
            "DESIGN.md",
            " 8%   8:1 ",
            "4 LF UTF-8",
        ] {
            assert!(
                status.contains(expected),
                "missing {expected:?} in {status:?}"
            );
        }
        assert!(!status.contains("DUCKX"), "{status}");
        assert!(!status.contains("COMMAND"), "{status}");
        assert!(status.find("NORMAL") < status.find("master"));
        assert!(status.find("master") < status.find("DESIGN.md"));
        assert!(status.find("8%") < status.find("4 LF"));
        assert!(!status.contains("[+]"));
    }

    #[test]
    fn status_line_without_repo_or_file() {
        let app = App::new(Buffer::new(), None);
        let status = draw(&app, 60, 3).lines().last().unwrap().to_string();
        assert!(status.contains("[No Name]"));
        assert!(status.contains("Top   1:1"));
        assert!(!status.contains("\u{e0a0}"));
        assert!(!status.contains("\u{f017}"));
    }

    #[test]
    fn focus_keeps_only_the_active_buffer_until_the_command_bar_opens() {
        let mut app = App::new(Buffer::from_text("alpha"), Some("left.rs".into()));
        app.handle(SemanticAction::ToggleSidebar);
        app.handle(SemanticAction::Sidebar(SidebarAction::Escape));
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "split-right".into(),
        )));
        app.handle(SemanticAction::CommandBar(CommandBarAction::Submit));
        for ch in "RIGHTTEXT".chars() {
            app.handle(SemanticAction::InsertChar(ch));
        }
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "focus".into(),
        )));
        app.handle(SemanticAction::CommandBar(CommandBarAction::Submit));

        let screen = draw(&app, 80, 12);
        let lines: Vec<&str> = screen.lines().collect();
        assert!(lines[0].contains("RIGHTTEXT"), "{screen}");
        assert!(!screen.contains("alpha"), "{screen}");
        assert!(!screen.contains("left.rs"), "{screen}");
        assert!(!screen.contains("[No Name]"), "{screen}");
        assert!(!screen.contains("NORMAL"), "{screen}");
        assert!(!screen.contains('╭'), "{screen}");

        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "go".into(),
        )));
        let open = draw(&app, 80, 12);
        let open_lines: Vec<&str> = open.lines().collect();
        assert!(open_lines[0].contains("RIGHTTEXT"), "{open}");
        assert!(open_lines.last().unwrap().contains("COMMAND"), "{open}");
        assert!(open.contains("Go to line number"), "{open}");
        assert!(!open.contains("NORMAL"), "{open}");
        assert!(!open.contains("left.rs"), "{open}");

        app.handle(SemanticAction::CommandBar(CommandBarAction::Close));
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "focus".into(),
        )));
        app.handle(SemanticAction::CommandBar(CommandBarAction::Submit));
        let restored = draw(&app, 80, 12);
        let restored_lines: Vec<&str> = restored.lines().collect();
        assert!(restored_lines[0].contains("left.rs"), "{restored}");
        assert!(restored.contains("alpha"), "{restored}");
        assert!(restored.contains("RIGHTTEXT"), "{restored}");
        assert!(
            restored_lines.last().unwrap().contains("NORMAL"),
            "{restored}"
        );
        assert!(restored.contains('╭'), "{restored}");
    }

    #[test]
    fn dirty_file_shows_modified_marker_on_the_title_and_status_bar() {
        let mut app = App::new(Buffer::from_text("hi"), Some("src/main.rs".into()));
        app.handle(SemanticAction::InsertChar('!'));
        let screen = draw(&app, 80, 8);
        let lines: Vec<&str> = screen.lines().collect();
        assert!(lines[0].contains("\u{e7a8} main.rs [+]"), "{screen}");
        assert!(lines.last().unwrap().contains("main.rs"), "{screen}");
        assert!(lines.last().unwrap().contains("[+]"), "{screen}");
    }

    #[test]
    fn pane_title_shows_an_icon_only_for_a_known_file_type() {
        use ratatui::style::{Color, Modifier};

        let draw_title = |app: &App| {
            let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
            terminal.draw(|frame| render(frame, app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            let title: String = (0..40).map(|x| buffer[(x, 0)].symbol()).collect();
            (title, buffer)
        };

        let (title, buffer) = draw_title(&App::new(
            Buffer::from_text("fn main() {}\n"),
            Some("hello.rs".into()),
        ));
        assert!(title.contains("\u{e7a8} hello.rs"), "{title}");
        let icon_x = (0..40)
            .find(|x| buffer[(*x, 0)].symbol() == "\u{e7a8}")
            .unwrap();
        let icon = &buffer[(icon_x, 0)];
        assert_eq!(icon.fg, Color::Rgb(0xde, 0xa5, 0x84));
        let name = &buffer[(icon_x + 2, 0)];
        assert_eq!(name.symbol(), "h");
        assert!(name.modifier.contains(Modifier::BOLD));

        let (title, _) = draw_title(&App::new(
            Buffer::from_text("x\n"),
            Some("notes.txt".into()),
        ));
        assert!(title.contains("notes.txt"), "{title}");
        assert!(!title.contains('\u{f15b}'), "{title}");
        assert!(!title.contains('\u{e7a8}'), "{title}");

        let (title, _) = draw_title(&App::new(Buffer::new(), None));
        assert!(title.contains("[No Name]"), "{title}");
        assert!(!title.contains('\u{f15b}'), "{title}");
    }

    #[test]
    fn each_pane_draws_its_tabs_on_the_title_row() {
        use ratatui::style::Modifier;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("duckx-ui-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("lib.rs"), "fn lib() {}\n").unwrap();

        let mut app = App::new(Buffer::new(), None);
        app.explorer = Explorer::new(dir.clone());
        app.explorer.open = true;
        app.explorer.focused = true;
        app.handle_event(AppEvent::TreeScanned {
            root: dir.clone(),
            entries: vec![
                FileEntry {
                    name: "main.rs".into(),
                    depth: 1,
                    path: PathBuf::from("main.rs"),
                    is_dir: false,
                },
                FileEntry {
                    name: "lib.rs".into(),
                    depth: 1,
                    path: PathBuf::from("lib.rs"),
                    is_dir: false,
                },
            ],
        });
        let open = |app: &mut App| {
            app.explorer.focused = true;
            app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
            for job in app.take_jobs() {
                let event = crate::jobs::run(job);
                app.handle_event(event);
            }
        };
        open(&mut app);
        app.handle(SemanticAction::Sidebar(SidebarAction::Down {
            extend: false,
        }));
        open(&mut app);

        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let title: String = (0..60).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(title.contains("main.rs"), "{title}");
        assert!(title.contains("lib.rs"), "{title}");
        let main_x = (0..60).find(|x| buffer[(*x, 0)].symbol() == "m").unwrap();
        let lib_x = (0..60).find(|x| buffer[(*x, 0)].symbol() == "l").unwrap();
        assert!(lib_x > main_x, "{title}");
        assert!(!buffer[(main_x, 0)].modifier.contains(Modifier::BOLD));
        assert!(buffer[(lib_x, 0)].modifier.contains(Modifier::BOLD));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsaved_prompt_is_a_rounded_dialog() {
        use ratatui::style::Modifier;

        let mut app = App::new(Buffer::from_text("hi"), Some("src/main.rs".into()));
        app.handle(SemanticAction::InsertChar('!'));
        app.handle(SemanticAction::Quit);
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..80).map(|x| buffer[(x, y)].symbol()).collect() };
        let screen = (0..16).map(|y| row(y)).collect::<Vec<_>>().join("\n");
        assert!(screen.contains("Unsaved"), "{screen}");
        assert!(screen.contains("before quitting"), "{screen}");
        assert!(screen.contains("Yes"), "{screen}");
        assert!(screen.contains("No"), "{screen}");
        assert!(screen.contains("Cancel"), "{screen}");
        assert!(!screen.contains("don't save"), "{screen}");
        assert!(!screen.contains("Esc"), "{screen}");
        for corner in ["╭", "╮", "╰", "╯"] {
            assert!(screen.contains(corner), "missing rounded corner {corner}");
        }
        for square in ["┌", "┐", "└", "┘"] {
            assert!(!screen.contains(square), "found square corner {square}");
        }

        let message_y = (0..16)
            .find(|y| row(*y).contains("before quitting"))
            .unwrap();
        let message = row(message_y);
        let left = message.find('│').unwrap();
        let right = message.rfind('│').unwrap();
        let inside = &message[left + '│'.len_utf8()..right];
        let lead = inside.chars().take_while(|ch| *ch == ' ').count();
        let trail = inside.chars().rev().take_while(|ch| *ch == ' ').count();
        assert_eq!(lead, 1, "message padding: {inside:?}");
        assert!(trail >= 1, "message padding: {inside:?}");
        let top = (0..message_y)
            .rev()
            .find(|y| row(*y).contains('╭'))
            .unwrap();
        assert_eq!(
            message_y,
            top + 1,
            "message sits on the row under the border"
        );
        let buttons_y = (0..16).find(|y| row(*y).contains("Cancel")).unwrap();
        let bottom = (buttons_y + 1..16).find(|y| row(*y).contains('╰')).unwrap();
        assert_eq!(
            bottom,
            buttons_y + 1,
            "buttons sit on the row above the border"
        );

        let buttons = row(buttons_y);
        let bleft = buttons.find('│').unwrap();
        let binside = &buttons[bleft + '│'.len_utf8()..buttons.rfind('│').unwrap()];
        let lead = binside.chars().take_while(|ch| *ch == ' ').count();
        let trail = binside.chars().rev().take_while(|ch| *ch == ' ').count();
        assert!(
            (lead as isize - trail as isize).abs() <= 1,
            "buttons should be centered: {binside:?}"
        );
        let column = |text: &str, needle: &str| -> u16 {
            text[..text.find(needle).unwrap()].chars().count() as u16
        };
        let y_cell = &buffer[(column(&buttons, "Yes"), buttons_y)];
        assert!(y_cell.modifier.contains(Modifier::UNDERLINED), "{y_cell:?}");
        assert_ne!(y_cell.bg, ratatui::style::Color::Reset, "{y_cell:?}");
        let n_cell = &buffer[(column(&buttons, "No"), buttons_y)];
        assert!(n_cell.modifier.contains(Modifier::UNDERLINED), "{n_cell:?}");
        assert_eq!(n_cell.bg, app.styles().editor_bg, "{n_cell:?}");
        let c_cell = &buffer[(column(&buttons, "Cancel"), buttons_y)];
        assert!(
            !c_cell.modifier.contains(Modifier::UNDERLINED),
            "{c_cell:?}"
        );
    }

    #[test]
    fn hover_popup_uses_rounded_corners() {
        let mut app = App::new(
            Buffer::from_text("fn main() {}\n"),
            Some(PathBuf::from("src/main.rs")),
        );
        app.handle_lsp(crate::lsp::LspEvent::Features {
            group: "rust-analyzer",
            hover: true,
            definition: true,
            completion: true,
            triggers: vec![".".into()],
            resolve: false,
            signature: false,
            rename: false,
            format: false,
            references: true,
            code_action: false,
            action_resolve: false,
        });
        app.handle(SemanticAction::Hover);
        let token = app
            .take_lsp_requests()
            .into_iter()
            .find_map(|request| match request {
                crate::lsp::LspRequest::Query { token, .. } => Some(token),
                _ => None,
            });
        app.handle_lsp(crate::lsp::LspEvent::Hover {
            token: token.expect("hover query"),
            text: Some("fn main".into()),
        });
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("Hover"), "{screen}");
        app.handle(SemanticAction::Complete);
        let token = app
            .take_lsp_requests()
            .into_iter()
            .find_map(|request| match request {
                crate::lsp::LspRequest::Query { token, .. } => Some(token),
                _ => None,
            });
        app.handle_lsp(crate::lsp::LspEvent::Completion {
            token: token.expect("completion query"),
            incomplete: false,
            items: vec![crate::lsp::protocol::CompletionItem {
                label: "println".into(),
                detail: "fn".into(),
                filter: "println".into(),
                insert: "println!".into(),
                kind: String::new(),
                docs: String::new(),
                snippet: false,
                resolve: None,
                replace: None,
                extra: Vec::new(),
            }],
        });
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("Completion"), "{screen}");
        assert!(screen.contains("println"), "{screen}");
        for corner in ["╭", "╮", "╰", "╯"] {
            assert!(screen.contains(corner), "missing {corner}\n{screen}");
        }
        for square in ["┌", "┐", "└", "┘"] {
            assert!(!screen.contains(square), "{screen}");
        }
    }

    #[test]
    fn references_popup_uses_rounded_corners() {
        let mut app = App::new(
            Buffer::from_text("fn main() {\n    work();\n}\nfn work() {}\n"),
            Some(PathBuf::from("src/main.rs")),
        );
        app.handle_lsp(crate::lsp::LspEvent::Features {
            group: "rust-analyzer",
            hover: false,
            definition: false,
            completion: false,
            triggers: Vec::new(),
            resolve: false,
            signature: false,
            rename: false,
            format: false,
            references: true,
            code_action: false,
            action_resolve: false,
        });
        app.handle(SemanticAction::GotoReferences);
        let token = app
            .take_lsp_requests()
            .into_iter()
            .find_map(|request| match request {
                crate::lsp::LspRequest::Query { token, .. } => Some(token),
                _ => None,
            });
        let place = |line, character| crate::lsp::protocol::JumpTarget {
            path: PathBuf::from("src/main.rs"),
            line,
            character,
            end_line: line,
            end_character: character + 4,
            encoding: crate::lsp::position::Encoding::Utf8,
        };
        app.handle_lsp(crate::lsp::LspEvent::References {
            token: token.expect("references query"),
            locations: vec![place(1, 4), place(3, 3)],
        });
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("References"), "{screen}");
        assert!(screen.contains("work()"), "{screen}");
        for corner in ["╭", "╮", "╰", "╯"] {
            assert!(screen.contains(corner), "missing {corner}\n{screen}");
        }
        for square in ["┌", "┐", "└", "┘"] {
            assert!(!screen.contains(square), "{screen}");
        }
    }

    #[test]
    fn panel_borders_are_rounded() {
        let mut app = App::new(Buffer::from_text("hi"), Some("src/main.rs".into()));
        app.handle(SemanticAction::InsertChar('!'));
        app.handle(SemanticAction::Quit);
        let screen = draw(&app, 80, 16);
        for corner in ["╭", "╮", "╰", "╯"] {
            assert!(screen.contains(corner), "missing rounded corner {corner}");
        }
        for square in ["┌", "┐", "└", "┘"] {
            assert!(!screen.contains(square), "found square corner {square}");
        }
    }

    fn app_with_sidebar() -> App {
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.explorer = Explorer::new(PathBuf::from("/work/duckx-code"));
        app.handle(SemanticAction::ToggleSidebar);
        let entry = |path: &str, is_dir| {
            let path = PathBuf::from(path);
            FileEntry {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                depth: path.components().count(),
                path,
                is_dir,
            }
        };
        app.handle_event(AppEvent::TreeScanned {
            root: app.explorer.root.clone(),
            entries: vec![
                entry("src", true),
                entry("src/main.rs", false),
                entry("Cargo.toml", false),
            ],
        });
        app.handle_event(AppEvent::GitStatus {
            root: app.explorer.root.clone(),
            statuses: [(PathBuf::from("Cargo.toml"), GitStatus::Modified)].into(),
        });
        app
    }

    #[test]
    fn sidebar_shows_filter_root_tree_and_git_status() {
        let mut app = app_with_sidebar();
        app.handle(SemanticAction::Sidebar(SidebarAction::Expand {
            extend: false,
        }));
        let screen = draw(&app, 80, 12);
        let lines: Vec<&str> = screen.lines().collect();
        assert!(lines[0].starts_with('╭') && !lines[0].contains("Explorer"));
        assert!(lines[1].starts_with("│ \u{f002} ") && lines[1].contains("3/3"));
        assert!(!lines[1].contains('❯'));
        assert!(lines[3].contains("duckx-code"));
        assert!(lines[4].contains("├╴\u{f07c} src"));
        assert!(lines[5].contains("│ └╴\u{e7a8} main.rs"));
        assert!(lines[6].contains("└╴\u{e615} Cargo.toml"));
        let cargo = lines[6].chars().take(30).collect::<String>();
        assert!(cargo.trim_end().ends_with('M'), "{cargo:?}");
        assert!(lines[0].chars().nth(30).is_some_and(|c| c != '╮'));
        assert!(lines[11].contains("NORMAL"));
    }

    #[test]
    fn split_right_draws_both_panes_and_a_divider() {
        let mut app = App::new(Buffer::from_text("alpha\n"), None);
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "split-right".into(),
        )));
        app.handle(SemanticAction::CommandBar(CommandBarAction::Submit));
        let screen = draw(&app, 40, 8);
        assert!(screen.contains("alpha"), "{screen}");
        assert!(screen.contains('│'), "{screen}");
        assert!(!screen.contains('┌'));
    }

    #[test]
    fn closed_sidebar_takes_no_space() {
        let mut app = app_with_sidebar();
        app.handle(SemanticAction::ToggleSidebar);
        assert!(!app.explorer.open);
        let screen = draw(&app, 60, 5);
        let lines: Vec<&str> = screen.lines().collect();
        assert!(!screen.contains("Explorer"));
        assert!(lines[0].contains("[No Name]"));
        assert!(lines[1].starts_with("   1 fn main()"));
    }

    #[test]
    fn each_pane_shows_its_file_name() {
        let mut app = App::new(Buffer::from_text("alpha\n"), Some("src/main.rs".into()));
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "split-down".into(),
        )));
        app.handle(SemanticAction::CommandBar(CommandBarAction::Submit));

        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let screen = (0..10)
            .map(|y| (0..40).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        let lines: Vec<&str> = screen.lines().collect();
        assert!(lines[0].contains("\u{e7a8} main.rs"), "{screen}");
        assert!(
            lines.iter().any(|line| line.contains("[No Name]")),
            "{screen}"
        );
        assert!(!lines[0].contains('╭'), "{screen}");

        let name_x = (0..40)
            .find(|x| buffer[(*x, 0)].symbol() == "m")
            .expect("file name");
        let original = buffer[(name_x, 0)].style();
        assert!(
            !original
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
        let active_row = lines
            .iter()
            .position(|line| line.contains("[No Name]"))
            .unwrap();
        let active = buffer[(1, active_row as u16)].style();
        assert!(active.add_modifier.contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn rendering_tiny_terminals_does_not_panic() {
        let mut app = App::new(Buffer::from_text("日本語\tx\n"), None);
        app.handle(SemanticAction::OpenCommandBar);
        for (w, h) in [(0, 0), (1, 1), (5, 2), (19, 3), (20, 4), (30, 6)] {
            draw(&app, w, h);
        }
        let mut app = app_with_sidebar();
        for (w, h) in [(0, 0), (1, 1), (5, 2), (19, 3), (20, 4), (30, 6)] {
            draw(&app, w, h);
        }
        app.handle(SemanticAction::Sidebar(SidebarAction::FilterInsert(
            "main".into(),
        )));
        draw(&app, 20, 4);
    }

    #[test]
    fn about_page_shows_the_duck_and_rounded_corners() {
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "about".into(),
        )));
        app.handle(SemanticAction::CommandBar(CommandBarAction::Submit));
        assert_eq!(app.focus(), crate::action::Focus::About);

        let screen = draw(&app, 80, 24);
        assert!(screen.contains("About"), "{screen}");
        assert!(screen.contains("▄▀▀▀▀▀▄"), "{screen}");
        assert!(screen.contains("CODE"), "{screen}");
        assert!(screen.contains("Duckx Code"), "{screen}");
        let lines: Vec<&str> = screen.lines().collect();
        let column = |line: &str, needle: &str| {
            let byte = line.find(needle).unwrap();
            line[..byte].chars().count()
        };
        let top = lines
            .iter()
            .position(|line| line.contains("▄▀▀▀▀▀▄"))
            .unwrap();
        let code = lines.iter().position(|line| line.contains("CODE")).unwrap();
        let wing = lines
            .iter()
            .position(|line| line.contains("█▀▀▀▀▀▀"))
            .unwrap();
        let beak = column(lines[top], "▄");
        assert_eq!(
            column(lines[code], "CODE") as isize - beak as isize,
            8,
            "{screen}"
        );
        assert_eq!(
            beak as isize - column(lines[wing], "█") as isize,
            6,
            "{screen}"
        );
        assert!(screen.contains(env!("CARGO_PKG_VERSION")), "{screen}");
        assert!(screen.contains("75%"), "{screen}");
        assert!(screen.contains("esc"), "{screen}");
        assert!(screen.contains("ABOUT"), "{screen}");
        for corner in ["╭", "╮", "╰", "╯"] {
            assert!(screen.contains(corner), "missing rounded corner {corner}");
        }
        for square in ["┌", "┐", "└", "┘"] {
            assert!(!screen.contains(square), "found square corner {square}");
        }
        for (w, h) in [(8, 4), (20, 8), (40, 12), (80, 24)] {
            draw(&app, w, h);
        }
    }

    #[test]
    fn settings_page_is_a_rounded_preview() {
        use crate::action::SettingsAction;

        let mut app = App::new(Buffer::from_text("fn main() {}\n"), None);
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "config".into(),
        )));
        app.handle(SemanticAction::CommandBar(CommandBarAction::Submit));
        assert_eq!(app.focus(), crate::action::Focus::Settings);

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..80).map(|x| buffer[(x, y)].symbol()).collect() };
        let screen = (0..24).map(|y| row(y)).collect::<Vec<_>>().join("\n");
        assert!(screen.contains("Settings"), "{screen}");
        assert!(screen.contains("Editor"), "{screen}");
        assert!(screen.contains("Appearance"), "{screen}");
        assert!(screen.contains("Tab width"), "{screen}");
        assert!(screen.contains("│"), "{screen}");
        assert!(screen.contains("enter"), "{screen}");
        assert!(screen.contains("esc"), "{screen}");
        assert!(screen.contains("back"), "{screen}");
        assert!(row(23).contains("SETTINGS"), "{screen}");
        for corner in ["╭", "╮", "╰", "╯"] {
            assert!(screen.contains(corner), "missing rounded corner {corner}");
        }
        for square in ["┌", "┐", "└", "┘"] {
            assert!(!screen.contains(square), "found square corner {square}");
        }

        let editor = (0..24).find(|y| row(*y).contains("Editor")).unwrap();
        let appearance = (0..24).find(|y| row(*y).contains("Appearance")).unwrap();
        let col = row(editor).find("Editor").unwrap() as u16;
        assert_eq!(buffer[(col, editor)].bg, app.styles().sidebar_selected);
        let other = row(appearance).find("Appearance").unwrap() as u16;
        assert_ne!(
            buffer[(other, appearance)].bg,
            app.styles().sidebar_selected
        );
        let tab = (0..24).find(|y| row(*y).contains("Tab width")).unwrap();
        let tab_col = row(tab).find("Tab width").unwrap() as u16;
        assert_ne!(buffer[(tab_col, tab)].bg, app.styles().sidebar_selected);

        for (w, h) in [(8, 4), (20, 6), (40, 10), (80, 24)] {
            draw(&app, w, h);
        }
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("Tab width"), "{screen}");
        assert!(screen.contains("Spaces inserted for a Tab."), "{screen}");
        assert!(screen.contains("4"), "{screen}");
        assert!(screen.contains("Appearance"), "{screen}");
        let opened = draw(&app, 80, 24);
        let tab_line = opened
            .lines()
            .find(|line| line.contains("Tab width"))
            .unwrap();
        assert!(tab_line.contains('4'), "{tab_line}");
        assert!(tab_line.contains('▾'), "{tab_line}");

        app.handle(SemanticAction::Settings(SettingsAction::Back));
        app.handle(SemanticAction::Settings(SettingsAction::Down));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("Color theme"), "{screen}");
        assert!(screen.contains("Tokyo Night"), "{screen}");
        assert!(screen.contains('▾'), "{screen}");
        assert!(screen.contains("Syntax colors"), "{screen}");
        assert!(screen.contains("fn"), "{screen}");
        assert!(!screen.contains("Placeholders"), "{screen}");
        assert!(!screen.contains("Tab width"), "{screen}");

        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        let screen = draw(&app, 80, 24);
        assert!(screen.contains('▴'), "{screen}");
        assert!(screen.contains("Catppuccin Latte"), "{screen}");
        assert!(screen.contains("Catppuccin Frappé"), "{screen}");
        assert!(screen.contains("Catppuccin Macchiato"), "{screen}");
        assert!(screen.contains("Catppuccin Mocha"), "{screen}");
        assert!(screen.contains("↑↓  enter  esc"), "{screen}");
        assert!(screen.contains('╭'), "{screen}");
        for square in ["┌", "┐", "└", "┘"] {
            assert!(!screen.contains(square), "found square corner {square}");
        }
        assert_eq!(app.appearance.color.id, "tokyo-night");
        app.handle(SemanticAction::Settings(SettingsAction::Down));
        assert_eq!(app.appearance.color.id, "catppuccin-latte");
        app.handle(SemanticAction::Settings(SettingsAction::Back));
        assert_eq!(app.appearance.color.id, "tokyo-night");
        assert!(
            app.settings
                .as_ref()
                .is_some_and(|page| page.menu.is_none())
        );

        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        app.handle(SemanticAction::Settings(SettingsAction::Down));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        assert_eq!(app.appearance.color.id, "catppuccin-latte");
        assert!(
            app.settings
                .as_ref()
                .is_some_and(|page| page.menu.is_none())
        );
        let screen = draw(&app, 80, 24);
        assert!(screen.contains("Catppuccin Latte"), "{screen}");
        assert!(!screen.contains("Catppuccin Mocha"), "{screen}");

        app.handle(SemanticAction::Settings(SettingsAction::Down));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        app.handle(SemanticAction::Settings(SettingsAction::Down));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        assert_eq!(app.appearance.syntax.id, "catppuccin-latte");
        assert_eq!(app.appearance.color.id, "catppuccin-latte");
    }

    #[test]
    fn explorer_command_bar_hides_editor_commands() {
        let mut app = app_with_sidebar();
        app.handle(SemanticAction::OpenCommandBar);
        let screen = draw(&app, 90, 16);
        assert!(screen.contains("select"), "{screen}");
        assert!(!screen.contains("delete"), "{screen}");
        assert!(screen.contains("Enter FILE mode"), "{screen}");
        assert!(!screen.contains("goto"), "{screen}");
        assert!(!screen.contains("split-right"), "{screen}");

        app.command_bar = None;
        app.explorer.selecting = true;
        app.handle(SemanticAction::OpenCommandBar);
        let screen = draw(&app, 90, 16);
        assert!(screen.contains("delete"), "{screen}");
        assert!(screen.contains("move"), "{screen}");
        assert!(screen.contains("rename"), "{screen}");
    }

    #[test]
    fn rust_fn_uses_the_keyword_color_and_selection_keeps_it() {
        use crate::theme::SyntaxRole;

        let mut app = App::new(
            Buffer::from_text("fn main() {}\n"),
            Some("src/main.rs".into()),
        );
        for job in app.take_jobs() {
            app.handle_event(crate::jobs::run(job));
        }
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let keyword = app.appearance.syntax.color(SyntaxRole::Keyword);
        let found = (0..40u16).find_map(|x| {
            (0..8u16).find_map(|y| {
                let cell = &buffer[(x, y)];
                (cell.symbol() == "f" && cell.fg == keyword).then_some((x, y))
            })
        });
        let (x, y) = found.expect("fn should be painted with the keyword color");
        assert_eq!(buffer[(x + 1, y)].symbol(), "n");
        assert_eq!(buffer[(x + 1, y)].fg, keyword);

        app.buffer_mut().select_span(0, 2);
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let cell = &buffer[(x, y)];
        assert_eq!(cell.fg, keyword);
        let selection = app.styles().selection.bg.expect("selection background");
        assert_eq!(cell.bg, selection);
    }

    #[test]
    fn diagnostics_mark_the_gutter_underline_the_range_and_fill_the_status_bar() {
        use crate::lsp::{Diagnostic, LspEvent, ServerStatus, Severity};
        use ratatui::style::Modifier;

        let mut app = App::new(
            Buffer::from_text("let x = 1;\nlet y = 2;\n"),
            Some(PathBuf::from("main.rs")),
        );
        app.take_lsp_requests();
        let path = app.path().unwrap().to_path_buf();
        app.handle_lsp(LspEvent::Diagnostics {
            path,
            version: 1,
            diagnostics: vec![Diagnostic {
                start: 15,
                end: 16,
                severity: Severity::Warning,
                message: "unused variable: `y`".into(),
                source: None,
                code: None,
            }],
        });
        app.handle_lsp(LspEvent::Status {
            group: "rust-analyzer",
            name: "rust-analyzer",
            status: ServerStatus::Ready,
        });
        app.handle(SemanticAction::GotoDiagnostic { next: true });

        let mut terminal = Terminal::new(TestBackend::new(160, 6)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = |y: u16| -> String { (0..160).map(|x| buffer[(x, y)].symbol()).collect() };
        let warning = app.styles().diagnostic_warning;

        let line_two = (0..6)
            .find(|y| row(*y).contains("let y"))
            .expect("second line drawn");
        let text = row(line_two);
        assert!(text.trim_start().starts_with("W"), "{text}");
        let marker_x = (0..160)
            .find(|x| buffer[(*x, line_two)].symbol() == "W")
            .unwrap();
        assert_eq!(buffer[(marker_x, line_two)].fg, warning);
        let first = (0..6).find(|y| row(*y).contains("let x")).unwrap();
        assert!(!row(first).contains('W'), "{}", row(first));

        let y_x = (0..160)
            .find(|x| buffer[(*x, line_two)].symbol() == "y")
            .unwrap();
        let cell = &buffer[(y_x, line_two)];
        assert!(cell.modifier.contains(Modifier::BOLD));
        assert_eq!(cell.fg, warning);
        assert!(!cell.modifier.contains(Modifier::UNDERLINED));
        assert!(
            !buffer[(y_x - 1, line_two)]
                .modifier
                .contains(Modifier::BOLD)
        );

        let status = row(5);
        assert!(!status.contains("unused variable"), "{status}");
        assert!(status.contains("W 1"), "{status}");
        assert!(status.contains("rust-analyzer"), "{status}");

        let bar = row(4);
        assert!(bar.contains("unused variable: `y`"), "{bar}");
        assert!(bar.trim_end().ends_with('1'), "{bar}");
        let message_x = (0..160)
            .find(|x| buffer[(*x, 4)].symbol() == "u")
            .expect("message on the diagnostic row");
        assert_eq!(buffer[(message_x, 4)].bg, warning);
        assert_eq!(buffer[(message_x, 4)].fg, ratatui::style::Color::Black);
    }

    #[test]
    fn diff_gutter_paints_changes_in_the_git_colors() {
        use crate::action::{CommandBarAction, SemanticAction};
        use crate::jobs::{AppEvent, Job};
        use crate::services::git::{DiffResult, diff_lines};

        let mut app = App::new(
            Buffer::from_text("keep\nchanged\nnext\nadded\n"),
            Some("a.rs".into()),
        );
        app.handle(SemanticAction::OpenCommandBar);
        app.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "diff".into(),
        )));
        app.handle(SemanticAction::CommandBar(CommandBarAction::Submit));
        let (tab_id, path, generation, fingerprint) = app
            .take_jobs()
            .into_iter()
            .find_map(|job| match job {
                Job::GitDiff {
                    tab_id,
                    path,
                    generation,
                    fingerprint,
                    ..
                } => Some((tab_id, path, generation, fingerprint)),
                _ => None,
            })
            .expect("diff job");
        app.handle_event(AppEvent::GitDiff {
            tab_id,
            path,
            generation,
            fingerprint,
            result: DiffResult::Diff(diff_lines(
                &["keep".into(), "old".into(), "gone".into(), "next".into()],
                &[
                    "keep".into(),
                    "changed".into(),
                    "next".into(),
                    "added".into(),
                ],
            )),
        });

        let mut terminal = Terminal::new(TestBackend::new(80, 8)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = |y: u16| -> String { (0..80).map(|x| buffer[(x, y)].symbol()).collect() };
        let at = |text: &str| {
            (0..8)
                .find(|y| row(*y).contains(text))
                .unwrap_or_else(|| panic!("{text} missing in {}", row(1)))
        };
        let changed = at("changed");
        let next = at("next");
        let added = at("added");
        assert_eq!(buffer[(0, changed)].symbol(), "┃");
        assert_eq!(buffer[(0, changed)].fg, app.styles().git_modified);
        assert_eq!(buffer[(0, next)].symbol(), "‾");
        assert_eq!(buffer[(0, next)].fg, app.styles().git_deleted);
        assert_eq!(buffer[(0, added)].symbol(), "┃");
        assert_eq!(buffer[(0, added)].fg, app.styles().git_added);
        assert!(row(7).contains("diff"), "{}", row(7));

        let mut counted = App::new(Buffer::from_text("a\nd\n"), Some("b.rs".into()));
        counted.handle(SemanticAction::OpenCommandBar);
        counted.handle(SemanticAction::CommandBar(CommandBarAction::Insert(
            "diff".into(),
        )));
        counted.handle(SemanticAction::CommandBar(CommandBarAction::Submit));
        let (tab_id, path, generation, fingerprint) = counted
            .take_jobs()
            .into_iter()
            .find_map(|job| match job {
                Job::GitDiff {
                    tab_id,
                    path,
                    generation,
                    fingerprint,
                    ..
                } => Some((tab_id, path, generation, fingerprint)),
                _ => None,
            })
            .unwrap();
        counted.handle_event(AppEvent::GitDiff {
            tab_id,
            path,
            generation,
            fingerprint,
            result: DiffResult::Diff(diff_lines(
                &["a".into(), "b".into(), "c".into(), "d".into()],
                &["a".into(), "d".into()],
            )),
        });
        terminal.draw(|frame| render(frame, &counted)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = |y: u16| -> String { (0..80).map(|x| buffer[(x, y)].symbol()).collect() };
        let deleted = (0..8).find(|y| row(*y).contains('d')).unwrap();
        assert_eq!(buffer[(0, deleted)].symbol(), "2");
        assert_eq!(buffer[(0, deleted)].fg, counted.styles().git_deleted);
    }
}
