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
    let text_row = row_id.is_some_and(crate::settings::AiPrefs::is_text);
    let has_picks = page.text.is_some() && app.draft_row().is_some();
    let recent_row = matches!(row_id, Some(SettingId::AiServer | SettingId::AiModel));
    let recent_hint = row_id
        .filter(|_| recent_row)
        .map(|id| match app.ai.recent_for(id).len() {
            0 => " ↑↓ move    enter edit · recent list    del reset    esc back ← back".to_string(),
            n => format!(" ↑↓ move    enter edit · {n} recent    del reset    esc back ← back"),
        });
    let footer = if has_picks {
        " type    ↑↓ recent    del forget    enter save    esc cancel"
    } else if page.text.is_some() {
        " type    ←→ move    enter save    esc cancel"
    } else if page.capture {
        " press a key    esc cancel"
    } else if page.menu.is_some() {
        " ↑↓ move    enter select    esc close"
    } else if !page.inside {
        " ↑↓ move    enter → open    del reset    esc back ← close"
    } else if binding {
        " ↑↓ move    enter bind    del reset    esc back ← back"
    } else if let Some(hint) = recent_hint.as_deref() {
        hint
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
            paint_picks(frame, inner, app, 0, width, anchor, footer_row);
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
        paint_picks(frame, inner, app, left, body_width, anchor, footer_row);
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
            false,
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
        let draft = page
            .text
            .as_ref()
            .filter(|_| page.inside && offset == page.row);
        let dim = !app.ai.applies(item.id);
        let syntax = (item.id == SettingId::SyntaxTheme).then(|| app.appearance.syntax.clone());
        let label = format!(" {}", item.label);
        let (value, draft_column) = match draft {
            Some(draft) => {
                let (value, column) =
                    draft_window(draft, item.id == SettingId::AiApiKey, &label, width);
                (value, Some(column))
            }
            None => {
                let value = choice_label(app, item.id, open, capturing);
                let value = if dim {
                    format!("{} · {}", value.trim_end_matches(" ▾"), only_for(item.id))
                } else {
                    value
                };
                (value, None)
            }
        };
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
            dim,
        );
        if let Some(column) = draft_column {
            place_draft_cursor(inner, x, row, column, width);
        }
        if open || draft.is_some() {
            anchor = Some(row);
        }
    }
    anchor
}

/// Which provider a greyed row waits for.
fn only_for(id: SettingId) -> &'static str {
    match id {
        SettingId::AiAppName | SettingId::AiReferer => "OpenRouter only",
        SettingId::AiProviderName
        | SettingId::AiProviderKind
        | SettingId::AiProviderKeyEnv
        | SettingId::AiProviderModels
        | SettingId::AiProviderDelete => "named only",
        _ => "Ollama only",
    }
}

/// The part of a draft that fits beside the label, with the cursor kept on screen,
/// and the cursor's column inside the row. An API key is drawn as one dot per letter.
fn draft_window(
    draft: &crate::settings::TextDraft,
    masked: bool,
    label: &str,
    width: usize,
) -> (String, usize) {
    use unicode_segmentation::UnicodeSegmentation;
    let text = draft.value();
    let cursor = draft.input.cursor();
    let (shown_text, shown_cursor) = if masked {
        let dots = "•".repeat(text.graphemes(true).count());
        let before = text[..cursor].graphemes(true).count() * "•".len();
        (dots, before)
    } else {
        (text.to_string(), cursor)
    };
    // Leave at least the first letters of the label and a gap.
    let budget = width
        .saturating_sub(label.width().min(width / 2) + 1)
        .max(1);
    let (start, end) = crate::picker::query_window(&shown_text, shown_cursor, budget);
    let mut value = shown_text.get(start..end).unwrap_or("").to_string();
    if end == shown_text.len() {
        value.push(' ');
    }
    let before = shown_text
        .get(start..shown_cursor.clamp(start, end))
        .unwrap_or("")
        .width();
    let column = width.saturating_sub(value.width()) + before;
    (value, column.min(width.saturating_sub(1)))
}

fn choice_label(app: &App, id: SettingId, open: bool, capturing: bool) -> String {
    if capturing {
        return "press a key".to_string();
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
        id @ (SettingId::AiEnabled
        | SettingId::AiAutoApprove
        | SettingId::AiStreaming
        | SettingId::AiProvider
        | SettingId::AiThink
        | SettingId::AiAutoCompact
        | SettingId::AiModelTools
        | SettingId::AiProviderKind) => {
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
        SettingId::AiApiKey => match app.ai.resolved_api_key().1 {
            crate::settings::KeySource::Env(name) => format!("•••••••• (from {name})"),
            crate::settings::KeySource::Saved => "••••••••".to_string(),
            crate::settings::KeySource::None => match app.ai.key_env_name() {
                Some(name) => format!("(not set · or set {name})"),
                None => "(not set)".to_string(),
            },
        },
        SettingId::AiProviderAdd => "enter, then type a name".to_string(),
        SettingId::AiProviderName => match app.ai.active_named() {
            Some(index) => app.ai.named[index].name.clone(),
            None => "(built-in)".to_string(),
        },
        SettingId::AiProviderKeyEnv => match app.ai.active_named() {
            Some(index) => text_or_none(&app.ai.named[index].api_key_env),
            None => "(built-in)".to_string(),
        },
        SettingId::AiProviderModels => match app.ai.active_named() {
            Some(index) if app.ai.named[index].models.is_empty() => "(none)".to_string(),
            Some(index) => app.ai.named[index].models.join(", "),
            None => "(built-in)".to_string(),
        },
        SettingId::AiProviderDelete => match app.ai.active_named() {
            Some(index) => format!("enter twice removes {}", app.ai.named[index].name),
            None => "(built-in)".to_string(),
        },
        SettingId::AiContextSize => {
            let raw = app.ai.context_size.trim();
            if raw.is_empty() {
                format!("{} (default)", app.ai.global_context_size())
            } else {
                app.ai.context_size.clone()
            }
        }
        SettingId::AiModelContext => match (app.ai.model_key(), app.ai.model_limits().context) {
            (None, _) => "(set a model first)".to_string(),
            (Some(_), Some(n)) => n.to_string(),
            (Some(_), None) => format!("{} (Context size)", app.ai.global_context_size()),
        },
        SettingId::AiModelMaxOutput => {
            match (app.ai.model_key(), app.ai.model_limits().max_output) {
                (None, _) => "(set a model first)".to_string(),
                (Some(_), Some(n)) => n.to_string(),
                (Some(_), None) => match app.ai.global_max_tokens() {
                    Some(n) => format!("{n} (Max tokens)"),
                    None => "(omit · Max tokens)".to_string(),
                },
            }
        }
        SettingId::AiMaxTokens => match (app.ai.max_tokens.trim(), app.ai.global_max_tokens()) {
            ("", Some(n)) => format!("{n} (default)"),
            ("", None) => "(omit)".to_string(),
            (_, Some(n)) => n.to_string(),
            (_, None) => "off (omit)".to_string(),
        },
        SettingId::AiTimeout => format!("{} s", app.ai.resolved_timeout()),
        SettingId::AiRetries => app.ai.resolved_retries().to_string(),
        SettingId::AiDetected => match (app.ai.model_key(), app.ai.detected_now()) {
            (None, _) => "(set a model first)".to_string(),
            (Some(_), (false, false)) => "none".to_string(),
            (Some(_), (tools, think)) => {
                let what = match (tools, think) {
                    (true, true) => "no tools, no think",
                    (true, false) => "no tools",
                    _ => "no think",
                };
                format!("{what} · enter forgets")
            }
        },
        SettingId::AiClearRecent => match app.ai.recent.len() {
            0 => "(empty)".to_string(),
            n => format!("{n} saved · enter clears"),
        },
        SettingId::AiAppName => text_or_none(&app.ai.openrouter_app_name),
        SettingId::AiReferer => text_or_none(&app.ai.openrouter_referer),
        SettingId::AiCompactKeep => format!("{} tokens", app.ai.resolved_compact_keep()),
        SettingId::AiCompactBuffer => format!("{} %", app.ai.resolved_compact_buffer()),
    }
}

fn text_or_none(value: &str) -> String {
    if value.trim().is_empty() {
        "(none)".to_string()
    } else {
        value.to_string()
    }
}

const MENU_KEYS: &str = "↑↓ move  enter select  esc";
const PICK_KEYS: &str = "↑↓ pick  del forget  enter save";

/// Most entries the recent list under a text row shows at once.
const PICK_ROWS: usize = 8;

/// Narrowest a settings list is drawn, so short names still read as a list.
const MIN_LIST_WIDTH: u16 = 30;

/// Where a settings list goes: right-aligned under the row at `anchor`, in the
/// column that starts at `left`, above the footer.
fn list_rect(
    inner: Rect,
    left: usize,
    width: usize,
    anchor: u16,
    footer_row: u16,
    list: &super::dropdown::List,
    max_rows: usize,
) -> Option<Rect> {
    let column = Rect::new(
        inner.x.saturating_add(left as u16),
        inner.y,
        (width as u16).min(inner.width.saturating_sub(left as u16)),
        footer_row.min(inner.height),
    );
    let list_width = list.natural_width().max(MIN_LIST_WIDTH).min(column.width);
    let x = column.right().saturating_sub(list_width);
    super::dropdown::anchored(
        column,
        x,
        list_width,
        inner.y + anchor,
        super::dropdown::frame_height(list.rows.len(), max_rows) + u16::from(!list.note.is_empty()),
    )
}

/// The list under the Server or Model row being typed: recent values first, then
/// (for Model) the server's models. The highlighted entry is the one in the row.
fn paint_picks(
    frame: &mut Frame,
    inner: Rect,
    app: &App,
    left: usize,
    width: usize,
    anchor: Option<u16>,
    footer_row: u16,
) {
    let Some(anchor) = anchor else {
        return;
    };
    let Some(draft) = app.settings.as_ref().and_then(|page| page.text.as_ref()) else {
        return;
    };
    if app.draft_row().is_none() {
        return;
    }
    // The list opens as soon as Server or Model is being typed, even with
    // nothing remembered yet, so it is clear where recent values will show.
    let picks = app.draft_picks();
    let rows: Vec<super::dropdown::Row> = picks
        .iter()
        .map(|(value, source)| super::dropdown::Row::new(value.clone()).tag(source.tag()))
        .collect();
    let empty = if draft.filter.trim().is_empty() {
        "no recent values yet · saved when you press enter"
    } else {
        "no match · enter saves what you typed"
    };
    let note = app.draft_note();
    let list = super::dropdown::List::new(" Recent ", PICK_KEYS, &rows)
        .selected(draft.pick)
        .empty(empty)
        .note(&note);
    let Some(rect) = list_rect(inner, left, width, anchor, footer_row, &list, PICK_ROWS) else {
        return;
    };
    super::dropdown::render(frame, rect, &list, app.styles());
}

/// A setting's choices (themes, provider, on/off) in a framed list under its row.
/// `Up` / `Down` preview, `Enter` keeps, `Esc` restores.
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
    let Some(item) = page.section().items.get(page.row) else {
        return;
    };
    let id = item.id;
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
        id @ (SettingId::AiEnabled
        | SettingId::AiAutoApprove
        | SettingId::AiStreaming
        | SettingId::AiProvider
        | SettingId::AiThink
        | SettingId::AiAutoCompact
        | SettingId::AiModelTools
        | SettingId::AiProviderKind) => app.ai.choice_names_with(id),
        SettingId::AiServer
        | SettingId::AiModel
        | SettingId::AiApiKey
        | SettingId::AiContextSize
        | SettingId::AiMaxTokens
        | SettingId::AiModelContext
        | SettingId::AiModelMaxOutput
        | SettingId::AiTimeout
        | SettingId::AiRetries
        | SettingId::AiAppName
        | SettingId::AiReferer
        | SettingId::AiProviderAdd
        | SettingId::AiProviderName
        | SettingId::AiProviderKeyEnv
        | SettingId::AiProviderModels
        | SettingId::AiProviderDelete
        | SettingId::AiClearRecent
        | SettingId::AiCompactKeep
        | SettingId::AiCompactBuffer
        | SettingId::AiDetected
        | SettingId::Binding(_) => return,
    };
    if names.is_empty() {
        return;
    }
    let rows: Vec<super::dropdown::Row> = names
        .iter()
        .map(|name| super::dropdown::Row::new(name.to_string()))
        .collect();
    let title = format!(" {} ", item.label);
    let list = super::dropdown::List::new(&title, MENU_KEYS, &rows).selected(Some(menu.index));
    let Some(rect) = list_rect(inner, left, width, anchor, footer_row, &list, rows.len()) else {
        return;
    };
    super::dropdown::render(frame, rect, &list, app.styles());
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
    dim: bool,
) {
    let bg = if selected {
        theme.sidebar_selected
    } else {
        canvas
    };
    let label_style = if dim {
        theme.sidebar_dim.bg(bg)
    } else if selected {
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
fn place_draft_cursor(inner: Rect, x: usize, y: u16, column: usize, width: usize) {
    let Some((rect, width)) = row_rect(inner, x, y, width) else {
        return;
    };
    let column = column.min(width.saturating_sub(1));
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
