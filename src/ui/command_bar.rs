use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::command::{CommandBar, CommandSpec, is_line_number, suggestions};

use super::dropdown::{List, Row};

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
    if app.command_bar.is_none() || width < 20 || height < 3 {
        return 0;
    }
    let count = if let Some(rows) = app.workspace_choices() {
        rows.len()
    } else {
        let bar = app.command_bar.as_ref().expect("bar is open");
        suggestions(
            &bar.input,
            app.recent_for(bar.context),
            bar.context,
            bar.files,
        )
        .len()
    }
    .clamp(1, MAX_VISIBLE) as u16;
    // Two more rows for the list's border.
    (count + 2).min(height.saturating_sub(2))
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
    let ghost = app.argument_ghost().is_some();
    let keys = if ghost { KEYS_GHOST } else { KEYS };
    let selected = app
        .command_bar
        .as_ref()
        .map(|bar| bar.selected)
        .unwrap_or(0);
    if let Some(paths) = app.workspace_choices() {
        let rows: Vec<Row> = paths.iter().map(|path| Row::new(path.clone())).collect();
        let list = List::new(" Folders ", keys, &rows)
            .selected(Some(selected))
            .empty("No saved folder");
        super::dropdown::render(frame, area, &list, &theme);
        return;
    }
    let Some(bar) = &app.command_bar else {
        return;
    };
    let recent = app.recent_for(bar.context);
    let matches = suggestions(&bar.input, recent, bar.context, bar.files);
    let rows: Vec<Row> = matches
        .iter()
        .map(|spec| {
            let marker = if Some(spec.id) == recent { "★ " } else { "" };
            Row::new(format!("{marker}{}", command_name(spec, bar)))
                .detail(spec.description)
                .tag(spec.shortcut)
        })
        .collect();
    let list = List::new(" Commands ", keys, &rows)
        .selected(Some(selected))
        .empty("No matching command")
        .label_width(NAME_WIDTH);
    super::dropdown::render(frame, area, &list, &theme);
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
