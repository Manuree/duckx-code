use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::View;
use crate::buffer::{Buffer, Selection, grapheme_width};
use crate::lsp::Severity;
use crate::services::git::{FileDiff, LineKind};
use crate::theme::{SyntaxRole, SyntaxTheme, UiStyles};

/// `diagnostics` are char ranges with their level, as from `Tab::diagnostic_spans`.
pub fn render(
    frame: &mut Frame,
    buffer: &Buffer,
    view: View,
    gutter: Rect,
    text: Rect,
    focused: bool,
    theme: &UiStyles,
    syntax: &SyntaxTheme,
    highlights: &[(usize, usize, SyntaxRole)],
    diagnostics: &[(usize, usize, Severity)],
    show_diff: bool,
    diff: Option<&FileDiff>,
) {
    let selection = buffer.selection();
    let (cursor_line, cursor_col) = buffer.cursor_line_col();
    let number_width = (gutter.width as usize).saturating_sub(2 + usize::from(show_diff));

    let mut gutter_lines = Vec::with_capacity(text.height as usize);
    let mut text_lines = Vec::with_capacity(text.height as usize);
    for row in 0..text.height as usize {
        let line = view.scroll_y + row;
        if line >= buffer.len_lines() {
            let mut spans = Vec::new();
            if show_diff {
                spans.push(diff_span(diff, line, theme.gutter, theme));
            }
            spans.push(Span::styled(
                format!(" {:>number_width$} ", "~"),
                theme.gutter,
            ));
            gutter_lines.push(Line::from(spans));
            text_lines.push(Line::default());
            continue;
        }
        let style = if focused && line == cursor_line {
            theme.gutter_active
        } else {
            theme.gutter
        };
        let marker = diagnostics
            .iter()
            .filter(|(start, _, _)| buffer.rope().char_to_line(*start) == line)
            .map(|(_, _, severity)| *severity)
            .min();
        let marker = match marker {
            Some(severity) => Span::styled(
                severity.letter().to_string(),
                style
                    .fg(theme.diagnostic(severity))
                    .add_modifier(Modifier::BOLD),
            ),
            None => Span::styled(" ", style),
        };
        let mut spans = Vec::new();
        if show_diff {
            spans.push(diff_span(diff, line, style, theme));
        }
        spans.push(marker);
        spans.push(Span::styled(format!("{:>number_width$} ", line + 1), style));
        gutter_lines.push(Line::from(spans));
        text_lines.push(render_line(
            buffer,
            line,
            selection,
            view.scroll_x,
            text.width as usize,
            theme,
            syntax,
            highlights,
            diagnostics,
        ));
    }

    let canvas = Style::new()
        .fg(syntax.color(SyntaxRole::Text))
        .bg(theme.editor_bg);
    frame.render_widget(Paragraph::new(gutter_lines).style(canvas), gutter);
    frame.render_widget(Paragraph::new(text_lines).style(canvas), text);

    let row = cursor_line.checked_sub(view.scroll_y);
    let col = cursor_col.checked_sub(view.scroll_x);
    if focused
        && let (Some(row), Some(col)) = (row, col)
        && row < text.height as usize
        && col < text.width as usize
    {
        frame.set_cursor_position((text.x + col as u16, text.y + row as u16));
    }
}

/// A column to the left of the diagnostic mark. `‾` is a deletion sitting above this
/// row; a digit is how many lines were removed; `┃` is an added or modified line.
fn diff_span(
    diff: Option<&FileDiff>,
    line: usize,
    style: Style,
    theme: &UiStyles,
) -> Span<'static> {
    let (kind, deleted) = diff
        .map(|diff| (diff.kind(line), diff.deleted_above(line)))
        .unwrap_or((None, 0));
    let (glyph, color) = match (kind, deleted) {
        (None, 0) => (" ".to_string(), None),
        (None, 1) => ("‾".to_string(), Some(theme.git_deleted)),
        (None, count) if count < 10 => (count.to_string(), Some(theme.git_deleted)),
        (None, _) => ("+".to_string(), Some(theme.git_deleted)),
        (Some(LineKind::Added), 0) => ("┃".to_string(), Some(theme.git_added)),
        (Some(LineKind::Modified), 0) => ("┃".to_string(), Some(theme.git_modified)),
        (_, 1) => ("~".to_string(), Some(theme.git_deleted)),
        (_, count) if count < 10 => (count.to_string(), Some(theme.git_deleted)),
        (_, _) => ("+".to_string(), Some(theme.git_deleted)),
    };
    match color {
        Some(color) => Span::styled(glyph, style.fg(color)),
        None => Span::styled(glyph, style),
    }
}

/// Merges consecutive graphemes that share a style into one span.
#[derive(Default)]
struct LineBuilder {
    spans: Vec<Span<'static>>,
    current: String,
    style: Style,
}

impl LineBuilder {
    fn push(&mut self, text: &str, style: Style) {
        if style != self.style && !self.current.is_empty() {
            self.flush();
        }
        self.style = style;
        self.current.push_str(text);
    }

    fn flush(&mut self) {
        let text = std::mem::take(&mut self.current);
        self.spans.push(Span::styled(text, self.style));
    }

    fn finish(mut self) -> Line<'static> {
        if !self.current.is_empty() {
            self.flush();
        }
        Line::from(self.spans)
    }
}

fn render_line(
    buffer: &Buffer,
    line: usize,
    selection: Selection,
    scroll_x: usize,
    width: usize,
    theme: &UiStyles,
    syntax: &SyntaxTheme,
    highlights: &[(usize, usize, SyntaxRole)],
    diagnostics: &[(usize, usize, Severity)],
) -> Line<'static> {
    let line_start = buffer.rope().line_to_char(line);
    let content = buffer.line_content(line);
    let right_edge = scroll_x + width;
    let role_at = |pos: usize| {
        let mut role = SyntaxRole::Text;
        for &(start, end, role_n) in highlights {
            if start > pos {
                break;
            }
            if pos < end {
                role = role_n;
            }
        }
        role
    };
    let style_at = |pos: usize| {
        let mut style = Style::new()
            .fg(syntax.color(role_at(pos)))
            .bg(theme.editor_bg);
        if pos >= selection.from() && pos < selection.to() {
            style = style.patch(theme.selection);
        }
        let worst = diagnostics
            .iter()
            .filter(|(start, end, _)| *start <= pos && pos < *end)
            .map(|(_, _, severity)| *severity)
            .min();
        if let Some(severity) = worst {
            // Same color as the diagnostic bar. An underline makes Windows Terminal
            // draw these glyphs far darker than that color, so the text is bold instead.
            style = style
                .fg(theme.diagnostic(severity))
                .add_modifier(Modifier::BOLD);
        }
        style
    };

    let mut builder = LineBuilder::default();
    let mut col = 0;
    let mut vcol = 0;
    for g in content.graphemes(true) {
        if vcol >= right_edge {
            break;
        }
        let w = grapheme_width(g, vcol, buffer.tab_width());
        let end = vcol + w;
        if end > scroll_x {
            let style = style_at(line_start + col);
            let clipped = vcol < scroll_x || end > right_edge;
            if clipped || g == "\t" {
                let visible = end.min(right_edge) - vcol.max(scroll_x);
                builder.push(&" ".repeat(visible), style);
            } else {
                builder.push(g, style);
            }
        }
        col += g.chars().count();
        vcol = end;
    }

    let line_end = line_start + col;
    let line_break_selected = selection.from() <= line_end && selection.to() > line_end;
    if line_break_selected && vcol >= scroll_x && vcol < right_edge {
        builder.push(
            " ",
            Style::new()
                .fg(syntax.color(SyntaxRole::Text))
                .patch(theme.selection),
        );
    }
    builder.finish()
}
