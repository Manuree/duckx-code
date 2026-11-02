use std::path::MAIN_SEPARATOR;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use super::icons::{self, BRANCH, SEPARATOR_LEFT, SEPARATOR_RIGHT};
use crate::app::{App, LspLabel};
use crate::buffer::{Buffer, IndentStyle, LineEnding};
use crate::lsp::{ServerStatus, Severity};

/// `[NORMAL|FILE|COMMAND|FIND|OPEN|SETTINGS|ABOUT][branch][icon file | ❯ input]  ...  [selection][progress line:col][indent ending]`.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let right = Line::from(right_spans(app));
    let right_width = (right.width() as u16).min(area.width);
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)]).areas(area);

    let theme = *app.styles();
    let (left, cursor) = left_line(app);
    frame.render_widget(
        Paragraph::new(Line::from(left)).style(theme.status_c),
        left_area,
    );
    frame.render_widget(Paragraph::new(right).style(theme.status_c), right_area);

    if (app.command_bar.is_some() || app.find_mode.is_some() || app.replace_mode.is_some())
        && left_area.width > 0
        && left_area.height > 0
    {
        let x = left_area
            .x
            .saturating_add(cursor as u16)
            .min(left_area.right().saturating_sub(1));
        super::cursor::place(x, left_area.y);
    }
}

fn separator(glyph: &'static str, fg: Color, bg: Color) -> Span<'static> {
    Span::styled(glyph, Style::new().fg(fg).bg(bg))
}

/// `(spans, cursor column relative to the left edge)`. The cursor is only used while the command bar is open.
fn left_line(app: &App) -> (Vec<Span<'static>>, usize) {
    let theme = *app.styles();
    if app.command_bar.is_some() {
        command_line(app)
    } else if app.replace_mode.is_some() {
        replace_line(app)
    } else if app.find_mode.is_some() {
        find_line(app)
    } else if app.about {
        (
            file_line(app, " ABOUT ", theme.status_find, theme.status_find_bg),
            0,
        )
    } else if app.focus() == crate::action::Focus::Agent {
        (
            file_line(
                app,
                " AGENT ",
                theme.status_command,
                theme.status_command_bg,
            ),
            0,
        )
    } else if app.settings.is_some() {
        (
            file_line(
                app,
                " SETTINGS ",
                theme.status_settings,
                theme.status_settings_bg,
            ),
            0,
        )
    } else if app.file_picker.is_some() {
        (
            file_line(app, " OPEN ", theme.status_command, theme.status_command_bg),
            0,
        )
    } else if app.explorer.focused && app.filtering() {
        (
            file_line(app, " FILTER ", theme.status_filter, theme.status_filter_bg),
            0,
        )
    } else if app.explorer.selecting {
        (
            file_line(app, " FILE ", theme.status_select, theme.status_select_bg),
            0,
        )
    } else {
        (
            file_line(app, " NORMAL ", theme.status_a, theme.status_a_bg),
            0,
        )
    }
}

fn command_line(app: &App) -> (Vec<Span<'static>>, usize) {
    let Some(input) = app.command_bar.as_ref().map(|bar| bar.input.clone()) else {
        return (Vec::new(), 0);
    };
    let theme = *app.styles();
    let prompt = " ❯ ";
    let cursor = " COMMAND ".width() + SEPARATOR_LEFT.width() + prompt.width() + input.width();
    let mut spans = vec![
        Span::styled(" COMMAND ", theme.status_command),
        separator(SEPARATOR_LEFT, theme.status_command_bg, theme.status_c_bg),
        Span::styled(prompt, theme.command_prompt.bg(theme.status_c_bg)),
        Span::styled(input, theme.status_c_bold),
    ];
    if let Some(ghost) = app.argument_ghost() {
        spans.push(Span::styled(ghost, theme.status_c_dim));
    }
    if let Some(message) = &app.message {
        spans.push(Span::styled(format!("  {message}"), theme.status_message));
    }
    (spans, cursor)
}

fn replace_line(app: &App) -> (Vec<Span<'static>>, usize) {
    let theme = *app.styles();
    let (head, replacement) = app.replace_parts().unwrap_or_default();
    let prompt = " ❯ ";
    let field = format!("{head} → {replacement}");
    let cursor = " REPLACE ".width() + SEPARATOR_LEFT.width() + prompt.width() + field.width();
    (
        vec![
            Span::styled(" REPLACE ", theme.status_select),
            separator(SEPARATOR_LEFT, theme.status_select_bg, theme.status_c_bg),
            Span::styled(prompt, theme.command_prompt.bg(theme.status_c_bg)),
            Span::styled(head, theme.status_c_dim.bg(theme.status_c_bg)),
            Span::styled(" → ", theme.status_c.bg(theme.status_c_bg)),
            Span::styled(replacement, theme.status_c_bold),
        ],
        cursor,
    )
}

fn find_line(app: &App) -> (Vec<Span<'static>>, usize) {
    let theme = *app.styles();
    let input = app
        .find_mode
        .as_ref()
        .map(|mode| mode.query.clone())
        .unwrap_or_default();
    let prompt = " ❯ ";
    let cursor = " FIND ".width() + SEPARATOR_LEFT.width() + prompt.width() + input.width();
    (
        vec![
            Span::styled(" FIND ", theme.status_find),
            separator(SEPARATOR_LEFT, theme.status_find_bg, theme.status_c_bg),
            Span::styled(prompt, theme.command_prompt.bg(theme.status_c_bg)),
            Span::styled(input, theme.status_c_bold),
        ],
        cursor,
    )
}

fn file_line(app: &App, label: &'static str, badge: Style, badge_bg: Color) -> Vec<Span<'static>> {
    let theme = *app.styles();
    let mut spans = vec![Span::styled(label, badge)];
    if let Some(branch) = app.branch.as_deref() {
        spans.push(separator(SEPARATOR_LEFT, badge_bg, theme.status_b_bg));
        spans.push(Span::styled(format!(" {BRANCH} {branch} "), theme.status_b));
        spans.push(separator(
            SEPARATOR_LEFT,
            theme.status_b_bg,
            theme.status_c_bg,
        ));
    } else {
        spans.push(separator(SEPARATOR_LEFT, badge_bg, theme.status_c_bg));
    }

    let (icon, icon_color) = icons::file_icon(app.path());
    spans.push(Span::styled(
        format!(" {icon} "),
        theme.status_c.fg(icon_color),
    ));
    match app.path() {
        Some(path) => {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                spans.push(Span::styled(
                    format!("{}{MAIN_SEPARATOR}", parent.display()),
                    theme.status_c_dim,
                ));
            }
            let name = path.file_name().unwrap_or(path.as_os_str());
            spans.push(Span::styled(
                name.to_string_lossy().into_owned(),
                theme.status_c_bold,
            ));
        }
        None => spans.push(Span::styled("[No Name]", theme.status_c_bold)),
    }
    if app.show_diff() {
        spans.push(Span::styled("  diff", theme.status_c_dim));
    }
    if app.is_dirty() {
        spans.push(Span::styled(" [+]", theme.status_message));
    }

    if let Some(message) = &app.message {
        spans.push(Span::styled(format!("  {message}"), theme.status_message));
    }
    spans
}

/// The diagnostic on the cursor line, on its own row above the status bar.
/// The message sits on the left and the count of diagnostics in the file sits on the right.
pub fn render_diagnostic(frame: &mut Frame, app: &App, area: Rect) {
    let Some(diagnostic) = app.cursor_diagnostic() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let message = diagnostic
        .message
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let count: usize = app.diagnostic_counts().iter().sum();
    let badge = format!(" {count} ");
    let badge_width = (badge.width() as u16).min(area.width);
    let bg = app.styles().diagnostic(diagnostic.severity);
    let fg = Color::Black;
    let text_width = area.width.saturating_sub(badge_width) as usize;
    let text = fit_width(&format!(" {message}"), text_width);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, Style::new().fg(fg).bg(bg))))
            .style(Style::new().bg(bg)),
        area,
    );
    let badge_area = Rect {
        x: area.x + area.width - badge_width,
        width: badge_width,
        ..area
    };
    frame.render_widget(
        Paragraph::new(badge).style(Style::new().fg(fg).bg(darken(bg))),
        badge_area,
    );
}

fn darken(color: Color) -> Color {
    match color {
        Color::Rgb(red, green, blue) => Color::Rgb(
            u16::from(red).saturating_mul(3).saturating_div(5) as u8,
            u16::from(green).saturating_mul(3).saturating_div(5) as u8,
            u16::from(blue).saturating_mul(3).saturating_div(5) as u8,
        ),
        other => other,
    }
}

fn fit_width(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width + 1 > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    if width > 0 {
        out.push('…');
    }
    out
}

/// `E 2  W 1` for the active file, then the language server name and what it is doing.
fn lsp_spans(app: &App) -> Vec<Span<'static>> {
    let theme = *app.styles();
    let mut spans = Vec::new();
    let counts = app.diagnostic_counts();
    for (severity, count) in [
        Severity::Error,
        Severity::Warning,
        Severity::Info,
        Severity::Hint,
    ]
    .into_iter()
    .zip(counts)
    {
        if count > 0 {
            spans.push(Span::styled(
                format!("{} {count} ", severity.letter()),
                theme.status_c.fg(theme.diagnostic(severity)),
            ));
        }
    }
    if let Some(LspLabel { name, status }) = app.lsp_label() {
        let text = match status {
            ServerStatus::Starting => format!("{name} starting "),
            ServerStatus::Ready => format!("{name} "),
            ServerStatus::Busy(work) => {
                let work: String = work.chars().take(MAX_WORK_CHARS).collect();
                format!("{name}: {work} ")
            }
            ServerStatus::Failed(_) => format!("{name} stopped "),
        };
        spans.push(Span::styled(text, theme.status_c_dim));
    }
    spans
}

/// Progress text from a server is cut here so the cursor position stays visible.
const MAX_WORK_CHARS: usize = 28;

fn right_spans(app: &App) -> Vec<Span<'static>> {
    let theme = *app.styles();
    let buffer = app.buffer();
    let mut spans = lsp_spans(app);

    let selection = buffer.selection();
    if !selection.is_empty() {
        spans.push(Span::styled(
            format!("{} selected ", selection.to() - selection.from()),
            theme.status_c_dim,
        ));
    }

    let (line, col) = buffer.cursor_line_col();
    spans.push(separator(
        SEPARATOR_RIGHT,
        theme.status_b_bg,
        theme.status_c_bg,
    ));
    spans.push(Span::styled(
        format!(
            " {} {:>3}:{:<2} ",
            progress(line, buffer.len_lines()),
            line + 1,
            col + 1
        ),
        theme.status_b,
    ));

    spans.push(separator(
        SEPARATOR_RIGHT,
        theme.status_a_bg,
        theme.status_b_bg,
    ));
    spans.push(Span::styled(
        format!(" {} ", style_label(buffer, app.encoding_label())),
        theme.status_a,
    ));
    spans
}

/// Indent the next Tab inserts, the line ending new breaks use, then the file encoding.
/// `4 LF UTF-8` is four spaces. `Tab 4 CRLF UTF-16 LE` inserts a tab drawn four columns wide.
fn style_label(buffer: &Buffer, encoding: &str) -> String {
    let indent = match buffer.indent_style() {
        IndentStyle::Spaces(width) => width.to_string(),
        IndentStyle::Tabs => format!("Tab {}", buffer.tab_width()),
    };
    let ending = match buffer.line_ending() {
        LineEnding::Lf => "LF",
        LineEnding::Crlf => "CRLF",
    };
    format!("{indent} {ending} {encoding}")
}

/// Same rule as lualine's `progress`: `Top` on the first line, `Bot` on the last, else a percentage.
fn progress(line: usize, total_lines: usize) -> String {
    let current = line + 1;
    if current == 1 {
        "Top".to_string()
    } else if current >= total_lines {
        "Bot".to_string()
    } else {
        format!("{}%", current * 100 / total_lines)
    }
}

#[cfg(test)]
mod tests {
    use super::{progress, style_label};
    use crate::buffer::Buffer;

    #[test]
    fn progress_matches_lualine() {
        assert_eq!(progress(0, 1), "Top");
        assert_eq!(progress(0, 100), "Top");
        assert_eq!(progress(7, 100), "8%");
        assert_eq!(progress(99, 100), "Bot");
    }

    #[test]
    fn style_label_names_the_indent_and_the_ending_the_buffer_will_write() {
        let spaces = Buffer::from_text("a\nb");
        assert_eq!(style_label(&spaces, "UTF-8"), "4 LF UTF-8");

        let mut crlf = Buffer::from_text("a\r\nb");
        assert_eq!(style_label(&crlf, "UTF-8"), "4 CRLF UTF-8");

        crlf.apply_editor_style(false, 8, None);
        assert_eq!(style_label(&crlf, "UTF-16 LE"), "Tab 8 CRLF UTF-16 LE");
    }
}
