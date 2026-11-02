use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::panel;
use crate::agent::wrap_rows;
use crate::app::App;

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Bottom assistant panel. Messages and the prompt wrap. The prompt keeps the bottom rows.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let theme = *app.styles();
    let focused = app.focus() == crate::action::Focus::Agent;
    let border = if focused {
        theme.sidebar_border_focused
    } else {
        theme.sidebar_border
    };
    let title = format!(" Assistant · {} ", fit(&status_label(app), 40));
    let block = panel(&title).border_style(border).style(theme.sidebar);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let content = width.saturating_sub(3).max(1);
    let prompt_rows = wrap_rows(&app.agent.prompt, content);
    let prompt_limit = (inner.height as usize).saturating_sub(1).max(1);
    let prompt_height = prompt_rows
        .len()
        .clamp(1, prompt_limit.min(inner.height as usize));
    let cursor_row = row_for(
        &prompt_rows,
        app.agent.cursor,
        app.agent.prompt.chars().count(),
    );
    let prompt_start = cursor_row
        .saturating_add(1)
        .saturating_sub(prompt_height)
        .min(prompt_rows.len().saturating_sub(prompt_height));
    let transcript_room = (inner.height as usize).saturating_sub(prompt_height);
    let transcript = transcript_lines(app, content);
    let max_scroll = transcript.len().saturating_sub(transcript_room);
    let from_end = if app.agent.reading {
        app.agent.scroll.min(max_scroll)
    } else {
        0
    };
    let transcript_start = max_scroll - from_end;
    for (offset, line) in transcript
        .iter()
        .skip(transcript_start)
        .take(transcript_room)
        .enumerate()
    {
        let style = if line.user {
            theme.sidebar
        } else {
            theme.sidebar_dim
        };
        frame.render_widget(
            Paragraph::new(Line::styled(fit(&line.text, width), style)).style(theme.sidebar),
            Rect::new(inner.x, inner.y + offset as u16, inner.width, 1),
        );
    }
    let marked = theme.sidebar.bg(theme.sidebar_selected);
    let mut cursor_at = None;
    for (offset, row) in prompt_rows
        .iter()
        .enumerate()
        .skip(prompt_start)
        .take(prompt_height)
    {
        let y = inner.y + transcript_room as u16 + (offset - prompt_start) as u16;
        let (spans, column) = prompt_line(
            &app.agent.prompt,
            *row,
            offset == 0,
            app.agent.cursor,
            app.agent.anchor,
            theme.sidebar,
            marked,
        );
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(theme.sidebar),
            Rect::new(inner.x, y, inner.width, 1),
        );
        if offset == cursor_row {
            cursor_at = Some((column, y));
        }
    }
    if focused && !app.agent.reading {
        if let Some((column, y)) = cursor_at {
            let column = (column as u16).min(inner.width.saturating_sub(1));
            super::cursor::place(inner.x + column, y);
        }
    }
}

struct DrawnLine {
    user: bool,
    text: String,
}

fn transcript_lines(app: &App, content: usize) -> Vec<DrawnLine> {
    let mut lines = Vec::new();
    for message in &app.agent.transcript {
        let chars: Vec<char> = message.text.chars().collect();
        for (index, (start, end)) in wrap_rows(&message.text, content).into_iter().enumerate() {
            let body: String = chars
                .get(start..end.min(chars.len()))
                .unwrap_or(&[])
                .iter()
                .collect();
            let mark = if index == 0 && message.user { ">" } else { " " };
            lines.push(DrawnLine {
                user: message.user,
                text: format!(" {mark} {body}"),
            });
        }
    }
    lines
}

fn row_for(rows: &[(usize, usize)], cursor: usize, len: usize) -> usize {
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

fn prompt_line(
    prompt: &str,
    row: (usize, usize),
    first: bool,
    cursor: usize,
    anchor: usize,
    base: Style,
    marked: Style,
) -> (Vec<Span<'static>>, usize) {
    let chars: Vec<char> = prompt.chars().collect();
    let len = chars.len();
    let cursor = cursor.min(len);
    let anchor = anchor.min(len);
    let (sel_from, sel_to) = (cursor.min(anchor), cursor.max(anchor));
    let prefix = if first { " > " } else { "   " };
    let mut spans = vec![Span::styled(prefix, base)];
    let mut index = row.0.min(len);
    let end = row.1.min(len);
    while index < end {
        let selected = index >= sel_from && index < sel_to;
        let mut chunk = String::new();
        while index < end && (index >= sel_from && index < sel_to) == selected {
            chunk.push(chars[index]);
            index += 1;
        }
        spans.push(Span::styled(chunk, if selected { marked } else { base }));
    }
    let at = cursor.clamp(row.0.min(len), end);
    let column = prefix.width() + crate::agent::column_at(prompt, row.0.min(len), at);
    (spans, column)
}

fn status_label(app: &App) -> String {
    if !app.agent.busy {
        return app.agent.status.clone();
    }
    let frame = SPINNER[app.agent.spin as usize % SPINNER.len()];
    format!("{frame} {}", app.agent.status)
}

fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        if out.width() + ch.width().unwrap_or(0) >= width {
            break;
        }
        out.push(ch);
    }
    out
}
