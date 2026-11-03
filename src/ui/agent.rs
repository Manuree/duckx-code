use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::panel;
use crate::agent::{wrap_rows, AgentPopup, filter_commands};
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
    let title = format!(" {} ", fit(&status_label(app), 56));
    let block = panel(&title).border_style(border).style(theme.sidebar);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let content = width.saturating_sub(3).max(1);
    let pending = app.agent.pending.as_ref();
    let prompt_height = if pending.is_some() {
        1usize
    } else {
        let prompt_rows = wrap_rows(&app.agent.prompt, content);
        let prompt_limit = (inner.height as usize).saturating_sub(1).max(1);
        // Grow with content, cap at 8 rows, scroll within that window.
        prompt_rows
            .len()
            .clamp(1, 8.min(prompt_limit).min(inner.height as usize))
    };
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

    if let Some(pending) = pending {
        let y = inner.y + transcript_room as u16;
        let marked = theme
            .sidebar
            .bg(theme.sidebar_selected)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED);
        let spans = approve_button_spans(pending.allow_selected, theme.sidebar, marked);
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(theme.sidebar),
            Rect::new(inner.x, y, inner.width, 1),
        );
        if focused {
            let column = button_cursor_column(pending.allow_selected).min(inner.width.saturating_sub(1) as usize);
            super::cursor::place(inner.x + column as u16, y);
        }
        return;
    }

    let prompt_rows = wrap_rows(&app.agent.prompt, content);
    let cursor_row = row_for(
        &prompt_rows,
        app.agent.cursor,
        app.agent.prompt.chars().count(),
    );
    let prompt_start = cursor_row
        .saturating_add(1)
        .saturating_sub(prompt_height)
        .min(prompt_rows.len().saturating_sub(prompt_height));
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

    // Popup sits above the prompt; approval buttons skip it (early return above).
    if let Some(popup) = app.agent.popup.as_ref() {
        let prompt_top = inner.y + transcript_room as u16;
        render_popup(frame, app, popup, inner, prompt_top, theme.sidebar, theme.sidebar.bg(theme.sidebar_selected));
    }
}


fn render_popup(
    frame: &mut Frame,
    _app: &App,
    popup: &AgentPopup,
    area: Rect,
    prompt_top: u16,
    base: Style,
    marked: Style,
) {
    let rows: Vec<(String, String, bool)> = match popup {
        AgentPopup::Slash { filter, selected } => {
            let hits = filter_commands(filter);
            if hits.is_empty() {
                vec![("(no matching commands)".into(), String::new(), false)]
            } else {
                hits.iter()
                    .enumerate()
                    .map(|(i, cmd)| {
                        (
                            format!("/{}", cmd.name),
                            cmd.summary.to_string(),
                            i == *selected,
                        )
                    })
                    .collect()
            }
        }
        AgentPopup::Mention {
            paths, selected, query, ..
        } => {
            if paths.is_empty() {
                let msg = if query.is_empty() {
                    "(type to filter files)".to_string()
                } else {
                    "(no matching files)".to_string()
                };
                vec![(msg, String::new(), false)]
            } else {
                paths
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.clone(), String::new(), i == *selected))
                    .collect()
            }
        }
    };

    let title = match popup {
        AgentPopup::Slash { .. } => " commands ",
        AgentPopup::Mention { .. } => " files ",
    };

    let max_rows = rows.len().clamp(1, 8);
    let list_h = max_rows as u16;
    let box_h = list_h.saturating_add(2); // borders
    let box_w = area.width.min(72).max(20.min(area.width));
    if box_w < 12 || area.height < 4 {
        return;
    }
    // Prefer sitting just above the prompt; clamp into the panel.
    let y = prompt_top
        .saturating_sub(box_h)
        .max(area.y);
    // If that would cover the prompt almost entirely in a tiny panel, still draw.
    let x = area.x;
    let rect = Rect::new(x, y, box_w, box_h.min(area.height.saturating_sub(prompt_top.saturating_sub(area.y).max(1))));
    if rect.height < 3 || rect.width < 12 {
        return;
    }

    let block = super::panel(title)
        .border_style(base)
        .style(base);
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let scroll = match popup {
        AgentPopup::Slash { selected, .. } | AgentPopup::Mention { selected, .. } => {
            let visible = inner.height as usize;
            selected.saturating_sub(visible.saturating_sub(1))
        }
    };

    for (offset, (left, right, selected)) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(inner.height as usize)
    {
        let style = if *selected { marked } else { base };
        let gap = 2usize;
        let left_w = left.width();
        let avail = inner.width as usize;
        let line = if right.is_empty() {
            format!(" {}{}", left, " ".repeat(avail.saturating_sub(left_w.saturating_add(1))))
        } else {
            let right_budget = avail.saturating_sub(left_w.saturating_add(gap).saturating_add(1));
            let right_shown = fit(right, right_budget);
            let pad = avail
                .saturating_sub(1 + left_w + gap + right_shown.width());
            format!(
                " {}{}{}{}",
                left,
                " ".repeat(gap + pad),
                right_shown,
                ""
            )
        };
        let shown = fit(&line, avail);
        // Pad to full width so selection highlight fills the row.
        let mut padded = shown;
        while padded.width() < avail {
            padded.push(' ');
        }
        let padded = fit(&padded, avail);
        frame.render_widget(
            Paragraph::new(Line::styled(padded, style)).style(base),
            Rect::new(inner.x, inner.y + (offset - scroll) as u16, inner.width, 1),
        );
    }
}

fn approve_button_spans(allow_selected: bool, base: Style, marked: Style) -> Vec<Span<'static>> {
    let yes = if allow_selected {
        Span::styled(" > [ Yes ] < ", marked)
    } else {
        Span::styled("   [ Yes ]   ", base)
    };
    let no = if !allow_selected {
        Span::styled(" > [ No ] < ", marked)
    } else {
        Span::styled("   [ No ]   ", base)
    };
    vec![yes, no]
}

fn button_cursor_column(allow_selected: bool) -> usize {
    if allow_selected {
        " > [ Yes ] < ".width() / 2
    } else {
        " > [ Yes ] < ".width() + " > [ No ] < ".width() / 2
    }
}

struct DrawnLine {
    user: bool,
    text: String,
}

fn transcript_lines(app: &App, content: usize) -> Vec<DrawnLine> {
    let mut lines = Vec::new();
    if let Some(pending) = &app.agent.pending {
        for (index, row) in pending.summary.lines().enumerate() {
            let mark = if index == 0 { "!" } else { " " };
            lines.push(DrawnLine {
                user: false,
                text: format!(" {mark} {row}"),
            });
        }
    }
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
    if let Some(draft) = app.agent.stream_draft.as_ref() {
        let chars: Vec<char> = draft.chars().collect();
        for (index, (start, end)) in wrap_rows(draft, content).into_iter().enumerate() {
            let body: String = chars
                .get(start..end.min(chars.len()))
                .unwrap_or(&[])
                .iter()
                .collect();
            let mark = if index == 0 { " " } else { " " };
            lines.push(DrawnLine {
                user: false,
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
    if app.agent.pending.is_some() {
        return short_status(&app.agent.status);
    }
    if !app.agent.busy {
        let status = app.agent.status.as_str();
        if status.is_empty() || status == "ready" {
            return "ready".to_string();
        }
        return short_status(status);
    }
    let frame = SPINNER[app.agent.spin as usize % SPINNER.len()];
    if let Some(draft) = app.agent.thinking_draft.as_ref() {
        if !draft.trim().is_empty() {
            let snip = last_line_snip(draft, 40);
            if snip.is_empty() {
                return format!("{frame} thinking…");
            }
            return format!("{frame} thinking… · {snip}");
        }
    }
    if app.agent.status == "thinking" || app.agent.status.starts_with("thinking") {
        return format!("{frame} thinking…");
    }
    format!("{frame} {}", short_status(&app.agent.status))
}

/// Drop legacy "approve X · ←/→ Enter" chrome down to `approve X`.
fn short_status(status: &str) -> String {
    let trimmed = status.split(" · ").next().unwrap_or(status).trim();
    trimmed.to_string()
}

fn last_line_snip(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    fit(line, max)
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
