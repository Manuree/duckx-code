use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::panel;
use crate::app::App;

/// The duck, then `CODE`. Leading spaces on each line are part of the picture,
/// so every line shares one left edge.
const LOGO: &[&str] = &[
    "       ▄▀▀▀▀▀▄",
    "     ▄▀     ▄ ▀▄",
    "     █         █",
    "     █      ▀▀▀▀▀▀▀▄",
    "        ▀▄         █",
    " █▀▀▀▀▀▀ ▄▀   ▀█▀▀▀",
    " ▀▄     ▀     ▄▀",
    "  ▀▄         ▄▀",
    "    ▀▀▄▄▄▄▄▀▀  CODE",
];

const NAME: &str = "Duckx Code";
const BLURB: &str = "A modeless editor for 75% keyboards.";

/// Logo, name, and version over the editor. `Esc` closes the page.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if !app.about || area.width < 8 || area.height < 4 {
        return;
    }

    let theme = *app.styles();
    let block = panel(" About ")
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
    let fill = Style::new().bg(canvas);
    let mark = Style::new().fg(theme.editor_fg).bg(canvas);
    let name = mark.add_modifier(Modifier::BOLD);
    let dim = theme.sidebar_dim.bg(canvas);
    let width = inner.width as usize;
    let footer_row = inner.height.saturating_sub(1);

    let version = format!("version {}", env!("CARGO_PKG_VERSION"));
    let captions = [NAME, version.as_str(), BLURB];
    let block_rows = LOGO.len() + 1 + captions.len();
    let start = footer_row.saturating_sub(block_rows as u16) / 2;
    let logo_width = LOGO.iter().map(|line| line.width()).max().unwrap_or(0);
    let left = width.saturating_sub(logo_width.min(width)) / 2;

    for (offset, line) in LOGO.iter().enumerate() {
        let row = start + offset as u16;
        if row >= footer_row {
            break;
        }
        put(frame, inner, row, placed(width, left, line, mark, fill));
    }
    let caption_at = start + LOGO.len() as u16 + 1;
    for (offset, line) in captions.iter().enumerate() {
        let row = caption_at + offset as u16;
        if row >= footer_row {
            break;
        }
        let style = if offset == 0 { name } else { dim };
        put(frame, inner, row, centered(width, line, style, fill));
    }
    put(
        frame,
        inner,
        footer_row,
        Paragraph::new(clip(" esc close", width)).style(dim),
    );
}

/// `left` is the shared margin. Spaces inside `text` stay, so the rows line up.
fn placed(width: usize, left: usize, text: &str, style: Style, fill: Style) -> Line<'static> {
    let left = left.min(width);
    let text = clip(text, width - left);
    let used = left + text.width();
    let mut spans = Vec::new();
    if left > 0 {
        spans.push(Span::styled(" ".repeat(left), fill));
    }
    spans.push(Span::styled(text, style));
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), fill));
    }
    Line::from(spans)
}

fn centered(width: usize, text: &str, style: Style, fill: Style) -> Line<'static> {
    let pad = width.saturating_sub(text.width().min(width)) / 2;
    placed(width, pad, text, style, fill)
}

fn put(frame: &mut Frame, inner: Rect, y: u16, widget: impl ratatui::widgets::Widget) {
    if y >= inner.height {
        return;
    }
    let rect = Rect::new(inner.x, inner.y + y, inner.width, 1);
    frame.render_widget(widget, rect);
}

fn clip(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}
