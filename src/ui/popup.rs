//! Hover and completion popups. Both use the shared rounded panel.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::App;
use crate::theme::UiStyles;

use super::dropdown::{List, Row};
use super::panel;

const HOVER_LINES: usize = 12;
const MENU_WIDTH: u16 = 48;

pub fn render(frame: &mut Frame, app: &App, text: Option<Rect>, theme: &UiStyles) {
    let Some(area) = text.filter(|area| area.width > 4 && area.height > 2) else {
        return;
    };
    if let Some(body) = app.signature_text() {
        render_signature(frame, area, body, app, theme);
    }
    if let Some(body) = app.hover_text() {
        render_hover(frame, area, body, app, theme);
    }
    if let Some(view) = app.completion_view() {
        render_completion(frame, area, &view, app, theme);
    }
    if let Some(view) = app.reference_view() {
        render_references(frame, area, &view, app, theme);
    }
    if let Some(view) = app.action_view() {
        render_actions(frame, area, &view, app, theme);
    }
}

const LIST_KEYS: &str = "↑↓ enter esc";

/// The `convert-keyboard` dropdown: each layout with what the text becomes on it.
/// It drops under the cursor in the editor, or sits just above the assistant
/// panel when the text is the assistant prompt.
pub fn render_layout_menu(
    frame: &mut Frame,
    app: &App,
    text: Option<Rect>,
    agent: Rect,
    theme: &UiStyles,
) {
    let Some(view) = app.layout_menu_view() else {
        return;
    };
    let rows: Vec<Row> = view
        .rows
        .iter()
        .map(|(name, preview)| Row::new(name.clone()).tag(preview.clone()))
        .collect();
    let list = List::new("Convert to", LIST_KEYS, &rows).selected(Some(view.selected));
    let height = rows.len() as u16 + 2;
    let area = match (view.target, text) {
        (crate::app::ConvertTarget::Agent, _) if agent.height > 0 => {
            let full = frame.area();
            let width = list.natural_width().min(full.width);
            Rect {
                x: agent
                    .x
                    .saturating_add(1)
                    .min(full.width.saturating_sub(width)),
                y: agent.y.saturating_sub(height),
                width,
                height: height.min(agent.y.max(1)),
            }
        }
        (_, Some(text)) if text.width > 4 && text.height > 2 => {
            let width = list.natural_width().min(text.width);
            place(text, width, height.min(text.height), app)
        }
        _ => return,
    };
    super::dropdown::render(frame, area, &list, theme);
}
const COMPLETION_KEYS: &str = "↑↓ tab enter esc";

fn render_references(
    frame: &mut Frame,
    text: Rect,
    view: &crate::app::ReferenceView,
    app: &App,
    theme: &UiStyles,
) {
    let rows: Vec<Row> = view
        .titles
        .iter()
        .map(|title| Row::new(title.clone()))
        .collect();
    let list = List::new("References", LIST_KEYS, &rows).selected(Some(view.selected));
    let width = 64.min(text.width);
    let height = (rows.len() as u16 + 2).min(text.height);
    let area = place(text, width, height, app);
    super::dropdown::render(frame, area, &list, theme);
}

fn render_actions(
    frame: &mut Frame,
    text: Rect,
    view: &crate::app::ActionView,
    app: &App,
    theme: &UiStyles,
) {
    let rows: Vec<Row> = view
        .titles
        .iter()
        .map(|title| Row::new(title.clone()))
        .collect();
    let list = List::new("Actions", LIST_KEYS, &rows).selected(Some(view.selected));
    let width = MENU_WIDTH.min(text.width);
    let height = (rows.len() as u16 + 2).min(text.height);
    let area = place(text, width, height, app);
    super::dropdown::render(frame, area, &list, theme);
}

fn render_signature(frame: &mut Frame, text: Rect, body: &str, app: &App, theme: &UiStyles) {
    let line = body.lines().next().unwrap_or("").trim();
    let width = (unicode_width::UnicodeWidthStr::width(line) as u16 + 2).clamp(12, text.width);
    let area = place_above(text, width, 3, app);
    let style = Style::new().fg(theme.editor_fg).bg(theme.editor_bg);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(line.to_string(), style)))
            .block(panel("Signature").style(style)),
        area,
    );
}

fn render_hover(frame: &mut Frame, text: Rect, body: &str, app: &App, theme: &UiStyles) {
    let lines: Vec<&str> = body.lines().take(HOVER_LINES).collect();
    let width = lines
        .iter()
        .map(|line| unicode_width::UnicodeWidthStr::width(*line))
        .max()
        .unwrap_or(0)
        .clamp(8, (text.width as usize).saturating_sub(2)) as u16
        + 2;
    let height = (lines.len() as u16 + 2).min(text.height);
    let area = place(text, width.min(text.width), height, app);
    let style = Style::new().fg(theme.editor_fg).bg(theme.editor_bg);
    let rows: Vec<Line> = lines
        .into_iter()
        .map(|line| Line::from(Span::styled(line.to_string(), style)))
        .collect();
    frame.render_widget(
        Paragraph::new(rows).block(panel("Hover").style(style)),
        area,
    );
}

fn render_completion(
    frame: &mut Frame,
    text: Rect,
    view: &crate::app::CompletionView,
    app: &App,
    theme: &UiStyles,
) {
    let rows: Vec<Row> = view
        .rows
        .iter()
        .map(|row| Row::new(row.label.clone()).detail(row.detail.clone()))
        .collect();
    let list = List::new("Completion", COMPLETION_KEYS, &rows)
        .selected(Some(view.selected))
        .note(&view.docs);
    let width = MENU_WIDTH.min(text.width);
    let docs = !view.docs.is_empty();
    let height = (rows.len() as u16 + 2 + u16::from(docs)).min(text.height);
    let area = place(text, width, height, app);
    super::dropdown::render(frame, area, &list, theme);
}

fn place_above(text: Rect, width: u16, height: u16, app: &App) -> Rect {
    let (line, col) = app.buffer().cursor_line_col();
    let view = app.view();
    let row = line
        .saturating_sub(view.scroll_y)
        .min(text.height.saturating_sub(1) as usize) as u16;
    let column = col
        .saturating_sub(view.scroll_x)
        .min(text.width.saturating_sub(1) as usize) as u16;
    let x = text.x + column.min(text.width.saturating_sub(width));
    let above = text.y + row.saturating_sub(height);
    Rect {
        x,
        y: above,
        width,
        height: height.min(text.height),
    }
}

/// Below the cursor when it fits, otherwise above it, clamped to the text area.
fn place(text: Rect, width: u16, height: u16, app: &App) -> Rect {
    let (line, col) = app.buffer().cursor_line_col();
    let view = app.view();
    let row = line
        .saturating_sub(view.scroll_y)
        .min(text.height.saturating_sub(1) as usize) as u16;
    let column = col
        .saturating_sub(view.scroll_x)
        .min(text.width.saturating_sub(1) as usize) as u16;
    let x = text.x + column.min(text.width.saturating_sub(width));
    let below = text.y + row + 1;
    let y = if below + height <= text.y + text.height {
        below
    } else {
        text.y + row.saturating_sub(height)
    };
    Rect {
        x,
        y,
        width,
        height,
    }
}
