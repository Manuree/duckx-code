use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::command::{CommandBar, CommandId, CommandSpec, is_line_number, suggestions};
use crate::theme::UiStyles;

const KEYS: &str = "↑↓  tab  enter  esc ";
const FIND_KEYS: &str = "↑↓  esc ";
const FIND_FILE_KEYS: &str = "enter  esc ";
const REPLACE_KEYS: &str = "↑ undo  ↓ next  ⇧↓ all  esc ";
const REPLACE_FILE_KEYS: &str = "enter  esc ";
const KEYS_GHOST: &str = "↑↓  →  tab  enter  esc ";
const MAX_VISIBLE: usize = 8;
const NAME_WIDTH: usize = 18;

/// Hint rows stacked above the status line, one command per row.
/// `Up` moves up the list and `Down` moves down it.
pub fn reserved_height(app: &App, width: u16, height: u16) -> u16 {
    let Some(bar) = &app.command_bar else {
        return 0;
    };
    if width < 20 || height < 3 {
        return 0;
    }
    let count = suggestions(
        &bar.input,
        app.recent_for(bar.context),
        bar.context,
        bar.files,
    )
    .len()
    .clamp(1, MAX_VISIBLE) as u16;
    count.min(height.saturating_sub(2))
}

/// One row above the status line while FIND or REPLACE mode is open.
pub fn mode_hint_rows(app: &App, width: u16, height: u16) -> u16 {
    if (app.find_mode.is_some() || app.replace_mode.is_some()) && width >= 20 && height >= 3 {
        1
    } else {
        0
    }
}

pub fn render_mode_hint(frame: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = *app.styles();
    let label = app
        .message
        .as_deref()
        .map(|message| format!(" {message}"))
        .unwrap_or_default();
    let rect = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    frame.render_widget(
        Paragraph::new(padded(
            &label,
            theme.hint,
            "",
            theme.hint,
            if app.replacing_files() {
                REPLACE_FILE_KEYS
            } else if app.replace_mode.is_some() {
                REPLACE_KEYS
            } else if app.finding_files() {
                FIND_FILE_KEYS
            } else {
                FIND_KEYS
            },
            theme.hint_keys,
            theme.hint,
            area.width as usize,
        ))
        .style(theme.hint),
        rect,
    );
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 || app.command_bar.is_none() {
        return;
    }
    let theme = *app.styles();
    let Some(bar) = &app.command_bar else {
        return;
    };
    let matches = suggestions(
        &bar.input,
        app.recent_for(bar.context),
        bar.context,
        bar.files,
    );
    if matches.is_empty() {
        let rect = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
        frame.render_widget(
            Paragraph::new(empty_row(area.width as usize, &theme)).style(theme.hint),
            rect,
        );
        return;
    }
    let rows = area.height as usize;
    let selected = bar.selected.min(matches.len() - 1);
    let ghost = app.argument_ghost().is_some();
    for (offset, index) in row_window(matches.len(), selected, rows).enumerate() {
        let rect = Rect::new(area.x, area.y + offset as u16, area.width, 1);
        let (line, style) = command_row(
            matches[index],
            bar,
            app.recent_for(bar.context),
            index == selected,
            ghost,
            area.width as usize,
            &theme,
        );
        frame.render_widget(Paragraph::new(line).style(style), rect);
    }
}

fn empty_row(width: usize, theme: &UiStyles) -> Line<'static> {
    let message = " No matching command";
    let keys = if width > message.width() + KEYS.width() {
        KEYS
    } else {
        ""
    };
    padded(
        message,
        theme.hint_dim,
        "",
        theme.hint,
        keys,
        theme.hint_keys,
        theme.hint,
        width,
    )
}

fn command_row(
    spec: &CommandSpec,
    bar: &CommandBar,
    recent: Option<CommandId>,
    selected: bool,
    ghost: bool,
    width: usize,
    theme: &UiStyles,
) -> (Line<'static>, Style) {
    let base = if selected {
        theme.hint_active
    } else {
        theme.hint
    };
    let name_style = if selected {
        theme.hint_active_name
    } else {
        theme.hint_dim
    };
    let extra_style = if selected {
        theme.hint_active_dim
    } else {
        theme.hint_keys
    };
    let marker = if Some(spec.id) == recent { "★ " } else { "" };
    let head = pad_width(&format!(" {marker}{}", command_name(spec, bar)), NAME_WIDTH);
    let mut right = if spec.shortcut.is_empty() {
        String::new()
    } else {
        format!("{} ", spec.shortcut)
    };
    if selected {
        let keys = if ghost { KEYS_GHOST } else { KEYS };
        let with_keys = format!("{right}{keys}");
        if head.width() + with_keys.width() + 2 <= width {
            right = with_keys;
        }
    }
    if head.width() + right.width() > width {
        right.clear();
    }
    let room = width.saturating_sub(head.width() + right.width());
    let description = if room < 2 {
        String::new()
    } else {
        truncate_width(&format!(" {}", spec.description), room)
    };
    (
        padded(
            &head,
            name_style,
            &description,
            base,
            &right,
            extra_style,
            base,
            width,
        ),
        base,
    )
}

fn padded(
    head: &str,
    head_style: Style,
    middle: &str,
    middle_style: Style,
    tail: &str,
    tail_style: Style,
    fill: Style,
    width: usize,
) -> Line<'static> {
    let gap = width.saturating_sub(head.width() + middle.width() + tail.width());
    Line::from(vec![
        Span::styled(head.to_string(), head_style),
        Span::styled(middle.to_string(), middle_style),
        Span::styled(" ".repeat(gap), fill),
        Span::styled(tail.to_string(), tail_style),
    ])
}

fn command_name(spec: &CommandSpec, bar: &CommandBar) -> String {
    match spec.arg {
        Some(_) if is_line_number(bar.query()) => format!("{} {}", spec.name, bar.query()),
        Some(arg) => format!("{} {arg}", spec.name),
        None => spec.name.to_string(),
    }
}

fn pad_width(text: &str, width: usize) -> String {
    let pad = width.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(pad))
}

/// Rows of the match list that fit, keeping `selected` on screen.
/// Index 0 is the top row, so `Down` moves the highlight downward.
fn row_window(len: usize, selected: usize, rows: usize) -> std::ops::Range<usize> {
    if len == 0 || rows == 0 {
        return 0..0;
    }
    let rows = rows.min(len);
    let selected = selected.min(len - 1);
    let mut start = selected.saturating_sub(rows / 2);
    if start + rows > len {
        start = len - rows;
    }
    start..start + rows
}

fn truncate_width(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    for grapheme in text.graphemes(true) {
        if out.width() + grapheme.width() + 1 > width {
            break;
        }
        out.push_str(grapheme);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_window_keeps_the_selection_and_grows_downward() {
        assert_eq!(row_window(2, 0, 8), 0..2);
        assert_eq!(row_window(16, 0, 8), 0..8);
        assert_eq!(row_window(16, 15, 8), 8..16);
        assert_eq!(row_window(0, 0, 4), 0..0);
    }

    #[test]
    fn truncate_width_marks_the_cut() {
        assert_eq!(truncate_width("save", 4), "save");
        assert_eq!(truncate_width("explorer", 4), "exp…");
        assert_eq!(truncate_width("explorer", 1), "…");
        assert_eq!(truncate_width("explorer", 0), "");
    }
}
