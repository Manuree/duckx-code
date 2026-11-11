use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::{icons, panel};
use crate::explorer::Explorer;
use crate::services::git::GitStatus;
use crate::theme::UiStyles;

const FILTER_HEIGHT: u16 = 3;

/// Rows below the filter box and the root line.
pub fn tree_area(area: Rect) -> Rect {
    let top = (FILTER_HEIGHT + 1).min(area.height);
    Rect::new(area.x, area.y + top, area.width, area.height - top)
}

pub fn render(frame: &mut Frame, explorer: &Explorer, area: Rect, theme: &UiStyles) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Block::new().style(theme.sidebar), area);

    let filter_area = Rect {
        height: FILTER_HEIGHT.min(area.height),
        ..area
    };
    render_filter(frame, explorer, filter_area, theme);

    if area.height > FILTER_HEIGHT {
        let root = Line::from(vec![
            Span::raw(" "),
            Span::styled(
                format!("{} ", icons::FOLDER_OPEN),
                Style::new().fg(theme.sidebar_folder),
            ),
            Span::styled(explorer.root_name.clone(), theme.sidebar_root),
        ]);
        let root_area = Rect::new(area.x, area.y + FILTER_HEIGHT, area.width, 1);
        frame.render_widget(Paragraph::new(root).style(theme.sidebar), root_area);
    }

    render_tree(frame, explorer, tree_area(area), theme);
}

fn render_filter(frame: &mut Frame, explorer: &Explorer, area: Rect, theme: &UiStyles) {
    let border = if explorer.focused {
        theme.sidebar_border_focused
    } else {
        theme.sidebar_border
    };
    let block = panel("").border_style(border).style(theme.sidebar);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let (shown, total) = explorer.counts();
    let count = format!("{shown}/{total} ");
    frame.render_widget(
        Paragraph::new(Span::styled(count, theme.sidebar_dim))
            .alignment(Alignment::Right)
            .style(theme.sidebar),
        inner,
    );
    let prompt = format!(" {} ", icons::SEARCH);
    let query = explorer.file_query.as_deref().unwrap_or(&explorer.filter);
    let cursor = prompt.width() + query.width();
    let input = Line::from(vec![
        Span::styled(prompt, theme.sidebar_prompt),
        Span::raw(query),
    ]);
    frame.render_widget(Paragraph::new(input), inner);

    if explorer.focused && explorer.file_query.is_none() {
        let col = cursor as u16;
        super::cursor::place(inner.x + col.min(inner.width.saturating_sub(1)), inner.y);
    }
}

fn render_tree(frame: &mut Frame, explorer: &Explorer, area: Rect, theme: &UiStyles) {
    if area.height == 0 {
        return;
    }
    if explorer.loading {
        frame.render_widget(
            Paragraph::new(Span::styled("  Loading…", theme.sidebar_dim)).style(theme.sidebar),
            area,
        );
        return;
    }

    let width = area.width as usize;
    let selected = explorer.selected_index();
    let rows = explorer.visible_rows();
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(explorer.scroll)
        .take(area.height as usize)
        .map(|(index, row)| {
            let entry = row.entry;
            let (icon, color) = if entry.is_dir {
                let icon = if row.open {
                    icons::FOLDER_OPEN
                } else {
                    icons::FOLDER_CLOSED
                };
                (icon, theme.sidebar_folder)
            } else {
                icons::file_icon(Some(&entry.path))
            };
            let status = explorer.git.get(&entry.path).copied();
            let hits = row.hits.map(|count| format!("{count} "));
            let hits_width = hits.as_ref().map(|text| text.width()).unwrap_or(0);

            let marked = explorer.is_marked(&entry.path);
            let covered = explorer.is_covered(&entry.path);
            let mark = if marked {
                "●"
            } else if covered {
                "○"
            } else {
                " "
            };
            let mark_style = if marked || covered {
                Style::new().fg(theme.git_added)
            } else {
                theme.sidebar_guide
            };
            let icon = format!("{icon} ");
            let status_width = if status.is_some() { 2 } else { 0 };
            let room = width
                .saturating_sub(1 + row.guide.width() + icon.width() + status_width + hits_width);
            let name = truncate(&entry.name, room);
            let padding = room.saturating_sub(name.width());

            let name_style = match (status, row.matched) {
                (_, true) => theme.sidebar_match,
                (Some(status), false) => Style::new().fg(git_color(theme, status)),
                // Dot entries (shown with Show hidden files) read as secondary.
                (None, false) if crate::services::tree::is_hidden_name(&entry.name) => {
                    theme.sidebar_dim
                }
                (None, false) if entry.is_dir => Style::new().fg(theme.sidebar_folder),
                (None, false) => Style::new(),
            };
            let mut spans = vec![
                Span::styled(mark, mark_style),
                Span::styled(row.guide.clone(), theme.sidebar_guide),
                Span::styled(icon, Style::new().fg(color)),
                Span::styled(name, name_style),
                Span::raw(" ".repeat(padding)),
            ];
            if let Some(hits) = hits {
                spans.push(Span::styled(hits, theme.sidebar_match));
            }
            if let Some(status) = status {
                spans.push(Span::styled(
                    format!("{} ", status.symbol()),
                    Style::new().fg(git_color(theme, status)),
                ));
            }

            let mut line = Line::from(spans);
            if Some(index) == selected {
                let bg = if explorer.focused {
                    theme.sidebar_selected
                } else {
                    theme.sidebar_selected_unfocused
                };
                line = line.style(Style::new().bg(bg));
            } else if covered {
                line = line.style(theme.selection);
            }
            line
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).style(theme.sidebar), area);
}

fn git_color(theme: &UiStyles, status: GitStatus) -> ratatui::style::Color {
    match status {
        GitStatus::Untracked => theme.git_untracked,
        GitStatus::Modified => theme.git_modified,
        GitStatus::Added => theme.git_added,
        GitStatus::Deleted => theme.git_deleted,
        GitStatus::Renamed => theme.git_renamed,
    }
}

/// Cuts `text` to `max` columns, ending with `…` when shortened.
fn truncate(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_marks_cut_names() {
        assert_eq!(truncate("main.rs", 10), "main.rs");
        assert_eq!(truncate("explorer.rs", 6), "explo…");
        assert_eq!(truncate("日本語.md", 4), "日…");
        assert_eq!(truncate("abc", 0), "");
    }
}
