//! A short notice above the status bar: a transient message or a hidden
//! assistant reply. The corners are rounded, like every panel, and the border
//! and title take the notice's color (error, warning, success, info).

use ratatui::{Frame, layout::Rect, style::Style, text::Line, widgets::Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, NoticeKind};

use super::panel;

const MIN_WIDTH: u16 = 24;
const MAX_WIDTH: u16 = 72;

/// Draws the notice at the bottom-right, sitting on `floor` (the status bar's top).
pub fn render(frame: &mut Frame, app: &App, area: Rect, floor: u16) {
    let Some(bar) = app.snackbar.as_ref() else {
        return;
    };
    let height = 3u16;
    if area.width < MIN_WIDTH || floor < area.y.saturating_add(height) {
        return;
    }
    let inner_limit = (MAX_WIDTH as usize).saturating_sub(4);
    let shown = fit(&bar.text, inner_limit);
    let width = (shown.width() + 4)
        .clamp(MIN_WIDTH as usize, MAX_WIDTH as usize)
        .min(area.width as usize) as u16;
    let x = area.x + area.width.saturating_sub(width + 1);
    let y = floor - height;
    let rect = Rect::new(x, y, width, height);
    let theme = *app.styles();
    let color = match bar.kind {
        NoticeKind::Error => theme.diagnostic_error,
        NoticeKind::Warning => theme.diagnostic_warning,
        NoticeKind::Success => theme.git_added,
        NoticeKind::Info => theme.diagnostic_info,
    };
    let block = panel(bar.title)
        .border_style(Style::new().fg(color))
        .title_style(Style::new().fg(color))
        .style(Style::new().fg(theme.editor_fg).bg(theme.editor_bg))
        .padding(ratatui::widgets::Padding::horizontal(1));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width > 0 && inner.height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(shown))
                .style(Style::new().fg(theme.editor_fg).bg(theme.editor_bg)),
            inner,
        );
    }
}

fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        if out.width() + ch.width().unwrap_or(0) + 1 > width {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}
