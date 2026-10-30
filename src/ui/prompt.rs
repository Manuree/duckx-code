use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::panel;
use crate::app::{App, DialogButton};
use crate::theme::UiStyles;

const PAD: usize = 1;
const GAP: &str = "  ";

/// Confirmation dialog. Buttons, title, and message all come from the prompt,
/// so delete and unsaved-changes share this widget.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.prompt.is_none() || area.width < 8 || area.height < 5 {
        return;
    }
    let theme = *app.styles();
    let Some(prompt) = &app.prompt else {
        return;
    };

    let buttons_width = button_row_width(&prompt.buttons);
    let text_width = prompt.message.width().max(buttons_width);
    let width = ((text_width + PAD * 2 + 2) as u16).clamp(8, area.width);
    let budget = (width as usize).saturating_sub(2 + PAD * 2);
    let message = truncate(&prompt.message, budget);
    let height = 5.min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let canvas = Style::new().bg(theme.editor_bg);
    let block = panel(&prompt.title)
        .border_style(theme.command_border)
        .style(canvas);
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let lines = vec![
        padded(Span::styled(
            message,
            theme.dialog_message.bg(theme.editor_bg),
        )),
        Line::raw(""),
        button_line(
            &prompt.buttons,
            prompt.selected,
            &theme,
            inner.width as usize,
        ),
    ];
    frame.render_widget(Paragraph::new(lines).style(canvas), inner);
}

fn padded(span: Span<'_>) -> Line<'_> {
    Line::from(vec![Span::raw(" ".repeat(PAD)), span])
}

fn button_line(
    buttons: &[DialogButton],
    selected: usize,
    theme: &UiStyles,
    inner_width: usize,
) -> Line<'static> {
    let lead = inner_width.saturating_sub(button_row_width(buttons)) / 2;
    let mut spans = vec![Span::styled(
        " ".repeat(lead),
        Style::new().bg(theme.editor_bg),
    )];
    for (index, button) in buttons.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(GAP, Style::new().bg(theme.editor_bg)));
        }
        spans.extend(button_spans(button, index == selected, theme));
    }
    Line::from(spans)
}

fn button_spans(button: &DialogButton, selected: bool, theme: &UiStyles) -> Vec<Span<'static>> {
    let style = if selected {
        theme.dialog_button_active
    } else {
        theme.dialog_button.bg(theme.editor_bg)
    };
    let Some(mnemonic) = button.mnemonic else {
        return vec![Span::styled(button.label.to_string(), style)];
    };
    let Some((at, ch)) = button
        .label
        .char_indices()
        .find(|(_, ch)| ch.eq_ignore_ascii_case(&mnemonic))
    else {
        return vec![Span::styled(button.label.to_string(), style)];
    };
    let underlined = style.add_modifier(Modifier::UNDERLINED);
    let mut spans = Vec::new();
    if at > 0 {
        spans.push(Span::styled(button.label[..at].to_string(), style));
    }
    spans.push(Span::styled(ch.to_string(), underlined));
    let rest = at + ch.len_utf8();
    if rest < button.label.len() {
        spans.push(Span::styled(button.label[rest..].to_string(), style));
    }
    spans
}

fn button_row_width(buttons: &[DialogButton]) -> usize {
    let labels: usize = buttons.iter().map(|button| button.label.width()).sum();
    let gaps = buttons.len().saturating_sub(1) * GAP.width();
    labels + gaps
}

fn truncate(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + width > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += width;
    }
    if max > 0 {
        out.push('…');
    }
    out
}
