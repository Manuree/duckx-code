//! Read-only Markdown rendering for the `preview` pane.
//!
//! The source text is parsed with `pulldown-cmark` and turned into wrapped, styled
//! terminal lines. Nothing here touches the disk; images show as their alt text and path.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Colors the preview uses. Built from the active color and syntax themes.
#[derive(Debug, Clone, Copy)]
pub struct MarkdownStyles {
    pub text: Style,
    pub heading: [Style; 6],
    pub code: Style,
    pub code_block: Style,
    pub quote: Style,
    pub link: Style,
    pub url: Style,
    pub marker: Style,
    pub rule: Style,
}

impl MarkdownStyles {
    pub fn from_theme(ui: &crate::theme::UiStyles, syntax: &crate::theme::SyntaxTheme) -> Self {
        use crate::theme::SyntaxRole;
        let base = Style::new().fg(ui.editor_fg).bg(ui.editor_bg);
        let color = |role| base.fg(syntax.color(role));
        let bold = Modifier::BOLD;
        Self {
            text: base,
            heading: [
                color(SyntaxRole::Keyword).add_modifier(bold | Modifier::UNDERLINED),
                color(SyntaxRole::Function).add_modifier(bold),
                color(SyntaxRole::Type).add_modifier(bold),
                color(SyntaxRole::Constant).add_modifier(bold),
                color(SyntaxRole::Property).add_modifier(bold),
                color(SyntaxRole::Comment).add_modifier(bold),
            ],
            code: color(SyntaxRole::String),
            code_block: color(SyntaxRole::String),
            quote: color(SyntaxRole::Comment).add_modifier(Modifier::ITALIC),
            link: color(SyntaxRole::Function).add_modifier(Modifier::UNDERLINED),
            url: color(SyntaxRole::Comment),
            marker: color(SyntaxRole::Operator),
            rule: color(SyntaxRole::Comment),
        }
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
    let mut renderer = Renderer {
        width: usize::from(width.max(1)),
        styles: *styles,
        lines: Vec::new(),
        pending: Vec::new(),
        containers: Vec::new(),
        inline: Vec::new(),
        heading: None,
        code_block: None,
        links: Vec::new(),
        image: 0,
        gap: false,
    };
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(source, options) {
        renderer.event(event);
    }
    renderer.flush();
    while renderer.lines.last().is_some_and(|line| line.width() == 0) {
        renderer.lines.pop();
    }
    renderer.lines
}

enum Container {
    Quote,
    List { next: Option<u64> },
    Item { marker: String, used: bool },
}

#[derive(Clone, Copy)]
enum Inline {
    Bold,
    Italic,
    Strike,
    Link,
}

struct Renderer {
    width: usize,
    styles: MarkdownStyles,
    lines: Vec<Line<'static>>,
    /// Text of the block being collected. `\n` inside a span forces a line break.
    pending: Vec<(String, Style)>,
    containers: Vec<Container>,
    inline: Vec<Inline>,
    heading: Option<usize>,
    code_block: Option<String>,
    /// Open links and images: URL and where their text starts in `pending`.
    links: Vec<(String, usize)>,
    image: usize,
    /// A blank line goes before the next block.
    gap: bool,
}

impl Renderer {
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if let Some(code) = self.code_block.as_mut() {
                    code.push_str(&text);
                } else {
                    let style = self.inline_style();
                    self.push(&text, style);
                }
            }
            Event::Code(text) => {
                let style = self.styles.code;
                self.push(&text, style);
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                let style = self.styles.code;
                self.push(&text, style);
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
                let width = self.width.saturating_sub(self.prefix_width());
                let rule = "─".repeat(width.max(1));
                let mut spans = self.prefix(true);
                spans.push(Span::styled(rule, self.styles.rule));
                self.lines.push(Line::from(spans));
                self.gap = true;
            }
            Event::TaskListMarker(done) => {
                let mark = if done { "[x] " } else { "[ ] " };
                self.push(mark, self.styles.marker);
            }
            Event::FootnoteReference(name) => {
                let style = self.styles.url;
                self.push(&format!("[^{name}]"), style);
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.begin_block(),
            Tag::Heading { level, .. } => {
                self.flush();
                self.begin_block();
                self.heading = Some(heading_index(level));
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.begin_block();
                self.containers.push(Container::Quote);
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.begin_block();
                if let CodeBlockKind::Fenced(lang) = kind
                    && !lang.trim().is_empty()
                {
                    let mut spans = self.prefix(true);
                    spans.push(Span::styled(format!("  {}", lang.trim()), self.styles.url));
                    self.lines.push(Line::from(spans));
                }
                self.code_block = Some(String::new());
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
                self.containers.push(Container::List { next: start });
            }
            Tag::Item => {
                self.flush();
                self.gap = false;
                let depth = self
                    .containers
                    .iter()
                    .filter(|container| matches!(container, Container::List { .. }))
                    .count();
                let marker =
                    match self
                        .containers
                        .iter_mut()
                        .rev()
                        .find_map(|container| match container {
                            Container::List { next } => Some(next),
                            _ => None,
                        }) {
                        Some(Some(number)) => {
                            let marker = format!("{number}. ");
                            *number += 1;
                            marker
                        }
                        _ => match depth {
                            0 | 1 => "• ".to_string(),
                            2 => "◦ ".to_string(),
                            _ => "▪ ".to_string(),
                        },
                    };
                self.containers.push(Container::Item {
                    marker,
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
            Tag::Image { dest_url, .. } => {
                self.image += 1;
                self.links.push((dest_url.to_string(), self.pending.len()));
                let style = self.styles.url;
                self.push("[image: ", style);
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                self.flush();
                self.heading = None;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
                self.gap = true;
            }
            TagEnd::CodeBlock => {
                let code = self.code_block.take().unwrap_or_default();
                let code = code.strip_suffix('\n').unwrap_or(&code);
                for line in code.split('\n') {
                    let line = line.replace('\t', "    ");
                    let text = format!("  {line}");
                    let room = self.width.saturating_sub(self.prefix_width());
                    for chunk in hard_chunks(&text, room.max(1)) {
                        let mut spans = self.prefix(false);
                        spans.push(Span::styled(chunk, self.styles.code_block));
                        self.lines.push(Line::from(spans));
                    }
                }
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
                let shown: String = self
                    .pending
                    .get(from..)
                    .unwrap_or_default()
                    .iter()
                    .map(|(text, _)| text.as_str())
                    .collect();
                if !url.is_empty() && shown != url && !url.starts_with('#') {
                    let style = self.styles.url;
                    self.push(&format!(" ({url})"), style);
                }
            }
            TagEnd::Image => {
                self.image = self.image.saturating_sub(1);
                let (url, _) = self.links.pop().unwrap_or_default();
                let style = self.styles.url;
                self.push(&format!("] ({url})"), style);
            }
            _ => {}
        }
    }

    fn begin_block(&mut self) {
        if self.gap && !self.lines.is_empty() {
            let spans = self.quote_prefix();
            self.lines.push(Line::from(spans));
        }
        self.gap = false;
    }

    fn inline_style(&self) -> Style {
        if self.image > 0 {
            return self.styles.url;
        }
        let mut style = match self.heading {
            Some(level) => self.styles.heading[level],
            None if self.in_quote() => self.styles.quote,
            None => self.styles.text,
        };
        for inline in &self.inline {
            style = match inline {
                Inline::Bold => style.add_modifier(Modifier::BOLD),
                Inline::Italic => style.add_modifier(Modifier::ITALIC),
                Inline::Strike => style.add_modifier(Modifier::CROSSED_OUT),
                Inline::Link => self.styles.link,
            };
        }
        style
    }

    fn in_quote(&self) -> bool {
        self.containers
            .iter()
            .any(|container| matches!(container, Container::Quote))
    }

    fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        self.pending.push((text.to_string(), style));
    }

    fn prefix_width(&self) -> usize {
        self.containers
            .iter()
            .map(|container| match container {
                Container::Quote => 2,
                Container::List { .. } => 0,
                Container::Item { marker, .. } => marker.width(),
            })
            .sum()
    }

    fn quote_prefix(&self) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        for container in &self.containers {
            match container {
                Container::Quote => spans.push(Span::styled("│ ", self.styles.marker)),
                Container::Item { marker, .. } => {
                    spans.push(Span::styled(" ".repeat(marker.width()), self.styles.text))
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
        let mut spans = Vec::new();
        for (index, container) in self.containers.iter_mut().enumerate() {
            match container {
                Container::Quote => spans.push(Span::styled("│ ", self.styles.marker)),
                Container::List { .. } => {}
                Container::Item { marker, used } => {
                    if first && !*used && Some(index) == innermost {
                        spans.push(Span::styled(marker.clone(), self.styles.marker));
                        *used = true;
                    } else {
                        spans.push(Span::styled(" ".repeat(marker.width()), self.styles.text));
                    }
                }
            }
        }
        spans
    }

    /// Wraps the collected block into lines.
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        let room = self.width.saturating_sub(self.prefix_width()).max(1);
        let mut first = true;
        for row in wrap(&pending, room) {
            let mut spans = self.prefix(first);
            first = false;
            spans.extend(row);
            self.lines.push(Line::from(spans));
        }
        self.gap = true;
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

/// Splits `text` into pieces of at most `width` columns.
fn hard_chunks(text: &str, width: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(c);
        used += w;
    }
    chunks.push(current);
    chunks
}

/// Greedy word wrap that keeps each word's style. `\n` starts a new row.
fn wrap(spans: &[(String, Style)], width: usize) -> Vec<Vec<Span<'static>>> {
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut tokens: Vec<(String, Style, bool)> = Vec::new();
    for (text, style) in spans {
        let mut word = String::new();
        let mut space = None;
        for c in text.chars() {
            let is_space = c == ' ' || c == '\t';
            if c == '\n' {
                if !word.is_empty() {
                    tokens.push((std::mem::take(&mut word), *style, space.unwrap_or(false)));
                }
                tokens.push(("\n".to_string(), *style, false));
                space = None;
                continue;
            }
            if space.is_some_and(|s| s != is_space) && !word.is_empty() {
                tokens.push((std::mem::take(&mut word), *style, space.unwrap_or(false)));
            }
            space = Some(is_space);
            word.push(if c == '\t' { ' ' } else { c });
        }
        if !word.is_empty() {
            tokens.push((word, *style, space.unwrap_or(false)));
        }
    }
    let flush_row = |rows: &mut Vec<Vec<Span<'static>>>, row: &mut Vec<Span<'static>>| {
        while row
            .last()
            .is_some_and(|span: &Span| span.content.trim().is_empty())
        {
            row.pop();
        }
        rows.push(std::mem::take(row));
    };
    for (text, style, is_space) in tokens {
        if text == "\n" {
            flush_row(&mut rows, &mut row);
            used = 0;
            continue;
        }
        if is_space {
            if used == 0 {
                continue;
            }
            let w = text.width();
            if used + w > width {
                flush_row(&mut rows, &mut row);
                used = 0;
                continue;
            }
            used += w;
            row.push(Span::styled(text, style));
            continue;
        }
        let w = text.width();
        if used + w > width && used > 0 {
            flush_row(&mut rows, &mut row);
            used = 0;
        }
        if w > width {
            for chunk in hard_chunks(&text, width) {
                let cw = chunk.width();
                if used + cw > width && used > 0 {
                    flush_row(&mut rows, &mut row);
                    used = 0;
                }
                used += cw;
                row.push(Span::styled(chunk, style));
            }
            continue;
        }
        used += w;
        row.push(Span::styled(text, style));
    }
    if !row.is_empty() || rows.is_empty() {
        flush_row(&mut rows, &mut row);
    }
    rows
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

    #[test]
    fn headings_paragraphs_and_inline_styles_render_without_markup() {
        let styles = styles();
        let lines = render(
            "# Header\n\nSome **bold** and *italic* with `code`.\n",
            40,
            &styles,
        );
        assert_eq!(
            plain(&lines),
            ["Header", "", "Some bold and italic with code."]
        );
        assert_eq!(lines[0].spans[0].style, styles.heading[0]);
        let bold = lines[2]
            .spans
            .iter()
            .find(|span| span.content == "bold")
            .unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        let italic = lines[2]
            .spans
            .iter()
            .find(|span| span.content == "italic")
            .unwrap();
        assert!(italic.style.add_modifier.contains(Modifier::ITALIC));
        let code = lines[2]
            .spans
            .iter()
            .find(|span| span.content == "code")
            .unwrap();
        assert_eq!(code.style, styles.code);
    }

    #[test]
    fn lists_quotes_code_rules_links_and_images() {
        let source = "- one\n- two\n  1. inner\n\n> quoted text\n\n```rust\nfn main() {}\n```\n\n---\n\n[site](https://example.com) ![logo](img/logo.png)\n";
        let lines = plain(&render(source, 30, &styles()));
        assert_eq!(
            lines,
            [
                "• one",
                "• two",
                "  1. inner",
                "",
                "│ quoted text",
                "",
                "  rust",
                "  fn main() {}",
                "",
                "──────────────────────────────",
                "",
                "site (https://example.com)",
                "[image: logo] (img/logo.png)",
            ]
        );
    }

    #[test]
    fn long_paragraphs_wrap_to_the_width_and_keep_the_list_indent() {
        let lines = plain(&render("- alpha beta gamma delta epsilon\n", 12, &styles()));
        assert_eq!(lines, ["• alpha beta", "  gamma", "  delta", "  epsilon"]);
        for line in &lines {
            assert!(line.width() <= 12, "{line}");
        }
        let long = plain(&render("abcdefghijklmnop", 5, &styles()));
        assert_eq!(long, ["abcde", "fghij", "klmno", "p"]);
    }
}
