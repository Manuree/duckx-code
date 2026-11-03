use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::panel;
use crate::app::App;
use crate::settings::{Page, SettingId, sections};
use crate::theme::{SyntaxRole, SyntaxTheme, UiStyles};

const NAV_WIDTH: usize = 20;

/// Section list on the left, that section's rows on the right.
/// `Enter` or `Right` moves into the rows. `Esc`, `Left`, or Backspace steps back.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(page) = app.settings.as_ref() else {
        return;
    };
    if area.width < 8 || area.height < 4 {
        return;
    }

    let theme = *app.styles();
    let block = panel(" Settings ")
        .border_style(theme.sidebar_border_focused)
        .style(theme.sidebar);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    frame.render_widget(Block::new().style(theme.sidebar), inner);

    let canvas = theme.sidebar_bg;
    let width = inner.width as usize;
    let footer_row = inner.height.saturating_sub(1);
    let row_id = page
        .inside
        .then(|| page.section().items.get(page.row).map(|item| item.id))
        .flatten();
    let binding = matches!(row_id, Some(SettingId::Binding(_)));
    let text_row = matches!(row_id, Some(SettingId::AiServer | SettingId::AiModel | SettingId::AiApiKey));
    let footer = if page.text.is_some() {
        " type    enter save    esc cancel"
    } else if page.capture {
        " press a key    esc cancel"
    } else if page.menu.is_some() {
        " ↑↓ move    enter select    esc close"
    } else if !page.inside {
        " ↑↓ move    enter → open    del reset    esc back ← close"
    } else if binding {
        " ↑↓ move    enter bind    del reset    esc back ← back"
    } else if text_row {
        " ↑↓ move    enter edit    del reset    esc back ← back"
    } else if row_id.is_some() {
        " ↑↓ move    enter → list    del reset    esc back ← back"
    } else {
        " ↑↓ move    del reset    esc back ← back"
    };
    render_columns(frame, inner, app, page, canvas, width, footer_row);
    put(
        frame,
        inner,
        0,
        footer_row,
        footer,
        theme.sidebar_dim.bg(canvas),
        width,
    );
}

fn render_columns(
    frame: &mut Frame,
    inner: Rect,
    app: &App,
    page: &Page,
    canvas: Color,
    width: usize,
    footer_row: u16,
) {
    if width == 0 || footer_row == 0 {
        return;
    }
    let show_nav = width >= NAV_WIDTH + 24;
    if !show_nav {
        let slots = footer_row as usize;
        if page.inside {
            let anchor = render_items(
                frame, inner, app, page, canvas, 0, width, 0, slots, footer_row,
            );
            paint_menu(frame, inner, app, page, 0, width, anchor, footer_row);
        } else {
            render_sections(
                frame, inner, app, page, canvas, 0, width, 0, slots, footer_row,
            );
        }
        return;
    }

    render_sections(
        frame,
        inner,
        app,
        page,
        canvas,
        0,
        NAV_WIDTH,
        0,
        footer_row as usize,
        footer_row,
    );
    let theme = *app.styles();
    for y in 0..footer_row {
        put(
            frame,
            inner,
            NAV_WIDTH,
            y,
            "│",
            theme.sidebar_guide.bg(canvas),
            1,
        );
    }

    let left = NAV_WIDTH + 1;
    let body_width = width.saturating_sub(left);
    let y = 0u16;
    if y < footer_row {
        let anchor = render_items(
            frame,
            inner,
            app,
            page,
            canvas,
            left,
            body_width,
            y,
            footer_row.saturating_sub(y) as usize,
            footer_row,
        );
        paint_menu(
            frame, inner, app, page, left, body_width, anchor, footer_row,
        );
    }
}

fn render_sections(
    frame: &mut Frame,
    inner: Rect,
    app: &App,
    page: &Page,
    canvas: Color,
    x: usize,
    width: usize,
    y: u16,
    slots: usize,
    footer_row: u16,
) {
    let start = window_start(page.section, sections().len(), slots.max(1));
    let theme = *app.styles();
    for (offset, section) in sections().iter().enumerate().skip(start) {
        let row = y + (offset - start) as u16;
        if row >= footer_row {
            break;
        }
        paint_row(
            frame,
            inner,
            x,
            row,
            &format!(" {}", section.name),
            None,
            "›",
            offset == page.section,
            canvas,
            width,
            1,
            footer_row,
            &theme,
            None,
        );
    }
}

fn render_items(
    frame: &mut Frame,
    inner: Rect,
    app: &App,
    page: &Page,
    canvas: Color,
    x: usize,
    width: usize,
    y: u16,
    slots: usize,
    footer_row: u16,
) -> Option<u16> {
    let section = page.section();
    let item_height = if slots >= section.items.len().saturating_mul(2) {
        2
    } else {
        1
    };
    let visible = (slots / item_height).max(1);
    let start = window_start(page.row, section.items.len(), visible);
    let theme = *app.styles();
    let mut anchor = None;
    for (offset, item) in section.items.iter().enumerate().skip(start) {
        let row = y + ((offset - start) * item_height) as u16;
        if row >= footer_row {
            break;
        }
        let open = page.menu.is_some() && page.inside && offset == page.row;
        let capturing = page.capture && page.inside && offset == page.row;
        let drafting = page.text.is_some() && page.inside && offset == page.row;
        let value = choice_label(app, item.id, open, capturing, drafting);
        let syntax = (item.id == SettingId::SyntaxTheme).then(|| app.appearance.syntax.clone());
        let label = format!(" {}", item.label);
        let value = if drafting { format!("{value} ") } else { value };
        paint_row(
            frame,
            inner,
            x,
            row,
            &label,
            (item_height == 2).then_some(item.detail),
            &value,
            page.inside && offset == page.row,
            canvas,
            width,
            item_height,
            footer_row,
            &theme,
            syntax.as_ref(),
        );
        if drafting {
            place_draft_cursor(inner, x, row, &label, &value, width);
        }
        if open {
            anchor = Some(row);
        }
    }
    anchor
}

fn choice_label(app: &App, id: SettingId, open: bool, capturing: bool, drafting: bool) -> String {
    if capturing {
        return "press a key".to_string();
    }
    if drafting {
        return app
            .settings
            .as_ref()
            .and_then(|page| page.text.as_ref())
            .map(|draft| draft.value.clone())
            .unwrap_or_default();
    }
    let mark = if open { "▴" } else { "▾" };
    match id {
        SettingId::TabWidth | SettingId::IndentWithSpaces | SettingId::LineEnding => {
            format!("{} {mark}", app.editor_prefs.choice_label(id))
        }
        SettingId::TrimTrailingSpace | SettingId::FinalNewline => {
            format!("{} {mark}", app.file_prefs.choice_label(id))
        }
        SettingId::ColorTheme => format!("{} {mark}", app.appearance.color.name),
        SettingId::SyntaxTheme => format!("{} {mark}", app.appearance.syntax.name),
        SettingId::Binding(binding) => app.keys().display_of(binding),
        id @ (SettingId::LanguageServer
        | SettingId::CheckOnSave
        | SettingId::FormatOnSave
        | SettingId::PythonChecking) => {
            format!("{} {mark}", app.appearance.lsp.choice_label(id))
        }
        id @ (SettingId::AiEnabled | SettingId::AiAutoApprove | SettingId::AiStreaming | SettingId::AiProvider) => {
            format!("{} {mark}", app.ai.choice_label(id))
        }
        SettingId::AiServer => {
            if app.ai.server.trim().is_empty() {
                format!("{} (default)", app.ai.provider.default_server())
            } else {
                app.ai.server.clone()
            }
        }
        SettingId::AiModel => {
            if app.ai.model.trim().is_empty() {
                "(not set)".to_string()
            } else {
                app.ai.model.clone()
            }
        }
        SettingId::AiApiKey => {
            if app.ai.api_key.trim().is_empty() {
                "(not set)".to_string()
            } else {
                "••••••••".to_string()
            }
        }
    }
}

const MENU_KEYS: &str = "↑↓  enter  esc ";

/// Theme names in the same rows as the command list: full width, no border,
/// the highlighted name in the accent color, and the keys on that row.
fn paint_menu(
    frame: &mut Frame,
    inner: Rect,
    app: &App,
    page: &Page,
    left: usize,
    width: usize,
    anchor: Option<u16>,
    footer_row: u16,
) {
    let Some(menu) = page.menu.as_ref() else {
        return;
    };
    let Some(anchor) = anchor else {
        return;
    };
    let Some(id) = page.section().items.get(page.row).map(|item| item.id) else {
        return;
    };
    let names = match id {
        SettingId::ColorTheme => app.appearance.color_names(),
        SettingId::SyntaxTheme => app.appearance.syntax_names(),
        SettingId::TabWidth | SettingId::IndentWithSpaces | SettingId::LineEnding => {
            crate::settings::EditorPrefs::choice_names(id)
        }
        SettingId::TrimTrailingSpace | SettingId::FinalNewline => {
            crate::settings::FilePrefs::choice_names(id)
        }
        id @ (SettingId::LanguageServer
        | SettingId::CheckOnSave
        | SettingId::FormatOnSave
        | SettingId::PythonChecking) => crate::settings::LspPrefs::choice_names(id),
        id @ (SettingId::AiEnabled | SettingId::AiAutoApprove | SettingId::AiStreaming | SettingId::AiProvider) => {
            crate::settings::AiPrefs::choice_names(id)
        }
        SettingId::AiServer | SettingId::AiModel | SettingId::AiApiKey | SettingId::Binding(_) => return,
    };
    if names.is_empty() || width < 8 || footer_row == 0 {
        return;
    }
    let want = names.len() as u16;
    let room_below = footer_row.saturating_sub(anchor.saturating_add(1));
    let room_above = anchor;
    let (top, height) = if room_below >= 1 && (room_below >= want || room_below >= room_above) {
        (anchor + 1, want.min(room_below))
    } else if room_above >= 1 {
        let height = want.min(room_above);
        (anchor - height, height)
    } else {
        return;
    };
    if height == 0 {
        return;
    }
    let rect = Rect::new(
        inner.x.saturating_add(left as u16),
        inner.y.saturating_add(top),
        width as u16,
        height,
    );
    if rect.right() > inner.right() || rect.bottom() > inner.bottom() {
        return;
    }
    let theme = *app.styles();
    let name_width = names
        .iter()
        .map(|name| name.width() + 1)
        .max()
        .unwrap_or(1)
        .clamp(1, width);
    let slots = height as usize;
    let start = window_start(menu.index, names.len(), slots);
    for (offset, name) in names.iter().enumerate().skip(start).take(slots) {
        let selected = offset == menu.index;
        let (line, style) = menu_row(name, selected, name_width, width, &theme);
        let row = rect.y + (offset - start) as u16;
        frame.render_widget(
            Paragraph::new(line).style(style),
            Rect::new(rect.x, row, rect.width, 1),
        );
    }
}

fn menu_row(
    name: &str,
    selected: bool,
    name_width: usize,
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
    let head = pad_width(&format!(" {name}"), name_width.min(width));
    let keys = if selected && head.width() + MENU_KEYS.width() <= width {
        MENU_KEYS
    } else {
        ""
    };
    let gap = width.saturating_sub(head.width() + keys.width());
    let line = Line::from(vec![
        Span::styled(head, name_style),
        Span::styled(" ".repeat(gap), base),
        Span::styled(keys.to_string(), theme.hint_active_dim),
    ]);
    (line, base)
}

fn pad_width(text: &str, width: usize) -> String {
    let pad = width.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(pad))
}

fn paint_row(
    frame: &mut Frame,
    inner: Rect,
    x: usize,
    row: u16,
    label: &str,
    detail: Option<&str>,
    value: &str,
    selected: bool,
    canvas: Color,
    width: usize,
    item_height: usize,
    footer_row: u16,
    theme: &UiStyles,
    syntax: Option<&SyntaxTheme>,
) {
    let bg = if selected {
        theme.sidebar_selected
    } else {
        canvas
    };
    let label_style = if selected {
        theme.title_active.bg(bg)
    } else {
        theme.sidebar.bg(bg)
    };
    put_row(
        frame,
        inner,
        x,
        row,
        label,
        label_style,
        value,
        theme.sidebar_dim.bg(bg),
        width,
    );
    if let Some(detail) = detail
        && item_height == 2
        && row + 1 < footer_row
    {
        put_detail(
            frame,
            inner,
            x,
            row + 1,
            &format!(" {detail}"),
            theme.sidebar_dim.bg(bg),
            width,
            syntax.map(|syntax| syntax_sample(syntax, bg)),
        );
    }
}

fn syntax_sample(syntax: &SyntaxTheme, bg: Color) -> Vec<Span<'static>> {
    let paint = |text: &str, role: SyntaxRole| {
        Span::styled(text.to_string(), Style::new().fg(syntax.color(role)).bg(bg))
    };
    vec![
        paint("fn", SyntaxRole::Keyword),
        paint(" ", SyntaxRole::Text),
        paint("main", SyntaxRole::Function),
        paint("() { ", SyntaxRole::Punctuation),
        paint("\"hi\"", SyntaxRole::String),
        paint(" }", SyntaxRole::Punctuation),
    ]
}

fn put_detail(
    frame: &mut Frame,
    inner: Rect,
    x: usize,
    y: u16,
    text: &str,
    style: Style,
    width: usize,
    sample: Option<Vec<Span<'static>>>,
) {
    let Some(sample) = sample else {
        put(frame, inner, x, y, text, style, width);
        return;
    };
    let Some((rect, width)) = row_rect(inner, x, y, width) else {
        return;
    };
    let sample_width: usize = sample.iter().map(|span| span.content.width()).sum();
    let text = clip(text, width.saturating_sub(sample_width + 1));
    let gap = width.saturating_sub(text.width() + sample_width);
    let sample_fits = text.width() + gap + sample_width <= width;
    let mut spans = vec![
        Span::styled(text, style),
        Span::styled(" ".repeat(gap), style),
    ];
    if sample_fits {
        spans.extend(sample);
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), rect);
}

fn window_start(selected: usize, len: usize, slots: usize) -> usize {
    if slots == 0 || len <= slots {
        return 0;
    }
    let start = selected.saturating_sub(slots / 2);
    start.min(len - slots)
}

fn put(frame: &mut Frame, inner: Rect, x: usize, y: u16, text: &str, style: Style, width: usize) {
    let Some((rect, width)) = row_rect(inner, x, y, width) else {
        return;
    };
    frame.render_widget(Paragraph::new(fill(text, width, style)).style(style), rect);
}

/// Block cursor on the cell where the next typed character will land.
fn place_draft_cursor(inner: Rect, x: usize, y: u16, label: &str, value: &str, width: usize) {
    let Some((rect, width)) = row_rect(inner, x, y, width) else {
        return;
    };
    let value_width = value.width().min(width);
    let label_budget = width.saturating_sub(value_width.saturating_add(1));
    let label = clip(label, label_budget);
    let gap = width.saturating_sub(label.width().saturating_add(value_width));
    let column = label
        .width()
        .saturating_add(gap)
        .saturating_add(value_width.saturating_sub(1))
        .min(width.saturating_sub(1));
    super::cursor::place(rect.x.saturating_add(column as u16), rect.y);
}

fn put_row(
    frame: &mut Frame,
    inner: Rect,
    x: usize,
    y: u16,
    label: &str,
    label_style: Style,
    value: &str,
    value_style: Style,
    width: usize,
) {
    let Some((rect, width)) = row_rect(inner, x, y, width) else {
        return;
    };
    let value_width = value.width();
    let label_budget = width.saturating_sub(value_width + 1);
    let label = clip(label, label_budget);
    let gap = width.saturating_sub(label.width() + value_width);
    let line = Line::from(vec![
        Span::styled(label, label_style),
        Span::styled(" ".repeat(gap), label_style),
        Span::styled(value.to_string(), value_style),
    ]);
    frame.render_widget(Paragraph::new(line), rect);
}

fn row_rect(inner: Rect, x: usize, y: u16, width: usize) -> Option<(Rect, usize)> {
    if y >= inner.height || width == 0 {
        return None;
    }
    let x = inner.x.saturating_add(x as u16);
    if x >= inner.right() {
        return None;
    }
    let width = (width as u16).min(inner.right().saturating_sub(x));
    (width > 0).then_some((Rect::new(x, inner.y + y, width, 1), width as usize))
}

fn fill(text: &str, width: usize, style: Style) -> Line<'static> {
    let text = clip(text, width);
    let gap = width.saturating_sub(text.width());
    Line::from(vec![
        Span::styled(text, style),
        Span::styled(" ".repeat(gap), style),
    ])
}

fn clip(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out.push('…');
    out
}
