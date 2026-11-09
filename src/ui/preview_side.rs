//! The side picker that bare `preview` opens: a small rounded compass of four arrows.

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::panel;
use crate::action::Side;
use crate::app::App;

const WIDTH: u16 = 34;
const HEIGHT: u16 = 8;

/// Label drawn for each side. Arrows sit on the outside edge, like a compass.
pub fn label(side: Side) -> &'static str {
    match side {
        Side::Up => "↑ Up",
        Side::Down => "↓ Down",
        Side::Left => "← Left",
        Side::Right => "Right →",
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(picker) = app.preview_picker() else {
        return;
    };
    if area.width < 12 || area.height < 7 {
        return;
    }
    let theme = *app.styles();
    let canvas = Style::new().bg(theme.editor_bg);
    let name = app
        .tab_label(picker.source)
        .unwrap_or_else(|| "Markdown".to_string());
    let title = format!(" Preview {name} ");
    let width = WIDTH.max(title.width() as u16 + 4).min(area.width);
    let height = HEIGHT.min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let block = panel(&title)
        .border_style(theme.command_border)
        .title_style(theme.dialog_message)
        .style(canvas);
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let item = |side: Side| {
        let style = if side == picker.selected {
            theme.hint_active_name
        } else {
            theme.dialog_button.bg(theme.editor_bg)
        };
        Span::styled(format!(" {} ", label(side)), style)
    };
    let room = inner.width as usize;
    let left = item(Side::Left);
    let right = item(Side::Right);
    let gap = room.saturating_sub(left.width() + right.width() + 4).max(1);
    let middle = Line::from(vec![
        Span::styled("  ", canvas),
        left,
        Span::styled(" ".repeat(gap), canvas),
        right,
        Span::styled("  ", canvas),
    ]);
    let lines = vec![
        Line::raw(""),
        Line::from(item(Side::Up)).alignment(Alignment::Center),
        middle.alignment(Alignment::Center),
        Line::from(item(Side::Down)).alignment(Alignment::Center),
        Line::raw(""),
        Line::from(Span::styled(
            "←↑↓→ open · enter · esc close",
            theme.hint_dim.bg(theme.editor_bg),
        ))
        .alignment(Alignment::Center),
    ];
    frame.render_widget(Paragraph::new(lines).style(canvas), inner);
}
