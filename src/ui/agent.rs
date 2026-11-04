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
        let spans = approve_button_spans(pending.choices(), pending.choice, theme.sidebar, marked);
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(theme.sidebar),
            Rect::new(inner.x, y, inner.width, 1),
        );
        if focused {
            let column = button_cursor_column(pending.choices(), pending.choice).min(inner.width.saturating_sub(1) as usize);
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

    // Hints sit in the rows above the prompt (command-bar style), even in a small panel.
    if let Some(popup) = app.agent.popup.as_ref() {
        let prompt_top = inner.y + transcript_room as u16;
        render_popup(frame, app, popup, inner, prompt_top, &theme);
    }
}


fn render_popup(
    frame: &mut Frame,
    app: &App,
    popup: &AgentPopup,
    area: Rect,
    prompt_top: u16,
    theme: &crate::theme::UiStyles,
) {
    let _ = app;
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

    // Space above the prompt inside the panel. Always keep at least one hint row
    // by borrowing from the prompt strip when the panel is tiny.
    let available_above = prompt_top.saturating_sub(area.y) as usize;
    let mut hint_rows = available_above.max(1).min(rows.len()).min(8);
    if hint_rows == 0 {
        return;
    }
    // Prefer drawing immediately above the prompt; if there is no room above,
    // draw into the top of the inner area (over transcript / overlapping prompt top).
    let y0 = if available_above > 0 {
        prompt_top.saturating_sub(hint_rows as u16)
    } else {
        area.y
    };
    // Clamp so we stay inside the panel and leave the last prompt row alone when possible.
    let max_y = prompt_top.saturating_sub(1).max(area.y);
    let y0 = y0.min(max_y);
    let room = (prompt_top.saturating_sub(y0)).max(1) as usize;
    hint_rows = hint_rows.min(room).min(rows.len()).max(1);

    let selected = match popup {
        AgentPopup::Slash { selected, .. } | AgentPopup::Mention { selected, .. } => *selected,
    };
    let scroll = selected.saturating_sub(hint_rows.saturating_sub(1));
    let width = area.width as usize;

    for (offset, (left, right, is_sel)) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(hint_rows)
    {
        let y = y0 + (offset - scroll) as u16;
        if y >= prompt_top && available_above > 0 {
            break;
        }
        if y < area.y || y >= area.y + area.height {
            continue;
        }
        let base = if *is_sel { theme.hint_active } else { theme.hint };
        let name_style = if *is_sel {
            theme.hint_active_name
        } else {
            theme.hint_dim
        };
        let desc_style = if *is_sel {
            theme.hint_active_dim
        } else {
            theme.hint_keys
        };
        let keys = if *is_sel { " ↑↓ tab enter esc" } else { "" };
        let name = format!(" {left}");
        let mut spans = vec![Span::styled(name.clone(), name_style)];
        let mut used = name.width();
        if !right.is_empty() {
            let gap = "  ";
            let budget = width.saturating_sub(used + gap.width() + keys.width());
            if budget > 0 {
                let shown = fit(right, budget);
                spans.push(Span::styled(gap, base));
                spans.push(Span::styled(shown.clone(), desc_style));
                used += gap.width() + shown.width();
            }
        }
        if !keys.is_empty() && used + keys.width() <= width {
            let pad = " ".repeat(width.saturating_sub(used + keys.width()));
            spans.push(Span::styled(pad, base));
            spans.push(Span::styled(keys, desc_style));
            used = width;
        }
        if used < width {
            spans.push(Span::styled(" ".repeat(width - used), base));
        }
        let rect = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(Line::from(spans)).style(base), rect);
    }
}

fn approve_label(choice: crate::agent::ApprovalChoice) -> &'static str {
    match choice {
        crate::agent::ApprovalChoice::Yes => "[ Yes ]",
        crate::agent::ApprovalChoice::Session => "[ Yes, this session ]",
        crate::agent::ApprovalChoice::No => "[ No ]",
    }
}

fn approve_button_text(choice: crate::agent::ApprovalChoice, selected: bool) -> String {
    let label = approve_label(choice);
    if selected {
        format!(" > {label} < ")
    } else {
        format!("   {label}   ")
    }
}

fn approve_button_spans(
    choices: &[crate::agent::ApprovalChoice],
    selected: crate::agent::ApprovalChoice,
    base: Style,
    marked: Style,
) -> Vec<Span<'static>> {
    choices
        .iter()
        .map(|choice| {
            let on = *choice == selected;
            Span::styled(approve_button_text(*choice, on), if on { marked } else { base })
        })
        .collect()
}

fn button_cursor_column(
    choices: &[crate::agent::ApprovalChoice],
    selected: crate::agent::ApprovalChoice,
) -> usize {
    let mut column = 0;
    for choice in choices {
        let text = approve_button_text(*choice, *choice == selected);
        if *choice == selected {
            return column + text.width() / 2;
        }
        column += text.width();
    }
    column
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
            if app.agent.session_allow.is_empty() {
                return "ready".to_string();
            }
            return format!("ready · ✓ session-approved: {}", session_allow_list(app));
        }
        return short_status(status);
    }
    let frame = SPINNER[app.agent.spin as usize % SPINNER.len()];
    let elapsed = busy_elapsed_label(app);
    if let Some(draft) = app.agent.thinking_draft.as_ref() {
        if !draft.trim().is_empty() {
            let snip = last_line_snip(draft, 36);
            let tok = rough_token_label(draft);
            if snip.is_empty() {
                return format!("{frame} thinking…{elapsed}{tok}");
            }
            return format!("{frame} thinking…{elapsed}{tok} · {snip}");
        }
    }
    if app.agent.status == "thinking" || app.agent.status.starts_with("thinking") {
        return format!("{frame} thinking…{elapsed}");
    }
    let status = short_status(&app.agent.status);
    if elapsed.is_empty() {
        format!("{frame} {status}")
    } else {
        format!("{frame} {status}{elapsed}")
    }
}

/// Session-approved tools in approval order of the common file tools.
fn session_allow_list(app: &App) -> String {
    const ORDER: [&str; 3] = ["create_file", "apply_edit", "save"];
    let mut names: Vec<&str> = ORDER
        .iter()
        .copied()
        .filter(|name| app.agent.session_allow.contains(*name))
        .collect();
    for name in &app.agent.session_allow {
        if !ORDER.contains(&name.as_str()) {
            names.push(name);
        }
    }
    names.join(", ")
}

fn busy_elapsed_label(app: &App) -> String {
    let Some(started) = app.agent.busy_started else {
        return String::new();
    };
    let secs = started.elapsed().as_secs();
    if secs < 1 {
        return String::new();
    }
    if secs < 60 {
        format!(" {secs}s")
    } else {
        format!(" {}m{:02}s", secs / 60, secs % 60)
    }
}

fn rough_token_label(text: &str) -> String {
    let chars = text.chars().count();
    if chars < 80 {
        return String::new();
    }
    // Rough Latin-ish estimate; title-only so precision does not matter.
    let approx = (chars as f64 / 4.0).round() as usize;
    if approx < 100 {
        format!(" ~{approx}tok")
    } else if approx < 1000 {
        format!(" ~{approx}tok")
    } else {
        format!(" ~{:.1}ktok", approx as f64 / 1000.0)
    }
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
