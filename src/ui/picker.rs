use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::icons;
use super::panel;
use crate::app::App;
use crate::picker::{self, FilePicker};
use crate::theme::UiStyles;

/// Centered file list. The query sits on the last row, with the block cursor in it.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(picker) = app.file_picker.as_ref() else {
        return;
    };
    let Some(rect) = modal_rect(area) else {
        return;
    };
    let theme = *app.styles();
    let block = panel(" Files ")
        .border_style(theme.command_border)
        .style(Style::new().bg(theme.editor_bg));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let list_height = inner.height.saturating_sub(1) as usize;
    let mut lines = Vec::with_capacity(inner.height as usize);
    if picker.scanning && picker.rows.is_empty() {
        lines.push(padded("Scanning…", inner.width, theme.dim));
    } else if picker.rows.is_empty() {
        lines.push(padded("No matching files", inner.width, theme.dim));
    } else {
        let end = (picker.scroll + list_height).min(picker.rows.len());
        for row in picker.scroll..end {
            lines.push(file_line(app, picker, row, inner.width as usize, &theme));
        }
    }
    while lines.len() < list_height {
        lines.push(Line::raw(""));
    }
    let cursor = query_line(picker, inner.width as usize, &theme, &mut lines);
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme.editor_bg)),
        inner,
    );
    if let Some(column) = cursor {
        let x = inner
            .x
            .saturating_add(column)
            .min(inner.right().saturating_sub(1));
        frame.set_cursor_position((x, inner.bottom().saturating_sub(1)));
    }
}

fn modal_rect(area: Rect) -> Option<Rect> {
    if area.width < 20 || area.height < 6 {
        return None;
    }
    let height = picker::modal_height(area.height).min(area.height);
    let width = (area.width * 4 / 5).clamp(20, area.width);
    let x = area.x + (area.width - width) / 2;
    let y = area.y + (area.height.saturating_sub(height) / 5).min(3);
    Some(Rect::new(x, y, width, height))
}

fn padded(text: &str, width: u16, style: Style) -> Line<'static> {
    let mut shown = format!(" {text}");
    let columns = width as usize;
    if shown.width() > columns {
        shown = clip_end(&shown, columns);
    }
    Line::from(Span::styled(shown, style))
}

fn file_line(
    app: &App,
    picker: &FilePicker,
    row: usize,
    width: usize,
    theme: &UiStyles,
) -> Line<'static> {
    let selected = row == picker.selected;
    let row_bg = if selected {
        theme.hint_active.bg.unwrap_or(theme.editor_bg)
    } else {
        theme.editor_bg
    };
    let base = if selected {
        theme.hint_active
    } else {
        Style::new().fg(theme.editor_fg)
    };
    let dir_style = if selected {
        theme.hint_active_dim
    } else {
        theme.dim
    }
    .bg(row_bg);
    let name_style = base.add_modifier(Modifier::BOLD).bg(row_bg);
    let match_style = theme.sidebar_match.bg(row_bg);
    let Some(&index) = picker.rows.get(row) else {
        return Line::raw("");
    };
    let Some(file) = app.picker_file(index) else {
        return Line::raw("");
    };
    let (icon, icon_color) = icons::file_icon(Some(std::path::Path::new(&file.relative)));
    let prefix = format!(" {icon} ");
    let prefix_width = prefix.width();
    let budget = width.saturating_sub(prefix_width);
    let (origin, clipped) = clip_start(&file.relative, budget);
    let visible = &file.relative[origin..];
    let hits = picker::highlight(&file.relative, file.name_at, &picker.query);
    let mut spans = vec![Span::styled(prefix, Style::new().fg(icon_color).bg(row_bg))];
    if clipped {
        spans.push(Span::styled("…", dir_style));
    }
    spans.extend(path_spans(
        visible,
        origin,
        file.name_at,
        &hits,
        dir_style,
        name_style,
        match_style,
    ));
    let used: usize = spans.iter().map(|span| span.width()).sum();
    if used < width {
        spans.push(Span::styled(
            " ".repeat(width - used),
            Style::new().bg(row_bg),
        ));
    }
    Line::from(spans)
}

fn path_spans(
    visible: &str,
    origin: usize,
    name_at: u32,
    hits: &[u32],
    dir_style: Style,
    name_style: Style,
    match_style: Style,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_style = None;
    for (offset, ch) in visible.char_indices() {
        let byte = (origin + offset) as u32;
        let hit = hits.binary_search(&byte).is_ok();
        let style = if hit {
            match_style
        } else if byte >= name_at {
            name_style
        } else {
            dir_style
        };
        if current_style != Some(style) {
            if !current.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut current),
                    current_style.unwrap_or(name_style),
                ));
            }
            current_style = Some(style);
        }
        current.push(ch);
    }
    if !current.is_empty() {
        spans.push(Span::styled(current, current_style.unwrap_or(name_style)));
    }
    spans
}

fn query_line(
    picker: &FilePicker,
    width: usize,
    theme: &UiStyles,
    lines: &mut Vec<Line<'static>>,
) -> Option<u16> {
    let count = counter(picker);
    let prompt = " ❯ ";
    let count_width = count.width();
    let gap = usize::from(count_width > 0);
    let budget = width.saturating_sub(prompt.width() + count_width + gap);
    let (start, end) = picker::query_window(&picker.query, picker.cursor, budget);
    let shown = picker.query.get(start..end).unwrap_or("");
    let at = picker.cursor.clamp(start, end);
    let head = picker.query.get(start..at).unwrap_or("");
    let mut text = format!("{prompt}{shown}");
    let cursor = prompt.width() + head.width();
    let pad = width.saturating_sub(text.width() + gap + count_width);
    text.push_str(&" ".repeat(pad + gap));
    text.push_str(&count);
    if text.width() > width {
        text = clip_end(&text, width);
    }
    lines.push(Line::from(vec![Span::styled(
        text,
        Style::new().fg(theme.editor_fg).bg(theme.editor_bg),
    )]));
    Some(cursor as u16)
}

fn counter(picker: &FilePicker) -> String {
    if picker.scanning && picker.rows.is_empty() {
        return "scanning".to_string();
    }
    let shown = if picker.rows.is_empty() {
        0
    } else {
        picker.selected + 1
    };
    let mut text = format!("{shown}/{}", picker.total);
    if picker.truncated {
        text.push('+');
    }
    if picker.refreshing {
        text.push_str(" …");
    }
    text
}

/// Drops characters from the left so `text` fits, leaving room for an ellipsis.
fn clip_start(text: &str, max_cols: usize) -> (usize, bool) {
    if max_cols == 0 || text.width() <= max_cols {
        return (0, false);
    }
    let mut drop_cols = text.width() + 1 - max_cols;
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        start = index + ch.len_utf8();
        drop_cols = drop_cols.saturating_sub(width);
        if drop_cols == 0 {
            break;
        }
    }
    (start, true)
}

fn clip_end(text: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + width > max_cols.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += width;
    }
    out.push('…');
    out
}
