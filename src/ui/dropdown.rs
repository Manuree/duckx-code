//! The one look for every list that drops down over other content: settings
//! choices, recent values, command-bar matches, assistant `/` and `@` lists, and
//! the editor's completion, references, and actions popups.
//!
//! A list is a rounded panel (R1) on the editor background with the accent
//! border, so it never blends into the rows behind it. The highlighted row is a
//! full-width bar with a `›` marker, the keys sit in the bottom border, tags sit
//! at the right edge, and a `n/len` counter appears in the top border when the
//! list scrolls. Areas too small for a frame fall back to plain rows.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::panel;
use crate::theme::UiStyles;

/// The highlighted row starts with this marker; other rows get spaces.
const MARKER: &str = "› ";
const GAP: &str = "  ";

/// One entry: the name, an optional dim description after it, and an optional
/// tag drawn at the right edge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Row {
    pub label: String,
    pub detail: String,
    pub tag: String,
}

impl Row {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = tag.into();
        self
    }
}

pub(crate) struct List<'a> {
    pub title: &'a str,
    /// Drawn at the right of the bottom border.
    pub keys: &'a str,
    pub rows: &'a [Row],
    pub selected: Option<usize>,
    /// Shown when `rows` is empty.
    pub empty: &'a str,
    /// A dim line under the rows (completion docs). Empty for none.
    pub note: &'a str,
    /// Labels are padded to this width so details line up. 0 keeps them natural.
    pub label_width: usize,
}

impl<'a> List<'a> {
    pub fn new(title: &'a str, keys: &'a str, rows: &'a [Row]) -> Self {
        Self {
            title,
            keys,
            rows,
            selected: None,
            empty: "",
            note: "",
            label_width: 0,
        }
    }

    pub fn selected(mut self, selected: Option<usize>) -> Self {
        self.selected = selected;
        self
    }

    pub fn empty(mut self, empty: &'a str) -> Self {
        self.empty = empty;
        self
    }

    pub fn note(mut self, note: &'a str) -> Self {
        self.note = note;
        self
    }

    pub fn label_width(mut self, width: usize) -> Self {
        self.label_width = width;
        self
    }

    /// Columns the list wants: the widest row plus marker, gaps, and borders,
    /// and never narrower than its title or keys.
    pub fn natural_width(&self) -> u16 {
        let widest = self
            .rows
            .iter()
            .map(|row| {
                let mut width = MARKER.width() + row.label.width().max(self.label_width);
                if !row.detail.is_empty() {
                    width += GAP.width() + row.detail.width();
                }
                if !row.tag.is_empty() {
                    width += GAP.width() + row.tag.width();
                }
                width + 1
            })
            .max()
            .unwrap_or(MARKER.width() + self.empty.width() + 1);
        let chrome = self.title.width().max(self.keys.width()) + 6;
        (widest + 2).max(chrome).min(u16::MAX as usize) as u16
    }
}

/// Outer height for `rows` entries, at most `max` visible, plus the border.
pub(crate) fn frame_height(rows: usize, max: usize) -> u16 {
    rows.clamp(1, max.max(1)).min(u16::MAX as usize - 2) as u16 + 2
}

/// A rectangle `width` x `height` inside `bounds`, directly under the row at
/// `anchor_y`, or above it when there is more room there. Wider than `bounds`
/// is clipped; `x` moves left so the list stays inside. `None` when no row fits.
pub(crate) fn anchored(
    bounds: Rect,
    x: u16,
    width: u16,
    anchor_y: u16,
    height: u16,
) -> Option<Rect> {
    if bounds.width == 0 || bounds.height == 0 || width == 0 || height == 0 {
        return None;
    }
    let width = width.min(bounds.width);
    let x = x.clamp(bounds.x, bounds.right() - width);
    let below = anchor_y.saturating_add(1).max(bounds.y);
    let room_below = bounds.bottom().saturating_sub(below);
    let room_above = anchor_y.min(bounds.bottom()).saturating_sub(bounds.y);
    let (y, height) = if room_below >= height || room_below >= room_above {
        (below, height.min(room_below))
    } else {
        let height = height.min(room_above);
        (anchor_y - height, height)
    };
    (height > 0).then(|| Rect::new(x, y, width, height))
}

/// The first row index shown so `selected` stays in view, kept near the middle.
pub(crate) fn window_start(selected: usize, len: usize, slots: usize) -> usize {
    if slots == 0 || len <= slots {
        return 0;
    }
    selected.saturating_sub(slots / 2).min(len - slots)
}

struct Colors {
    base: Style,
    dim: Style,
    on: Style,
    on_name: Style,
    on_dim: Style,
    border: Style,
}

fn colors(theme: &UiStyles) -> Colors {
    let bg = theme.editor_bg;
    let bar = theme.selection.bg.unwrap_or(theme.editor_bg);
    let accent = theme.hint_active_name;
    // Descriptions and tags use the hint text color, which stays readable on
    // both the list background and the bar (the editor's `dim` is too faint).
    let quiet = theme.hint_dim.fg.unwrap_or(theme.editor_fg);
    Colors {
        base: Style::new().fg(theme.editor_fg).bg(bg),
        dim: Style::new().fg(quiet).bg(bg),
        on: Style::new().fg(theme.editor_fg).bg(bar),
        on_name: accent.bg(bar),
        on_dim: Style::new().fg(theme.editor_fg).bg(bar),
        border: theme.command_border.bg(bg),
    }
}

/// Draws `list` into `area`, clearing what is behind it first.
pub(crate) fn render(frame: &mut Frame, area: Rect, list: &List, theme: &UiStyles) {
    let area = area.intersection(frame.area());
    if area.width == 0 || area.height == 0 {
        return;
    }
    let colors = colors(theme);
    frame.render_widget(Clear, area);
    let framed = area.width >= 6 && area.height >= 3;
    let inner = if framed {
        let len = list.rows.len();
        let note_rows = u16::from(!list.note.is_empty() && area.height >= 4);
        let slots = area.height.saturating_sub(2 + note_rows) as usize;
        let mut block = panel(list.title)
            .border_style(colors.border)
            .style(colors.base);
        if !list.keys.is_empty() {
            block =
                block.title_bottom(Line::from(format!(" {} ", list.keys.trim())).right_aligned());
        }
        if len > slots {
            let at = list.selected.map_or(0, |index| index.min(len - 1) + 1);
            block = block.title(Line::from(format!(" {at}/{len} ")).right_aligned());
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    } else {
        frame.render_widget(Paragraph::new("").style(colors.base), area);
        area
    };
    let note_rows = u16::from(framed && !list.note.is_empty() && inner.height >= 2);
    let slots = inner.height.saturating_sub(note_rows) as usize;
    let width = inner.width as usize;
    if list.rows.is_empty() {
        let line = Line::from(Span::styled(
            clip(&format!("  {}", list.empty), width),
            colors.dim,
        ));
        frame.render_widget(
            Paragraph::new(line).style(colors.base),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
    } else {
        let selected = list.selected.map(|index| index.min(list.rows.len() - 1));
        let start = window_start(selected.unwrap_or(0), list.rows.len(), slots);
        for (offset, row) in list.rows.iter().enumerate().skip(start).take(slots) {
            let on = selected == Some(offset);
            let y = inner.y + (offset - start) as u16;
            let (line, style) = row_line(row, on, list.label_width, width, &colors);
            frame.render_widget(
                Paragraph::new(line).style(style),
                Rect::new(inner.x, y, inner.width, 1),
            );
        }
    }
    if note_rows == 1 {
        let line = Line::from(Span::styled(
            clip(&format!("  {}", list.note), width),
            colors.dim,
        ));
        frame.render_widget(
            Paragraph::new(line).style(colors.base),
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        );
    }
}

fn row_line(
    row: &Row,
    on: bool,
    label_width: usize,
    width: usize,
    colors: &Colors,
) -> (Line<'static>, Style) {
    let (base, name, dim) = if on {
        (colors.on, colors.on_name, colors.on_dim)
    } else {
        (colors.base, colors.base, colors.dim)
    };
    let marker = if on { MARKER } else { "  " };
    let marker = clip(marker, width);
    let mut room = width.saturating_sub(marker.width());
    // The tag keeps its place at the right while the name still gets 8 columns.
    let tag = if row.tag.is_empty() || room < row.tag.width() + GAP.width() + 8 {
        String::new()
    } else {
        format!("{} ", row.tag)
    };
    room = room.saturating_sub(tag.width());
    let label = clip(&pad(&row.label, label_width), room);
    room = room.saturating_sub(label.width());
    let detail = if row.detail.is_empty() || room <= GAP.width() + 1 {
        String::new()
    } else {
        clip(&format!("{GAP}{}", row.detail), room.saturating_sub(1))
    };
    room = room.saturating_sub(detail.width());
    let line = Line::from(vec![
        Span::styled(marker, if on { name } else { base }),
        Span::styled(label, name),
        Span::styled(detail, dim),
        Span::styled(" ".repeat(room), base),
        Span::styled(tag, dim),
    ]);
    (line, base)
}

fn pad(text: &str, width: usize) -> String {
    let gap = width.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(gap))
}

/// `text` cut to `width` columns, with `…` marking the cut.
pub(crate) fn clip(text: &str, width: usize) -> String {
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
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    fn theme() -> UiStyles {
        *crate::app::App::new(crate::buffer::Buffer::from_text(""), None).styles()
    }

    fn rows(count: usize) -> Vec<Row> {
        (0..count)
            .map(|index| Row::new(format!("item {index}")))
            .collect()
    }

    fn draw(width: u16, height: u16, area: Rect, list: &List) -> Buffer {
        let theme = theme();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, area, list, &theme))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn line(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn a_list_is_a_rounded_panel_with_the_keys_in_the_bottom_border() {
        let items = rows(3);
        let list = List::new(" Provider ", "↑↓ enter esc", &items).selected(Some(1));
        let buffer = draw(30, 6, Rect::new(0, 0, 30, 5), &list);
        assert_eq!(buffer[(0, 0)].symbol(), "╭");
        assert_eq!(buffer[(29, 0)].symbol(), "╮");
        assert_eq!(buffer[(0, 4)].symbol(), "╰");
        assert_eq!(buffer[(29, 4)].symbol(), "╯");
        assert!(line(&buffer, 0).contains("Provider"));
        assert!(
            line(&buffer, 4).contains("↑↓ enter esc"),
            "{}",
            line(&buffer, 4)
        );
        assert!(
            line(&buffer, 2).contains("› item 1"),
            "{}",
            line(&buffer, 2)
        );
    }

    #[test]
    fn the_highlighted_row_is_a_full_width_bar() {
        let theme = theme();
        let bar = theme.selection.bg.unwrap();
        let items = rows(3);
        let list = List::new(" x ", "", &items).selected(Some(1));
        let buffer = draw(30, 6, Rect::new(0, 0, 30, 5), &list);
        for x in 1..29 {
            assert_eq!(buffer[(x, 2)].bg, bar, "column {x} of the highlighted row");
            assert_ne!(buffer[(x, 1)].bg, bar, "column {x} of a plain row");
        }
        assert_ne!(bar, theme.editor_bg);
    }

    #[test]
    fn tags_sit_at_the_right_edge() {
        let items = vec![
            Row::new("anthropic/claude").tag("recent"),
            Row::new("openai/gpt-4o").tag("server"),
        ];
        let list = List::new(" Model ", "", &items).selected(Some(0));
        let buffer = draw(40, 5, Rect::new(0, 0, 40, 4), &list);
        assert!(
            line(&buffer, 1).ends_with("recent │"),
            "{}",
            line(&buffer, 1)
        );
        assert!(
            line(&buffer, 2).ends_with("server │"),
            "{}",
            line(&buffer, 2)
        );
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_highlight_and_shows_a_counter() {
        let items = rows(20);
        let list = List::new(" x ", "", &items).selected(Some(15));
        let buffer = draw(30, 7, Rect::new(0, 0, 30, 6), &list);
        let screen: String = (0..6).map(|y| line(&buffer, y) + "\n").collect();
        assert!(screen.contains("› item 15"), "{screen}");
        assert!(line(&buffer, 0).contains("16/20"), "{screen}");
        assert!(!screen.contains("item 0 "), "{screen}");
    }

    #[test]
    fn tiny_areas_never_panic_and_fall_back_to_plain_rows() {
        let items = rows(5);
        for width in 0..10u16 {
            for height in 0..6u16 {
                let list = List::new(" A long title ", "↑↓ enter esc", &items)
                    .selected(Some(4))
                    .note("docs");
                let _ = draw(10, 6, Rect::new(0, 0, width, height), &list);
                // Larger than the screen is clipped, not a panic.
                let _ = draw(4, 2, Rect::new(2, 1, width, height), &list);
            }
        }
        let items = rows(3);
        let list = List::new(" x ", "", &items).selected(Some(2));
        let buffer = draw(20, 2, Rect::new(0, 0, 20, 2), &list);
        assert!(line(&buffer, 0).contains("item 1") || line(&buffer, 1).contains("› item 2"));
        let empty: Vec<Row> = Vec::new();
        let list = List::new(" x ", "", &empty).empty("No matching command");
        let buffer = draw(30, 3, Rect::new(0, 0, 30, 3), &list);
        assert!(line(&buffer, 1).contains("No matching command"));
    }

    #[test]
    fn anchored_prefers_below_and_flips_above_when_there_is_more_room() {
        let bounds = Rect::new(0, 0, 40, 20);
        assert_eq!(anchored(bounds, 5, 10, 2, 6), Some(Rect::new(5, 3, 10, 6)));
        assert_eq!(
            anchored(bounds, 5, 10, 17, 6),
            Some(Rect::new(5, 11, 10, 6))
        );
        assert_eq!(
            anchored(bounds, 35, 10, 2, 6),
            Some(Rect::new(30, 3, 10, 6))
        );
        assert_eq!(anchored(bounds, 0, 80, 2, 6).map(|r| r.width), Some(40));
        assert_eq!(anchored(Rect::new(0, 0, 40, 1), 0, 10, 0, 6), None);
        assert_eq!(anchored(Rect::default(), 0, 10, 0, 6), None);
    }

    #[test]
    fn window_start_keeps_the_selection_in_view() {
        assert_eq!(window_start(0, 2, 8), 0);
        assert_eq!(window_start(0, 16, 8), 0);
        assert_eq!(window_start(15, 16, 8), 8);
        assert_eq!(window_start(9, 16, 8), 5);
        assert_eq!(window_start(3, 0, 4), 0);
        assert_eq!(window_start(3, 10, 0), 0);
    }

    #[test]
    fn clip_marks_the_cut() {
        assert_eq!(clip("save", 4), "save");
        assert_eq!(clip("explorer", 4), "exp…");
        assert_eq!(clip("explorer", 1), "…");
        assert_eq!(clip("explorer", 0), "");
    }
}
