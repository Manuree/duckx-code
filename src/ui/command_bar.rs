use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
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
/// Modeless off: one row for the mode's keys (Settings → Editor → Mode hints).
pub fn mode_hint_rows(app: &App, width: u16, height: u16) -> u16 {
    let searching = app.find_mode.is_some() || app.replace_mode.is_some();
    if (searching || app.mode_hints_shown()) && width >= 20 && height >= 3 {
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
        .status_message()
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

/// The keys of the current mode, one row: a strip in the mode color, then `key word`
/// pairs. A notice (an unbound key) comes first. Pairs that do not fit are left out
/// from the end, but the last one (`? keys` or `Esc normal`) always stays.
pub fn render_modal_hint(frame: &mut Frame, app: &App, area: Rect) {
    let Some(state) = app.modal_input() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = *app.styles();
    let color = app.mode_color().unwrap_or(theme.status_a_bg);
    let width = area.width as usize;
    let mut spans = vec![Span::styled("▌", theme.hint.fg(color))];
    let mut used = 1;
    if let Some(notice) = state.notice.as_deref() {
        let text = format!(" {notice} ");
        used += text.width();
        spans.push(Span::styled(
            text,
            theme.status_message.bg(theme.hint.bg.unwrap_or_default()),
        ));
    }
    let pairs = crate::modal::hints(state, app.keys());
    let pair_width = |hint: &crate::modal::Hint| {
        if hint.marked() {
            1 + hint.word.width() + 1
        } else {
            1 + hint.key.width() + 1 + hint.word.width() + 1
        }
    };
    // The letters to press: the mode color, bold, and underlined, so they read
    // without color too.
    let mark = theme
        .hint_keys
        .fg(color)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let mut shown: Vec<&crate::modal::Hint> = Vec::new();
    if let Some((last, rest)) = pairs.split_last() {
        let mut room = width.saturating_sub(used + pair_width(last));
        for hint in rest {
            let need = pair_width(hint);
            if need > room {
                break;
            }
            room -= need;
            shown.push(hint);
        }
        if pair_width(last) <= width.saturating_sub(used) {
            shown.push(last);
        }
    }
    for hint in shown {
        if hint.marked() {
            spans.push(Span::styled(" ", theme.hint));
            spans.extend(marked_spans(&hint.word, &hint.marks, theme.hint, mark));
            spans.push(Span::styled(" ", theme.hint));
        } else {
            spans.push(Span::styled(" ", theme.hint));
            spans.push(Span::styled(hint.key.clone(), mark));
            spans.push(Span::styled(format!(" {} ", hint.word), theme.hint));
        }
        used += pair_width(hint);
    }
    spans.push(Span::styled(
        " ".repeat(width.saturating_sub(used)),
        theme.hint,
    ));
    let rect = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.hint), rect);
}

/// `word` as spans: the chars at `marks` in `mark`, the rest in `base`.
pub(crate) fn marked_spans(
    word: &str,
    marks: &[usize],
    base: Style,
    mark: Style,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    for (index, c) in word.chars().enumerate() {
        if marks.contains(&index) {
            if !run.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut run), base));
            }
            spans.push(Span::styled(c.to_string(), mark));
        } else {
            run.push(c);
        }
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, base));
    }
    spans
}

/// The `?` key list, a dropdown standing on `floor` (the hint row or status line).
pub fn render_key_list(frame: &mut Frame, app: &App, area: Rect, floor: u16) {
    let Some(state) = app.modal_input() else {
        return;
    };
    let Some(list) = state.key_list.as_ref() else {
        return;
    };
    let entries = crate::modal::key_list(state.style, state.mode, &list.query);
    let rows: Vec<Row> = entries
        .iter()
        .map(|entry| {
            let tag = crate::modal::command_name(entry)
                .map(str::to_string)
                .or_else(|| crate::modal::mirror_name(entry))
                .unwrap_or_default();
            let name = crate::modal::list_hint(entry);
            Row::new(entry.keys)
                .lead(name.word, name.marks)
                .detail(entry.label)
                .tag(tag)
        })
        .collect();
    let mode = state.mode.badge().trim();
    let title = if list.query.is_empty() {
        format!(" Keys · {mode} ")
    } else {
        format!(" Keys · {mode} · {} ", list.query)
    };
    let theme = theme_of(app);
    let color = app.mode_color().unwrap_or(theme.status_a_bg);
    let dropdown = List::new(&title, "↑↓  enter  type to filter  esc ", &rows)
        .marks(
            Style::new()
                .fg(color)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
        .selected(Some(list.selected))
        .empty("No key matches")
        .label_width(4);
    let room = floor.saturating_sub(area.y);
    if room < 3 || area.width < 20 {
        return;
    }
    let height = super::dropdown::frame_height(rows.len().max(1), 14).min(room);
    let width = dropdown.natural_width().min(area.width);
    let rect = Rect::new(area.x, floor - height, width, height);
    super::dropdown::render(frame, rect, &dropdown, &theme);
}

fn theme_of(app: &App) -> crate::theme::UiStyles {
    *app.styles()
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
                .tag(app.command_shortcut(spec))
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
