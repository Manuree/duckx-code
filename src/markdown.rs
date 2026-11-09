//! Read-only Markdown rendering for the `preview` pane.
//!
//! The source is parsed with `pulldown-cmark` and laid out into styled, pre-wrapped
//! terminal lines, so the preview can scroll by row and cache the result. Nothing
//! here touches the disk; images show as their alt text.
//!
//! The visual design of the preview (heading hierarchy, framed code blocks with a
//! language label, ☐/☑ task boxes, and the box-drawn table look) was inspired by
//! markdown-reader by leboiko (<https://github.com/leboiko/markdown-reader>, MIT
//! License). No code from markdown-reader is included; everything here, including the
//! table column-width allocator, was written for duckx-code's theme, wrapping, and
//! tree-sitter highlighter.
//!
//! Layout rules:
//! * Every width is display width (`unicode-width`) and text only breaks on grapheme
//!   boundaries, so Thai marks stay on their letter and emoji count as two columns.
//! * Words break at whitespace. A run longer than the line (a long Thai phrase, a
//!   URL) is split between graphemes.
//! * A list item's continuation lines line up under its text, not its bullet.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::theme::SyntaxRole;

/// Every highlight role, in the order `MarkdownStyles::syntax` stores their colors.
const ROLES: [SyntaxRole; 21] = [
    SyntaxRole::Text,
    SyntaxRole::Comment,
    SyntaxRole::Keyword,
    SyntaxRole::Function,
    SyntaxRole::Type,
    SyntaxRole::String,
    SyntaxRole::Number,
    SyntaxRole::Constant,
    SyntaxRole::Operator,
    SyntaxRole::Punctuation,
    SyntaxRole::Variable,
    SyntaxRole::Parameter,
    SyntaxRole::Property,
    SyntaxRole::Tag,
    SyntaxRole::Attribute,
    SyntaxRole::Error,
    SyntaxRole::Documentation,
    SyntaxRole::Macro,
    SyntaxRole::Builtin,
    SyntaxRole::Escape,
    SyntaxRole::Lifetime,
];

/// Code blocks larger than this are drawn without syntax colors.
const MAX_HIGHLIGHT_BYTES: usize = 64 * 1024;

/// Colors the preview uses. Built from the active color and syntax themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkdownStyles {
    pub text: Style,
    pub heading: [Style; 6],
    /// The rule drawn under an H1.
    pub heading_rule: Style,
    /// Inline `code`.
    pub code: Style,
    /// Plain text inside a code block (on `code_bg`).
    pub code_block: Style,
    pub code_bg: Color,
    pub code_border: Style,
    pub code_label: Style,
    pub quote: Style,
    pub quote_bar: Style,
    pub link: Style,
    /// Dim text: image labels, raw HTML, a link with no text.
    pub url: Style,
    pub marker: Style,
    pub task_done: Style,
    pub rule: Style,
    pub table_border: Style,
    pub table_header: Style,
    /// Syntax colors for code blocks, indexed like `ROLES`.
    pub syntax: [Color; 21],
}

impl MarkdownStyles {
    pub fn from_theme(ui: &crate::theme::UiStyles, syntax: &crate::theme::SyntaxTheme) -> Self {
        let base = Style::new().fg(ui.editor_fg).bg(ui.editor_bg);
        let color = |role| syntax.color(role);
        let bold = Modifier::BOLD;
        let code_bg = ui.status_c_bg;
        let dim = color(SyntaxRole::Comment);
        Self {
            text: base,
            heading: [
                base.fg(color(SyntaxRole::Keyword)).add_modifier(bold),
                base.fg(color(SyntaxRole::Function)).add_modifier(bold),
                base.fg(color(SyntaxRole::Type)).add_modifier(bold),
                base.fg(color(SyntaxRole::Constant)).add_modifier(bold),
                base.fg(color(SyntaxRole::Property)).add_modifier(bold),
                base.fg(dim).add_modifier(bold),
            ],
            heading_rule: base.fg(color(SyntaxRole::Keyword)),
            code: Style::new().fg(color(SyntaxRole::String)).bg(code_bg),
            code_block: Style::new().fg(ui.editor_fg).bg(code_bg),
            code_bg,
            code_border: base.fg(dim),
            code_label: base.fg(color(SyntaxRole::Type)).add_modifier(bold),
            quote: base.fg(dim),
            quote_bar: base.fg(color(SyntaxRole::Function)),
            link: base
                .fg(color(SyntaxRole::Function))
                .add_modifier(Modifier::UNDERLINED),
            url: base.fg(dim),
            marker: base.fg(color(SyntaxRole::Operator)),
            task_done: base.fg(color(SyntaxRole::String)),
            rule: base.fg(dim),
            table_border: base.fg(dim),
            table_header: base.fg(color(SyntaxRole::Type)).add_modifier(bold),
            syntax: ROLES.map(color),
        }
    }

    fn role(&self, role: SyntaxRole) -> Style {
        let index = ROLES.iter().position(|r| *r == role).unwrap_or(0);
        self.code_block.fg(self.syntax[index])
    }
}

impl Default for MarkdownStyles {
    fn default() -> Self {
        let appearance = crate::theme::Appearance::builtin();
        Self::from_theme(&appearance.styles, &appearance.syntax)
    }
}

/// Renders `source` into lines no wider than `width` columns.
pub fn render(source: &str, width: u16, styles: &MarkdownStyles) -> Vec<Line<'static>> {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES;
    let events: Vec<Event<'_>> = Parser::new_ext(source, options).collect();
    let mut renderer = Renderer {
        width: usize::from(width.max(1)),
        styles: *styles,
        lines: Vec::new(),
        pending: Vec::new(),
        containers: Vec::new(),
        inline: Vec::new(),
        heading: None,
        code: None,
        table: None,
        links: Vec::new(),
        image: 0,
        gap: false,
    };
    for (index, event) in events.iter().enumerate() {
        renderer.event(event, &events[index + 1..]);
    }
    renderer.flush();
    while renderer.lines.last().is_some_and(|line| line.width() == 0) {
        renderer.lines.pop();
    }
    renderer.lines
}

enum Container {
    Quote,
    List {
        next: Option<u64>,
        /// Widest number in the list, so `9.` and `10.` line up.
        digits: usize,
        /// Items hold paragraphs, so a blank line goes between them.
        loose: bool,
        items: usize,
    },
    Item {
        marker: String,
        style: Style,
        width: usize,
        used: bool,
    },
}

#[derive(Clone, Copy)]
enum Inline {
    Bold,
    Italic,
    Strike,
    Link,
}

type Piece = (String, Style);

struct Table {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<Vec<Piece>>>,
    head: bool,
    header_rows: usize,
}

struct Renderer {
    width: usize,
    styles: MarkdownStyles,
    lines: Vec<Line<'static>>,
    /// Inline text of the block being collected. `\n` forces a line break.
    pending: Vec<Piece>,
    containers: Vec<Container>,
    inline: Vec<Inline>,
    heading: Option<usize>,
    /// Language and text of the open code block.
    code: Option<(String, String)>,
    table: Option<Table>,
    /// Open links: URL and where their text starts in `pending`.
    links: Vec<(String, usize)>,
    image: usize,
    /// A blank line goes before the next block.
    gap: bool,
}

impl Renderer {
    fn event(&mut self, event: &Event<'_>, rest: &[Event<'_>]) {
        match event {
            Event::Start(tag) => self.start(tag, rest),
            Event::End(tag) => self.end(*tag),
            Event::Text(text) => {
                if let Some((_, code)) = self.code.as_mut() {
                    code.push_str(text);
                } else {
                    let style = self.inline_style();
                    self.push(text, style);
                }
            }
            Event::Code(text) => {
                let style = self.with_inline(self.styles.code);
                self.push(text, style);
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                let style = self.styles.code;
                self.push(text, style);
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                let style = self.styles.url;
                self.push(text.trim_end_matches('\n'), style);
            }
            Event::SoftBreak => {
                let style = self.inline_style();
                self.push(" ", style);
            }
            Event::HardBreak => self.push("\n", self.styles.text),
            Event::Rule => {
                self.flush();
                self.begin_block();
                let room = self.room();
                let mut spans = self.prefix(true);
                spans.push(Span::styled("─".repeat(room), self.styles.rule));
                self.lines.push(Line::from(spans));
                self.gap = true;
            }
            Event::TaskListMarker(done) => self.task_marker(*done),
            Event::FootnoteReference(name) => {
                let style = self.styles.url;
                self.push(&format!("[^{name}]"), style);
            }
        }
    }

    fn start(&mut self, tag: &Tag<'_>, rest: &[Event<'_>]) {
        match tag {
            Tag::Paragraph => {
                if self.table.is_none() {
                    self.begin_block();
                }
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.gap = !self.lines.is_empty();
                self.begin_block();
                self.heading = Some(heading_index(*level));
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.begin_block();
                self.containers.push(Container::Quote);
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.begin_block();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                let nested = self
                    .containers
                    .iter()
                    .any(|container| matches!(container, Container::Item { .. }));
                if nested {
                    self.gap = false;
                } else {
                    self.begin_block();
                }
                let (count, loose) = scan_list(rest);
                let last = start.map(|first| first + count.saturating_sub(1) as u64);
                self.containers.push(Container::List {
                    next: *start,
                    digits: last.map_or(0, |n| n.to_string().len()),
                    loose,
                    items: 0,
                });
            }
            Tag::Item => {
                self.flush();
                let depth = self
                    .containers
                    .iter()
                    .filter(|container| matches!(container, Container::List { .. }))
                    .count();
                let mut gap = false;
                let marker = match self.containers.iter_mut().rev().find_map(|c| match c {
                    Container::List {
                        next,
                        digits,
                        loose,
                        items,
                    } => Some((next, *digits, *loose, items)),
                    _ => None,
                }) {
                    Some((next, digits, loose, items)) => {
                        gap = loose && *items > 0;
                        *items += 1;
                        match next {
                            Some(number) => {
                                let marker = format!("{number:>digits$}. ");
                                *number += 1;
                                marker
                            }
                            None => format!("{} ", bullet(depth)),
                        }
                    }
                    None => "• ".to_string(),
                };
                self.gap = gap;
                if gap {
                    self.begin_block();
                }
                self.containers.push(Container::Item {
                    width: marker.width(),
                    marker,
                    style: self.styles.marker,
                    used: false,
                });
            }
            Tag::Emphasis => self.inline.push(Inline::Italic),
            Tag::Strong => self.inline.push(Inline::Bold),
            Tag::Strikethrough => self.inline.push(Inline::Strike),
            Tag::Link { dest_url, .. } => {
                self.inline.push(Inline::Link);
                self.links.push((dest_url.to_string(), self.pending.len()));
            }
            Tag::Image { .. } => {
                self.image += 1;
                let style = self.styles.url;
                self.push("[image: ", style);
            }
            Tag::Table(alignments) => {
                self.flush();
                self.begin_block();
                self.table = Some(Table {
                    alignments: alignments.clone(),
                    rows: Vec::new(),
                    head: false,
                    header_rows: 0,
                });
            }
            Tag::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.head = true;
                    table.rows.push(Vec::new());
                }
            }
            Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.rows.push(Vec::new());
                }
            }
            Tag::TableCell => self.pending.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if self.table.is_none() {
                    self.flush();
                }
            }
            TagEnd::Heading(_) => {
                let level = self.heading.unwrap_or(0);
                self.flush();
                if level == 0 {
                    let room = self.room();
                    let mut spans = self.prefix(false);
                    spans.push(Span::styled("━".repeat(room), self.styles.heading_rule));
                    self.lines.push(Line::from(spans));
                }
                self.heading = None;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
                self.gap = true;
            }
            TagEnd::CodeBlock => {
                let (lang, code) = self.code.take().unwrap_or_default();
                self.code_block(&lang, &code);
                self.gap = true;
            }
            TagEnd::List(_) => {
                self.flush();
                self.containers.pop();
                self.gap = true;
            }
            TagEnd::Item => {
                self.flush();
                self.containers.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.inline.pop();
            }
            TagEnd::Link => {
                self.inline.pop();
                let (url, from) = self.links.pop().unwrap_or_default();
                let empty = self
                    .pending
                    .get(from..)
                    .unwrap_or_default()
                    .iter()
                    .all(|(text, _)| text.trim().is_empty());
                if empty && !url.is_empty() {
                    let style = self.styles.link;
                    self.push(&url, style);
                }
            }
            TagEnd::Image => {
                self.image = self.image.saturating_sub(1);
                let style = self.styles.url;
                self.push("]", style);
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.pending);
                if let Some(row) = self.table.as_mut().and_then(|table| table.rows.last_mut()) {
                    row.push(cell);
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.head = false;
                    table.header_rows = table.rows.len();
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.table_block(table);
                }
                self.gap = true;
            }
            _ => {}
        }
    }

    fn task_marker(&mut self, done: bool) {
        let (mark, style) = if done {
            ("☑", self.styles.task_done)
        } else {
            ("☐", self.styles.marker)
        };
        let item = self
            .containers
            .iter_mut()
            .rev()
            .find(|container| matches!(container, Container::Item { .. }));
        if let Some(Container::Item {
            marker,
            style: marker_style,
            width,
            used: false,
        }) = item
            && !marker
                .trim_start()
                .starts_with(|c: char| c.is_ascii_digit())
        {
            // A bulleted task: the box takes the bullet's place.
            *marker = format!("{mark} ");
            *marker_style = style;
            *width = marker.width();
            return;
        }
        self.push(&format!("{mark} "), style);
    }

    /// Blank line before a block, carrying the quote bars it sits in.
    fn begin_block(&mut self) {
        if self.gap && !self.lines.is_empty() {
            let spans = self.bars();
            self.lines.push(Line::from(spans));
        }
        self.gap = false;
    }

    fn with_inline(&self, mut style: Style) -> Style {
        for inline in &self.inline {
            style = match inline {
                Inline::Bold => style.add_modifier(Modifier::BOLD),
                Inline::Italic => style.add_modifier(Modifier::ITALIC),
                Inline::Strike => style.add_modifier(Modifier::CROSSED_OUT),
                Inline::Link => style
                    .fg(self.styles.link.fg.unwrap_or(Color::Reset))
                    .add_modifier(Modifier::UNDERLINED),
            };
        }
        style
    }

    fn inline_style(&self) -> Style {
        if self.image > 0 {
            return self.styles.url;
        }
        let base = match self.heading {
            Some(level) => self.styles.heading[level],
            None if self.table.as_ref().is_some_and(|table| table.head) => self.styles.table_header,
            None if self.in_quote() => self.styles.quote,
            None => self.styles.text,
        };
        self.with_inline(base)
    }

    fn in_quote(&self) -> bool {
        self.containers
            .iter()
            .any(|container| matches!(container, Container::Quote))
    }

    fn push(&mut self, text: &str, style: Style) {
        if !text.is_empty() {
            self.pending.push((text.to_string(), style));
        }
    }

    fn prefix_width(&self) -> usize {
        self.containers
            .iter()
            .map(|container| match container {
                Container::Quote => 2,
                Container::List { .. } => 0,
                Container::Item { width, .. } => *width,
            })
            .sum()
    }

    /// Columns left for content after the quote bars and list indents.
    fn room(&self) -> usize {
        self.width.saturating_sub(self.prefix_width()).max(1)
    }

    /// The prefix of a blank line: only the quote bars (and the indent before them).
    fn bars(&self) -> Vec<Span<'static>> {
        let last = self
            .containers
            .iter()
            .rposition(|container| matches!(container, Container::Quote));
        let Some(last) = last else {
            return Vec::new();
        };
        let mut spans = Vec::new();
        for container in &self.containers[..=last] {
            match container {
                Container::Quote => spans.push(Span::styled("│ ", self.styles.quote_bar)),
                Container::Item { width, .. } => {
                    spans.push(Span::styled(" ".repeat(*width), self.styles.text))
                }
                Container::List { .. } => {}
            }
        }
        spans
    }

    /// Prefix for one output line. `first` shows the innermost unused list marker.
    fn prefix(&mut self, first: bool) -> Vec<Span<'static>> {
        let innermost = self
            .containers
            .iter()
            .rposition(|container| matches!(container, Container::Item { .. }));
        let quote_bar = self.styles.quote_bar;
        let text = self.styles.text;
        let mut spans = Vec::new();
        for (index, container) in self.containers.iter_mut().enumerate() {
            match container {
                Container::Quote => spans.push(Span::styled("│ ", quote_bar)),
                Container::List { .. } => {}
                Container::Item {
                    marker,
                    style,
                    width,
                    used,
                } => {
                    if first && !*used && Some(index) == innermost {
                        spans.push(Span::styled(marker.clone(), *style));
                        *used = true;
                    } else {
                        spans.push(Span::styled(" ".repeat(*width), text));
                    }
                }
            }
        }
        spans
    }

    /// Wraps the collected inline text into lines under the current prefix.
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        let room = self.room();
        let mut first = true;
        for row in wrap(&pending, room) {
            let mut spans = self.prefix(first);
            first = false;
            spans.extend(row);
            self.lines.push(Line::from(spans));
        }
        self.gap = true;
    }

    /// A rounded frame with the language in its top edge. Lines that do not fit wrap
    /// at spaces inside the frame; nothing is cut off.
    fn code_block(&mut self, lang: &str, code: &str) {
        let code = code.strip_suffix('\n').unwrap_or(code);
        let room = self.room();
        let rows = highlight(lang, code, &self.styles);
        if room < 8 {
            for row in rows {
                for chunk in wrap_code(&row, room) {
                    let mut spans = self.prefix(false);
                    spans.extend(chunk);
                    self.lines.push(Line::from(spans));
                }
            }
            return;
        }
        let border = self.styles.code_border;
        let inner = room - 4;
        let label = lang.trim();
        let top = if label.is_empty() || label.width() + 5 > room {
            vec![Span::styled(format!("╭{}╮", "─".repeat(room - 2)), border)]
        } else {
            vec![
                Span::styled("╭─ ", border),
                Span::styled(label.to_string(), self.styles.code_label),
                Span::styled(
                    format!(" {}╮", "─".repeat(room.saturating_sub(label.width() + 5))),
                    border,
                ),
            ]
        };
        let mut spans = self.prefix(false);
        spans.extend(top);
        self.lines.push(Line::from(spans));
        let fill = self.styles.code_block;
        for row in rows {
            for chunk in wrap_code(&row, inner) {
                let used: usize = chunk.iter().map(|span| span.width()).sum();
                let mut spans = self.prefix(false);
                spans.push(Span::styled("│", border));
                spans.push(Span::styled(" ", fill));
                spans.extend(chunk);
                spans.push(Span::styled(" ".repeat(inner - used.min(inner) + 1), fill));
                spans.push(Span::styled("│", border));
                self.lines.push(Line::from(spans));
            }
        }
        let mut spans = self.prefix(false);
        spans.push(Span::styled(format!("╰{}╯", "─".repeat(room - 2)), border));
        self.lines.push(Line::from(spans));
    }

    fn table_block(&mut self, table: Table) {
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }
        let room = self.room();
        let border = self.styles.table_border;
        // Each column has a space of padding on both sides and a bar after it.
        let target = room.saturating_sub(columns * 3 + 1);
        if target < columns * MIN_COLUMN {
            // Too narrow for a grid: one wrapped line per row, cells split by `│`.
            for row in &table.rows {
                let mut pieces: Vec<Piece> = Vec::new();
                for (index, cell) in row.iter().enumerate() {
                    if index > 0 {
                        pieces.push((" │ ".to_string(), border));
                    }
                    pieces.extend(cell.iter().cloned());
                }
                let mut first = true;
                for line in wrap(&pieces, room) {
                    let mut spans = self.prefix(first);
                    first = false;
                    spans.extend(line);
                    self.lines.push(Line::from(spans));
                }
            }
            return;
        }
        let natural: Vec<usize> = (0..columns)
            .map(|column| {
                table
                    .rows
                    .iter()
                    .filter_map(|row| row.get(column))
                    .map(|cell| cell.iter().map(|(text, _)| text.width()).sum::<usize>())
                    .max()
                    .unwrap_or(1)
                    .max(1)
            })
            .collect();
        let widths = fit_column_widths(&natural, target);
        let edge = |left: &str, mid: &str, right: &str| -> Span<'static> {
            let parts: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
            Span::styled(format!("{left}{}{right}", parts.join(mid)), border)
        };
        let top = edge("╭", "┬", "╮");
        let mut spans = self.prefix(false);
        spans.push(top);
        self.lines.push(Line::from(spans));
        for (index, row) in table.rows.iter().enumerate() {
            if index > 0 && index == table.header_rows {
                let mut spans = self.prefix(false);
                spans.push(edge("├", "┼", "┤"));
                self.lines.push(Line::from(spans));
            }
            let cells: Vec<Vec<Vec<Span<'static>>>> = (0..columns)
                .map(|column| {
                    let cell = row.get(column).map(Vec::as_slice).unwrap_or_default();
                    wrap(cell, widths[column])
                })
                .collect();
            let height = cells.iter().map(Vec::len).max().unwrap_or(1);
            for sub in 0..height {
                let mut spans = self.prefix(false);
                spans.push(Span::styled("│", border));
                for (column, lines) in cells.iter().enumerate() {
                    let content = lines.get(sub).cloned().unwrap_or_default();
                    let used: usize = content.iter().map(|span| span.width()).sum();
                    let free = widths[column].saturating_sub(used);
                    let (before, after) = match table.alignments.get(column) {
                        Some(Alignment::Right) => (free, 0),
                        Some(Alignment::Center) => (free / 2, free - free / 2),
                        _ => (0, free),
                    };
                    spans.push(Span::styled(" ".repeat(before + 1), self.styles.text));
                    spans.extend(content);
                    spans.push(Span::styled(" ".repeat(after + 1), self.styles.text));
                    spans.push(Span::styled("│", border));
                }
                self.lines.push(Line::from(spans));
            }
        }
        let mut spans = self.prefix(false);
        spans.push(edge("╰", "┴", "╯"));
        self.lines.push(Line::from(spans));
    }
}

/// Items in the list that starts at the beginning of `rest`, and whether any of them
/// holds a paragraph (a loose list, drawn with blank lines between items).
fn scan_list(rest: &[Event<'_>]) -> (usize, bool) {
    let mut depth = 0usize;
    let mut items = 0;
    let mut loose = false;
    let mut item_open = false;
    for event in rest {
        match event {
            Event::Start(Tag::List(_)) => depth += 1,
            Event::End(TagEnd::List(_)) => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            Event::Start(Tag::Item) if depth == 0 => {
                items += 1;
                item_open = true;
            }
            Event::End(TagEnd::Item) if depth == 0 => item_open = false,
            Event::Start(Tag::Paragraph) if depth == 0 && item_open => loose = true,
            _ => {}
        }
    }
    (items, loose)
}

fn bullet(depth: usize) -> &'static str {
    match depth {
        0 | 1 => "•",
        2 => "◦",
        _ => "▪",
    }
}

fn heading_index(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 0,
        HeadingLevel::H2 => 1,
        HeadingLevel::H3 => 2,
        HeadingLevel::H4 => 3,
        HeadingLevel::H5 => 4,
        HeadingLevel::H6 => 5,
    }
}

/// Narrowest width any table column is squeezed to: room for one double-width
/// grapheme (a Thai cluster with its marks, an emoji). `table_block` already falls
/// back to plain `│`-separated rows when it cannot give every column this much.
const MIN_COLUMN: usize = 2;

/// Fits a table's columns into `budget` display columns.
///
/// `wanted` holds each column's natural width (its widest cell). When they all fit
/// they are returned unchanged. Otherwise the widest columns give way first: one
/// shared ceiling is lowered until every column, cut down to it, fits, and columns
/// already narrower than the ceiling keep their natural width. The few columns left
/// below the next ceiling step go, one each, to the columns cut the most (leftmost
/// first on a tie), so the whole budget is used.
///
/// Every column therefore keeps at least `min(wanted, budget / columns)`, the most an
/// even split could promise it, and a wider budget never makes a column narrower.
fn fit_column_widths(wanted: &[usize], budget: usize) -> Vec<usize> {
    let total: usize = wanted.iter().sum();
    if total <= budget {
        return wanted.to_vec();
    }
    // Settle columns narrowest first. Once the narrowest unsettled column cannot have
    // its full width out of an equal slice of what is left, every unsettled column is
    // capped at that slice.
    let mut ascending = wanted.to_vec();
    ascending.sort_unstable();
    let mut remaining = budget;
    let mut ceiling = 0;
    for (settled, &width) in ascending.iter().enumerate() {
        let unsettled = ascending.len() - settled;
        if width.saturating_mul(unsettled) > remaining {
            ceiling = remaining / unsettled;
            break;
        }
        remaining -= width;
    }
    let mut widths: Vec<usize> = wanted.iter().map(|&width| width.min(ceiling)).collect();
    let mut spare = budget.saturating_sub(widths.iter().sum());
    // Hand out the leftover columns to the biggest shortfalls.
    let mut cut: Vec<usize> = (0..wanted.len())
        .filter(|&column| wanted[column] > widths[column])
        .collect();
    cut.sort_by_key(|&column| std::cmp::Reverse(wanted[column] - widths[column]));
    for column in cut {
        if spare == 0 {
            break;
        }
        widths[column] += 1;
        spare -= 1;
    }
    widths
}

/// One styled grapheme.
struct Unit {
    text: String,
    style: Style,
    width: usize,
    space: bool,
}

fn units(pieces: &[Piece]) -> Vec<Unit> {
    let mut out = Vec::new();
    for (text, style) in pieces {
        for grapheme in text.graphemes(true) {
            let grapheme = if grapheme == "\t" { " " } else { grapheme };
            out.push(Unit {
                text: grapheme.to_string(),
                style: *style,
                width: grapheme.width(),
                space: grapheme.chars().all(|c| c == ' ' || c == '\u{a0}'),
            });
        }
    }
    out
}

/// Joins neighboring graphemes of one style into spans.
fn spans_of(units: &[&Unit]) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut text = String::new();
    let mut style = None;
    for unit in units {
        if style != Some(unit.style) && !text.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut text),
                style.unwrap_or_default(),
            ));
        }
        style = Some(unit.style);
        text.push_str(&unit.text);
    }
    if !text.is_empty() {
        spans.push(Span::styled(text, style.unwrap_or_default()));
    }
    spans
}

/// Greedy word wrap by display width. Spaces at a break are dropped; a word wider
/// than the line splits between graphemes. `\n` starts a new row.
fn wrap(pieces: &[Piece], width: usize) -> Vec<Vec<Span<'static>>> {
    wrap_units(&units(pieces), width.max(1), false)
}

/// Wrap for code: leading indentation is kept on the first row of each source line.
fn wrap_code(pieces: &[Piece], width: usize) -> Vec<Vec<Span<'static>>> {
    wrap_units(&units(pieces), width.max(1), true)
}

fn wrap_units(units: &[Unit], width: usize, keep_indent: bool) -> Vec<Vec<Span<'static>>> {
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut row: Vec<&Unit> = Vec::new();
    let mut used = 0usize;
    let mut first_row = true;
    let finish = |rows: &mut Vec<Vec<Span<'static>>>, row: &mut Vec<&Unit>| {
        while row.last().is_some_and(|unit| unit.space) {
            row.pop();
        }
        rows.push(spans_of(row));
        row.clear();
    };
    let mut index = 0;
    while index < units.len() {
        let unit = &units[index];
        if unit.text == "\n" || unit.text == "\r\n" {
            finish(&mut rows, &mut row);
            used = 0;
            first_row = true;
            index += 1;
            continue;
        }
        // The next token: a run of spaces or a run of non-spaces.
        let space = unit.space;
        let end = units[index..]
            .iter()
            .position(|u| u.space != space || u.text == "\n" || u.text == "\r\n")
            .map_or(units.len(), |offset| index + offset);
        let token = &units[index..end];
        let token_width: usize = token.iter().map(|u| u.width).sum();
        index = end;
        if space {
            if used == 0 && !(keep_indent && first_row) {
                continue;
            }
            if used + token_width > width {
                finish(&mut rows, &mut row);
                used = 0;
                first_row = false;
                continue;
            }
            used += token_width;
            row.extend(token.iter());
            continue;
        }
        if used + token_width > width && used > 0 && token_width <= width {
            finish(&mut rows, &mut row);
            used = 0;
            first_row = false;
        }
        for unit in token {
            if used + unit.width > width && used > 0 {
                finish(&mut rows, &mut row);
                used = 0;
                first_row = false;
            }
            used += unit.width;
            row.push(unit);
        }
    }
    if !row.is_empty() || rows.is_empty() {
        finish(&mut rows, &mut row);
    }
    rows
}

/// One list of styled pieces per source line of a code block. Languages the editor
/// highlights get their colors from the same tree-sitter queries; others stay plain.
fn highlight(lang: &str, code: &str, styles: &MarkdownStyles) -> Vec<Vec<Piece>> {
    let plain = styles.code_block;
    let mut roles: Vec<Option<SyntaxRole>> = vec![None; code.len()];
    if let Some(language) = crate::syntax::Language::from_info(lang)
        && code.len() <= MAX_HIGHLIGHT_BYTES
    {
        let rope = ropey::Rope::from_str(code);
        if let Some(parsed) = crate::syntax::parse(language, &rope, None) {
            let mut state = crate::syntax::SyntaxState::fresh(language, 0, 0);
            state.install(parsed);
            for (start, end, role) in
                crate::syntax::highlight_range(&state, &rope, 0, rope.len_lines())
            {
                for slot in roles.iter_mut().take(end.min(code.len())).skip(start) {
                    *slot = Some(role);
                }
            }
        }
    }
    let mut lines = Vec::new();
    let mut offset = 0;
    for line in code.split('\n') {
        let mut pieces: Vec<Piece> = Vec::new();
        for (at, ch) in line.char_indices() {
            let style = roles[offset + at].map_or(plain, |role| styles.role(role));
            let text = if ch == '\t' {
                "    ".to_string()
            } else {
                ch.to_string()
            };
            match pieces.last_mut() {
                Some((last, last_style)) if *last_style == style => last.push_str(&text),
                _ => pieces.push((text, style)),
            }
        }
        offset += line.len() + 1;
        lines.push(pieces);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    fn styles() -> MarkdownStyles {
        MarkdownStyles::default()
    }

    fn span<'a>(lines: &'a [Line<'_>], text: &str) -> &'a Span<'a> {
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content == text)
            .unwrap_or_else(|| panic!("no span {text:?}"))
    }

    #[test]
    fn headings_have_a_hierarchy_and_even_spacing() {
        let styles = styles();
        let lines = render(
            "# Title\n\nIntro text.\n## Section\nBody.\n### Sub\n#### Four\n",
            20,
            &styles,
        );
        assert_eq!(
            plain(&lines),
            [
                "Title",
                "━━━━━━━━━━━━━━━━━━━━",
                "",
                "Intro text.",
                "",
                "Section",
                "",
                "Body.",
                "",
                "Sub",
                "",
                "Four",
            ]
        );
        assert_eq!(span(&lines, "Title").style, styles.heading[0]);
        assert!(styles.heading[0].add_modifier.contains(Modifier::BOLD));
        assert_eq!(span(&lines, "Section").style, styles.heading[1]);
        assert_ne!(styles.heading[0].fg, styles.heading[1].fg);
        assert_eq!(span(&lines, "Sub").style, styles.heading[2]);
    }

    #[test]
    fn inline_styles_render_without_markup() {
        let styles = styles();
        let lines = render(
            "Some **bold** and *italic* and ~~gone~~ with `code`.\n",
            60,
            &styles,
        );
        assert_eq!(plain(&lines), ["Some bold and italic and gone with code."]);
        assert!(
            span(&lines, "bold")
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            span(&lines, "italic")
                .style
                .add_modifier
                .contains(Modifier::ITALIC)
        );
        assert!(
            span(&lines, "gone")
                .style
                .add_modifier
                .contains(Modifier::CROSSED_OUT)
        );
        let code = span(&lines, "code");
        assert_eq!(code.style, styles.code);
        assert_ne!(
            code.style.bg, styles.text.bg,
            "inline code has its own background"
        );
    }

    #[test]
    fn links_are_underlined_and_hide_their_url() {
        let styles = styles();
        let lines = render(
            "See [the docs](https://example.com) and <https://a.io>.\n",
            60,
            &styles,
        );
        assert_eq!(plain(&lines), ["See the docs and https://a.io."]);
        let link = span(&lines, "the docs");
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(link.style.fg, styles.link.fg);
        let images = plain(&render("![logo](img/logo.png)\n", 60, &styles));
        assert_eq!(images, ["[image: logo]"]);
    }

    #[test]
    fn nested_lists_indent_and_continue_under_the_text() {
        let source = "- one\n- two words that wrap here\n  - inner\n    - deep\n- three\n";
        let lines = plain(&render(source, 16, &styles()));
        assert_eq!(
            lines,
            [
                "• one",
                "• two words that",
                "  wrap here",
                "  ◦ inner",
                "    ▪ deep",
                "• three",
            ]
        );
    }

    #[test]
    fn numbered_lists_align_and_tasks_show_boxes() {
        let source = "8. eight\n9. nine\n10. ten\n\n- [x] done\n- [ ] todo\n";
        let styles = styles();
        let lines = render(source, 30, &styles);
        assert_eq!(
            plain(&lines),
            [" 8. eight", " 9. nine", "10. ten", "", "☑ done", "☐ todo"]
        );
        assert_eq!(span(&lines, "☑ ").style, styles.task_done);
        assert_eq!(span(&lines, "☐ ").style, styles.marker);
    }

    #[test]
    fn loose_lists_keep_a_blank_line_between_items() {
        let lines = plain(&render("- one\n\n- two\n", 20, &styles()));
        assert_eq!(lines, ["• one", "", "• two"]);
    }

    #[test]
    fn quotes_get_a_bar_on_every_line() {
        let styles = styles();
        let lines = render("> first part of the quote\n>\n> second\n", 14, &styles);
        assert_eq!(
            plain(&lines),
            ["│ first part", "│ of the quote", "│ ", "│ second"]
        );
        assert_eq!(lines[0].spans[0].style, styles.quote_bar);
    }

    #[test]
    fn code_blocks_are_framed_labeled_and_highlighted() {
        let styles = styles();
        let lines = render(
            "```rust\nfn main() {\n    let x = 1;\n}\n```\n",
            24,
            &styles,
        );
        assert_eq!(
            plain(&lines),
            [
                "╭─ rust ───────────────╮",
                "│ fn main() {          │",
                "│     let x = 1;       │",
                "│ }                    │",
                "╰──────────────────────╯",
            ]
        );
        for line in &lines {
            assert_eq!(line.width(), 24);
        }
        let keyword = span(&lines, "fn");
        assert_ne!(
            keyword.style.fg, styles.code_block.fg,
            "keywords are colored"
        );
        assert_eq!(keyword.style.bg, Some(styles.code_bg));
        let unlabeled = plain(&render("```\nplain\n```\n", 12, &styles));
        assert_eq!(unlabeled[0], "╭──────────╮");
    }

    #[test]
    fn long_code_lines_wrap_at_spaces_inside_the_frame() {
        let lines = plain(&render("```\nalpha beta gamma\n```\n", 14, &styles()));
        assert_eq!(
            lines,
            [
                "╭────────────╮",
                "│ alpha beta │",
                "│ gamma      │",
                "╰────────────╯",
            ]
        );
    }

    #[test]
    fn tables_are_aligned_grids() {
        let source = "| Key | Action | N |\n| :-- | :--: | --: |\n| a | open file | 1 |\n| `Ctrl+W` | pane | 10 |\n";
        let styles = styles();
        let lines = render(source, 40, &styles);
        assert_eq!(
            plain(&lines),
            [
                "╭────────┬───────────┬────╮",
                "│ Key    │  Action   │  N │",
                "├────────┼───────────┼────┤",
                "│ a      │ open file │  1 │",
                "│ Ctrl+W │   pane    │ 10 │",
                "╰────────┴───────────┴────╯",
            ]
        );
        assert_eq!(span(&lines, "Key").style, styles.table_header);
    }

    #[test]
    fn narrow_tables_wrap_cells_and_keep_the_grid() {
        let source = "| a | b |\n| - | - |\n| one two three | x |\n";
        let lines = plain(&render(source, 16, &styles()));
        for line in &lines {
            assert!(line.width() <= 16, "{line}");
        }
        assert_eq!(lines[0].chars().next(), Some('╭'));
        assert!(lines.iter().any(|line| line.contains("one two")));
        assert!(lines.iter().any(|line| line.contains("three")));
        assert_eq!(lines.last().unwrap().chars().next(), Some('╰'));
    }

    #[test]
    fn rules_span_the_width() {
        let lines = plain(&render("a\n\n---\n\nb\n", 10, &styles()));
        assert_eq!(lines, ["a", "", "──────────", "", "b"]);
    }

    #[test]
    fn thai_and_emoji_wrap_by_display_width() {
        // Thai marks stay on their letter, and 🎨 counts as two columns.
        let source = "- 🎨 **ทุกอย่าง** ทำได้จากคีย์บอร์ด ไม่ต้องใช้เมาส์\n";
        let lines = plain(&render(source, 14, &styles()));
        for line in &lines {
            assert!(line.width() <= 14, "{line:?} is {} wide", line.width());
        }
        assert!(lines[0].starts_with("• 🎨 ทุกอย่าง"), "{lines:?}");
        for line in &lines[1..] {
            assert!(
                line.starts_with("  "),
                "continuation under the text: {line:?}"
            );
        }
        let joined: String = lines.iter().map(|line| line.trim_start()).collect();
        assert!(joined.contains("คีย์"), "no grapheme split: {lines:?}");
        for line in &lines {
            let first = line.trim_start().chars().next().unwrap();
            assert!(
                !('\u{0E31}'..='\u{0E3A}').contains(&first)
                    && !('\u{0E47}'..='\u{0E4E}').contains(&first),
                "a line starts with a combining mark: {line:?}"
            );
        }
    }

    #[test]
    fn long_words_split_between_graphemes() {
        let long = plain(&render("abcdefghijklmnop", 5, &styles()));
        assert_eq!(long, ["abcde", "fghij", "klmno", "p"]);
    }

    #[test]
    fn fit_column_widths_fits_the_target() {
        assert_eq!(fit_column_widths(&[3, 4], 20), [3, 4]);
        let widths = fit_column_widths(&[40, 10, 4], 30);
        assert_eq!(widths.iter().sum::<usize>(), 30);
        assert!(widths[0] > widths[1] && widths[2] == 4, "{widths:?}");
    }

    #[test]
    fn fit_column_widths_shrinks_the_widest_first() {
        assert_eq!(fit_column_widths(&[8, 35, 6], 48), [8, 34, 6]);
        assert_eq!(fit_column_widths(&[11, 17, 5], 24), [9, 10, 5]);
        // The leftover column goes to the biggest shortfall, leftmost on a tie.
        assert_eq!(fit_column_widths(&[10, 10, 10], 20), [7, 7, 6]);
    }

    #[test]
    fn fit_column_widths_handles_degenerate_input() {
        assert_eq!(fit_column_widths(&[], 0), Vec::<usize>::new());
        assert_eq!(fit_column_widths(&[], 10), Vec::<usize>::new());
        assert_eq!(fit_column_widths(&[5, 5], 0), [0, 0]);
        assert_eq!(fit_column_widths(&[5, 5], 1), [1, 0]);
        assert_eq!(fit_column_widths(&[1], 1), [1]);
        assert_eq!(fit_column_widths(&[usize::MAX / 2, 3], 9), [6, 3]);
    }

    /// Every natural-width list over `len` columns with widths in `1..=max`.
    fn width_lists(len: usize, max: usize) -> Vec<Vec<usize>> {
        let mut lists = vec![Vec::new()];
        for _ in 0..len {
            lists = lists
                .into_iter()
                .flat_map(|list: Vec<usize>| {
                    (1..=max).map(move |width| {
                        let mut next = list.clone();
                        next.push(width);
                        next
                    })
                })
                .collect();
        }
        lists
    }

    #[test]
    fn fit_column_widths_properties() {
        for len in 1..=4 {
            for wanted in width_lists(len, 9) {
                let total: usize = wanted.iter().sum();
                let mut previous = vec![0; len];
                for budget in 0..=total + 3 {
                    let widths = fit_column_widths(&wanted, budget);
                    let case = format!("{wanted:?} in {budget}: {widths:?}");
                    assert_eq!(widths.len(), len, "{case}");
                    let sum: usize = widths.iter().sum();
                    assert!(sum <= budget, "over budget: {case}");
                    assert_eq!(sum, budget.min(total), "budget left unused: {case}");
                    for (column, &width) in widths.iter().enumerate() {
                        assert!(width <= wanted[column], "wider than needed: {case}");
                        // Never below the even split (capped at the natural width) ...
                        assert!(width >= wanted[column].min(budget / len), "starved: {case}");
                        // ... so the table minimum holds whenever it is feasible.
                        if budget >= len * MIN_COLUMN {
                            assert!(width >= wanted[column].min(MIN_COLUMN), "under min: {case}");
                        }
                        assert!(width >= previous[column], "not monotonic: {case}");
                    }
                    previous = widths;
                }
            }
        }
    }

    #[test]
    fn tables_never_overflow_the_pane() {
        let sources = [
            TABLE_WIDE_FITS,
            TABLE_SLIGHTLY_NARROW,
            TABLE_VERY_NARROW_FALLBACK,
            TABLE_THAI_ENGLISH_EMOJI,
            TABLE_ALIGNMENT,
            TABLE_SINGLE_COLUMN,
            TABLE_MANY_COLUMNS,
            TABLE_EMPTY_CELLS,
            TABLE_LONG_CELL_BESIDE_SHORT,
        ];
        for source in sources {
            // Widths 0 and 1 only have to not panic: a two-column emoji cannot fit.
            render(source, 0, &styles());
            render(source, 1, &styles());
            for width in 2..=130u16 {
                for line in plain(&render(source, width, &styles())) {
                    assert!(line.width() <= usize::from(width), "{line:?} at {width}");
                }
            }
        }
    }

    /// Renders `source` at `width` and checks every line exactly, and that no line is
    /// wider than the pane.
    fn assert_table(source: &str, width: u16, expected: &[&str]) {
        let lines = plain(&render(source, width, &styles()));
        assert_eq!(lines, expected, "table at width {width}");
        for line in &lines {
            assert!(
                line.width() <= usize::from(width),
                "{line:?} overflows {width}"
            );
        }
    }

    const TABLE_WIDE_FITS: &str = "| Key | Action | N |\n| :-- | :--: | --: |\n| a | open file | 1 |\n| `Ctrl+W` | pane | 10 |\n";

    const TABLE_SLIGHTLY_NARROW: &str = "| Command | What it does | Keys |\n| --- | --- | --- |\n| new-file | Create a file in the focused folder | Ctrl+N |\n| rename | Rename the open file | F2 |\n";

    const TABLE_VERY_NARROW_FALLBACK: &str =
        "| a | b | c | d |\n| - | - | - | - |\n| one | two | three | four |\n";

    const TABLE_THAI_ENGLISH_EMOJI: &str = "| ชื่อ | Status | หมายเหตุ |\n| --- | :-: | --- |\n| แมว 🐱 | ok ✅ | น่ารักมาก very cute |\n| Dog 🐶 | ไม่ ❌ | ชอบวิ่งเล่น in the park |\n";

    const TABLE_ALIGNMENT: &str = "| Left | Centre | Right |\n| :--- | :----: | ----: |\n| a | b | c |\n| longer left | mid | 12345 |\n| x | wider centre text | 7 |\n";

    const TABLE_SINGLE_COLUMN: &str =
        "| Notes |\n| --- |\n| short |\n| a much longer note that has to wrap somewhere |\n";

    const TABLE_MANY_COLUMNS: &str = "| A | B | C | D | E | F | G | H |\n| - | - | - | - | - | - | - | - |\n| alpha | beta | gamma | delta | epsilon | zeta | eta | theta |\n";

    const TABLE_EMPTY_CELLS: &str = "| Name | Value | Comment |\n| --- | --- | --- |\n| a |  |  |\n|  | 2 |  |\n|  |  | only a comment here |\n";

    const TABLE_LONG_CELL_BESIDE_SHORT: &str = "| # | Description | OK |\n| - | --- | - |\n| 1 | This description is very long and will certainly not fit on a single line of the preview | y |\n| 2 | short | n |\n";

    #[test]
    fn table_snapshot_wide_fits() {
        assert_table(
            TABLE_WIDE_FITS,
            60,
            &[
                "╭────────┬───────────┬────╮",
                "│ Key    │  Action   │  N │",
                "├────────┼───────────┼────┤",
                "│ a      │ open file │  1 │",
                "│ Ctrl+W │   pane    │ 10 │",
                "╰────────┴───────────┴────╯",
            ],
        );
        assert_table(
            TABLE_WIDE_FITS,
            27,
            &[
                "╭────────┬───────────┬────╮",
                "│ Key    │  Action   │  N │",
                "├────────┼───────────┼────┤",
                "│ a      │ open file │  1 │",
                "│ Ctrl+W │   pane    │ 10 │",
                "╰────────┴───────────┴────╯",
            ],
        );
    }

    #[test]
    fn table_snapshot_slightly_narrow() {
        assert_table(
            TABLE_SLIGHTLY_NARROW,
            64,
            &[
                "╭──────────┬─────────────────────────────────────┬────────╮",
                "│ Command  │ What it does                        │ Keys   │",
                "├──────────┼─────────────────────────────────────┼────────┤",
                "│ new-file │ Create a file in the focused folder │ Ctrl+N │",
                "│ rename   │ Rename the open file                │ F2     │",
                "╰──────────┴─────────────────────────────────────┴────────╯",
            ],
        );
        assert_table(
            TABLE_SLIGHTLY_NARROW,
            58,
            &[
                "╭──────────┬────────────────────────────────────┬────────╮",
                "│ Command  │ What it does                       │ Keys   │",
                "├──────────┼────────────────────────────────────┼────────┤",
                "│ new-file │ Create a file in the focused       │ Ctrl+N │",
                "│          │ folder                             │        │",
                "│ rename   │ Rename the open file               │ F2     │",
                "╰──────────┴────────────────────────────────────┴────────╯",
            ],
        );
        assert_table(
            TABLE_SLIGHTLY_NARROW,
            50,
            &[
                "╭──────────┬────────────────────────────┬────────╮",
                "│ Command  │ What it does               │ Keys   │",
                "├──────────┼────────────────────────────┼────────┤",
                "│ new-file │ Create a file in the       │ Ctrl+N │",
                "│          │ focused folder             │        │",
                "│ rename   │ Rename the open file       │ F2     │",
                "╰──────────┴────────────────────────────┴────────╯",
            ],
        );
    }

    #[test]
    fn table_snapshot_very_narrow_fallback() {
        assert_table(
            TABLE_VERY_NARROW_FALLBACK,
            20,
            &["a │ b │ c │ d", "one │ two │ three │", "four"],
        );
        assert_table(
            TABLE_VERY_NARROW_FALLBACK,
            16,
            &["a │ b │ c │ d", "one │ two │", "three │ four"],
        );
        assert_table(
            TABLE_VERY_NARROW_FALLBACK,
            12,
            &["a │ b │ c │", "d", "one │ two │", "three │ four"],
        );
        assert_table(
            TABLE_VERY_NARROW_FALLBACK,
            6,
            &["a │ b", "│ c │", "d", "one │", "two │", "three", "│ four"],
        );
    }

    #[test]
    fn table_snapshot_thai_english_emoji() {
        assert_table(
            TABLE_THAI_ENGLISH_EMOJI,
            60,
            &[
                "╭────────┬────────┬──────────────────────╮",
                "│ ชื่อ     │ Status │ หมายเหตุ              │",
                "├────────┼────────┼──────────────────────┤",
                "│ แมว 🐱 │ ok ✅  │ น่ารักมาก very cute    │",
                "│ Dog 🐶 │ ไม่ ❌  │ ชอบวิ่งเล่น in the park │",
                "╰────────┴────────┴──────────────────────╯",
            ],
        );
        assert_table(
            TABLE_THAI_ENGLISH_EMOJI,
            36,
            &[
                "╭────────┬────────┬────────────────╮",
                "│ ชื่อ     │ Status │ หมายเหตุ        │",
                "├────────┼────────┼────────────────┤",
                "│ แมว 🐱 │ ok ✅  │ น่ารักมาก very   │",
                "│        │        │ cute           │",
                "│ Dog 🐶 │ ไม่ ❌  │ ชอบวิ่งเล่น in    │",
                "│        │        │ the park       │",
                "╰────────┴────────┴────────────────╯",
            ],
        );
        assert_table(
            TABLE_THAI_ENGLISH_EMOJI,
            28,
            &[
                "╭────────┬────────┬────────╮",
                "│ ชื่อ     │ Status │ หมายเห │",
                "│        │        │ ตุ      │",
                "├────────┼────────┼────────┤",
                "│ แมว 🐱 │ ok ✅  │ น่ารักมา │",
                "│        │        │ ก very │",
                "│        │        │ cute   │",
                "│ Dog 🐶 │ ไม่ ❌  │ ชอบวิ่งเ │",
                "│        │        │ ล่น in  │",
                "│        │        │ the    │",
                "│        │        │ park   │",
                "╰────────┴────────┴────────╯",
            ],
        );
    }

    #[test]
    fn table_snapshot_alignment() {
        assert_table(
            TABLE_ALIGNMENT,
            60,
            &[
                "╭─────────────┬───────────────────┬───────╮",
                "│ Left        │      Centre       │ Right │",
                "├─────────────┼───────────────────┼───────┤",
                "│ a           │         b         │     c │",
                "│ longer left │        mid        │ 12345 │",
                "│ x           │ wider centre text │     7 │",
                "╰─────────────┴───────────────────┴───────╯",
            ],
        );
        assert_table(
            TABLE_ALIGNMENT,
            34,
            &[
                "╭───────────┬────────────┬───────╮",
                "│ Left      │   Centre   │ Right │",
                "├───────────┼────────────┼───────┤",
                "│ a         │     b      │     c │",
                "│ longer    │    mid     │ 12345 │",
                "│ left      │            │       │",
                "│ x         │   wider    │     7 │",
                "│           │   centre   │       │",
                "│           │    text    │       │",
                "╰───────────┴────────────┴───────╯",
            ],
        );
        assert_table(
            TABLE_ALIGNMENT,
            26,
            &[
                "╭───────┬────────┬───────╮",
                "│ Left  │ Centre │ Right │",
                "├───────┼────────┼───────┤",
                "│ a     │   b    │     c │",
                "│ longe │  mid   │ 12345 │",
                "│ r     │        │       │",
                "│ left  │        │       │",
                "│ x     │ wider  │     7 │",
                "│       │ centre │       │",
                "│       │  text  │       │",
                "╰───────┴────────┴───────╯",
            ],
        );
    }

    #[test]
    fn table_snapshot_single_column() {
        assert_table(
            TABLE_SINGLE_COLUMN,
            60,
            &[
                "╭───────────────────────────────────────────────╮",
                "│ Notes                                         │",
                "├───────────────────────────────────────────────┤",
                "│ short                                         │",
                "│ a much longer note that has to wrap somewhere │",
                "╰───────────────────────────────────────────────╯",
            ],
        );
        assert_table(
            TABLE_SINGLE_COLUMN,
            20,
            &[
                "╭──────────────────╮",
                "│ Notes            │",
                "├──────────────────┤",
                "│ short            │",
                "│ a much longer    │",
                "│ note that has to │",
                "│ wrap somewhere   │",
                "╰──────────────────╯",
            ],
        );
        assert_table(
            TABLE_SINGLE_COLUMN,
            6,
            &[
                "╭────╮",
                "│ No │",
                "│ te │",
                "│ s  │",
                "├────┤",
                "│ sh │",
                "│ or │",
                "│ t  │",
                "│ a  │",
                "│ mu │",
                "│ ch │",
                "│ lo │",
                "│ ng │",
                "│ er │",
                "│ no │",
                "│ te │",
                "│ th │",
                "│ at │",
                "│ ha │",
                "│ s  │",
                "│ to │",
                "│ wr │",
                "│ ap │",
                "│ so │",
                "│ me │",
                "│ wh │",
                "│ er │",
                "│ e  │",
                "╰────╯",
            ],
        );
        assert_table(
            TABLE_SINGLE_COLUMN,
            4,
            &[
                "Note", "s", "shor", "t", "a", "much", "long", "er", "note", "that", "has", "to",
                "wrap", "some", "wher", "e",
            ],
        );
    }

    #[test]
    fn table_snapshot_many_columns() {
        assert_table(
            TABLE_MANY_COLUMNS,
            80,
            &[
                "╭───────┬──────┬───────┬───────┬─────────┬──────┬─────┬───────╮",
                "│ A     │ B    │ C     │ D     │ E       │ F    │ G   │ H     │",
                "├───────┼──────┼───────┼───────┼─────────┼──────┼─────┼───────┤",
                "│ alpha │ beta │ gamma │ delta │ epsilon │ zeta │ eta │ theta │",
                "╰───────┴──────┴───────┴───────┴─────────┴──────┴─────┴───────╯",
            ],
        );
        assert_table(
            TABLE_MANY_COLUMNS,
            50,
            &[
                "╭─────┬─────┬─────┬─────┬──────┬─────┬─────┬─────╮",
                "│ A   │ B   │ C   │ D   │ E    │ F   │ G   │ H   │",
                "├─────┼─────┼─────┼─────┼──────┼─────┼─────┼─────┤",
                "│ alp │ bet │ gam │ del │ epsi │ zet │ eta │ the │",
                "│ ha  │ a   │ ma  │ ta  │ lon  │ a   │     │ ta  │",
                "╰─────┴─────┴─────┴─────┴──────┴─────┴─────┴─────╯",
            ],
        );
        assert_table(
            TABLE_MANY_COLUMNS,
            40,
            &[
                "A │ B │ C │ D │ E │ F │ G │ H",
                "alpha │ beta │ gamma │ delta │ epsilon │",
                "zeta │ eta │ theta",
            ],
        );
        assert_table(
            TABLE_MANY_COLUMNS,
            30,
            &[
                "A │ B │ C │ D │ E │ F │ G │ H",
                "alpha │ beta │ gamma │ delta │",
                "epsilon │ zeta │ eta │ theta",
            ],
        );
    }

    #[test]
    fn table_snapshot_empty_cells() {
        assert_table(
            TABLE_EMPTY_CELLS,
            50,
            &[
                "╭──────┬───────┬─────────────────────╮",
                "│ Name │ Value │ Comment             │",
                "├──────┼───────┼─────────────────────┤",
                "│ a    │       │                     │",
                "│      │ 2     │                     │",
                "│      │       │ only a comment here │",
                "╰──────┴───────┴─────────────────────╯",
            ],
        );
        assert_table(
            TABLE_EMPTY_CELLS,
            30,
            &[
                "╭──────┬───────┬─────────────╮",
                "│ Name │ Value │ Comment     │",
                "├──────┼───────┼─────────────┤",
                "│ a    │       │             │",
                "│      │ 2     │             │",
                "│      │       │ only a      │",
                "│      │       │ comment     │",
                "│      │       │ here        │",
                "╰──────┴───────┴─────────────╯",
            ],
        );
        assert_table(
            TABLE_EMPTY_CELLS,
            24,
            &[
                "╭──────┬───────┬───────╮",
                "│ Name │ Value │ Comme │",
                "│      │       │ nt    │",
                "├──────┼───────┼───────┤",
                "│ a    │       │       │",
                "│      │ 2     │       │",
                "│      │       │ only  │",
                "│      │       │ a com │",
                "│      │       │ ment  │",
                "│      │       │ here  │",
                "╰──────┴───────┴───────╯",
            ],
        );
    }

    #[test]
    fn table_snapshot_long_cell_beside_short() {
        assert_table(
            TABLE_LONG_CELL_BESIDE_SHORT,
            120,
            &[
                "╭───┬──────────────────────────────────────────────────────────────────────────────────────────┬────╮",
                "│ # │ Description                                                                              │ OK │",
                "├───┼──────────────────────────────────────────────────────────────────────────────────────────┼────┤",
                "│ 1 │ This description is very long and will certainly not fit on a single line of the preview │ y  │",
                "│ 2 │ short                                                                                    │ n  │",
                "╰───┴──────────────────────────────────────────────────────────────────────────────────────────┴────╯",
            ],
        );
        assert_table(
            TABLE_LONG_CELL_BESIDE_SHORT,
            60,
            &[
                "╭───┬─────────────────────────────────────────────────┬────╮",
                "│ # │ Description                                     │ OK │",
                "├───┼─────────────────────────────────────────────────┼────┤",
                "│ 1 │ This description is very long and will          │ y  │",
                "│   │ certainly not fit on a single line of the       │    │",
                "│   │ preview                                         │    │",
                "│ 2 │ short                                           │ n  │",
                "╰───┴─────────────────────────────────────────────────┴────╯",
            ],
        );
        assert_table(
            TABLE_LONG_CELL_BESIDE_SHORT,
            30,
            &[
                "╭───┬───────────────────┬────╮",
                "│ # │ Description       │ OK │",
                "├───┼───────────────────┼────┤",
                "│ 1 │ This description  │ y  │",
                "│   │ is very long and  │    │",
                "│   │ will certainly    │    │",
                "│   │ not fit on a      │    │",
                "│   │ single line of    │    │",
                "│   │ the preview       │    │",
                "│ 2 │ short             │ n  │",
                "╰───┴───────────────────┴────╯",
            ],
        );
        assert_table(
            TABLE_LONG_CELL_BESIDE_SHORT,
            18,
            &[
                "╭───┬───────┬────╮",
                "│ # │ Descr │ OK │",
                "│   │ iptio │    │",
                "│   │ n     │    │",
                "├───┼───────┼────┤",
                "│ 1 │ This  │ y  │",
                "│   │ descr │    │",
                "│   │ iptio │    │",
                "│   │ n is  │    │",
                "│   │ very  │    │",
                "│   │ long  │    │",
                "│   │ and   │    │",
                "│   │ will  │    │",
                "│   │ certa │    │",
                "│   │ inly  │    │",
                "│   │ not   │    │",
                "│   │ fit   │    │",
                "│   │ on a  │    │",
                "│   │ singl │    │",
                "│   │ e     │    │",
                "│   │ line  │    │",
                "│   │ of    │    │",
                "│   │ the p │    │",
                "│   │ revie │    │",
                "│   │ w     │    │",
                "│ 2 │ short │ n  │",
                "╰───┴───────┴────╯",
            ],
        );
    }
}
