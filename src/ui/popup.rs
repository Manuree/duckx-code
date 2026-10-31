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

fn render_references(
    frame: &mut Frame,
    text: Rect,
    view: &crate::app::ReferenceView,
    app: &App,
    theme: &UiStyles,
) {
    let width = 64.min(text.width);
    let height = (view.titles.len() as u16 + 2).min(text.height);
    let area = place(text, width, height, app);
    let style = Style::new().fg(theme.editor_fg).bg(theme.editor_bg);
    let rows: Vec<Line> = view
        .titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            let row_style = if index == view.selected {
                theme.selection
            } else {
                style
            };
            Line::from(Span::styled(title.clone(), row_style))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(rows).block(panel("References").style(style)),
        area,
    );
}

fn render_actions(
    frame: &mut Frame,
    text: Rect,
    view: &crate::app::ActionView,
    app: &App,
    theme: &UiStyles,
) {
    let width = MENU_WIDTH.min(text.width);
    let height = (view.titles.len() as u16 + 2).min(text.height);
    let area = place(text, width, height, app);
    let style = Style::new().fg(theme.editor_fg).bg(theme.editor_bg);
    let rows: Vec<Line> = view
        .titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            let row_style = if index == view.selected {
                theme.hint_active_name
            } else {
                style
            };
            Line::from(Span::styled(title.clone(), row_style))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(rows).block(panel("Actions").style(style)),
        area,
    );
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
    let width = MENU_WIDTH.min(text.width);
    let docs = !view.docs.is_empty();
    let height = (view.rows.len() as u16 + 2 + u16::from(docs)).min(text.height);
    let area = place(text, width, height, app);
    let base = Style::new().fg(theme.editor_fg).bg(theme.editor_bg);
    let inner_width = area.width.saturating_sub(2) as usize;
    let rows: Vec<Line> = view
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let style = if index == view.selected {
                theme.selection
            } else {
                base
            };
            let label_width = unicode_width::UnicodeWidthStr::width(row.label.as_str());
            let gap = inner_width.saturating_sub(label_width).min(2);
            let detail_room = inner_width.saturating_sub(label_width + gap);
            let detail: String = row.detail.chars().take(detail_room).collect();
            let pad = " ".repeat(gap);
            Line::from(vec![
                Span::styled(row.label.clone(), style),
                Span::styled(pad, style),
                Span::styled(
                    detail,
                    if index == view.selected {
                        style
                    } else {
                        theme.dim
                    },
                ),
            ])
        })
        .collect();
    let mut rows = rows;
    if docs {
        let doc: String = view.docs.chars().take(inner_width).collect();
        rows.push(Line::from(Span::styled(doc, theme.dim)));
    }
    frame.render_widget(
        Paragraph::new(rows).block(panel("Completion").style(base)),
        area,
    );
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
