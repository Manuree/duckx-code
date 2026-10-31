use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const TAB_WIDTH: usize = 4;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// Same hash [`Buffer::fingerprint`] computes, over an owned byte string (the bytes written to disk).
pub fn fingerprint_bytes(bytes: &[u8]) -> u64 {
    fnv1a(FNV_OFFSET, bytes)
}

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }

    fn detect(text: &str) -> Self {
        if text.contains("\r\n") {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentStyle {
    Tabs,
    Spaces(usize),
}

impl IndentStyle {
    fn unit(self) -> String {
        match self {
            IndentStyle::Tabs => "\t".to_string(),
            IndentStyle::Spaces(width) => " ".repeat(width),
        }
    }
}

/// Positions are char indices into the rope. `anchor` stays fixed while
/// `Shift` is held; `head` is where the cursor is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn point(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    pub fn from(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn to(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Movement {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocumentStart,
    DocumentEnd,
    PageUp(usize),
    PageDown(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Change {
    pos: usize,
    removed: String,
    inserted: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Typing,
    Other,
}

#[derive(Debug)]
struct Transaction {
    changes: Vec<Change>,
    before: Selection,
    after: Selection,
    kind: EditKind,
}

#[derive(Debug, Default)]
struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    coalesce: bool,
}

impl History {
    fn record(&mut self, change: Change, before: Selection, after: Selection, kind: EditKind) {
        self.redo.clear();

        if kind == EditKind::Typing
            && self.coalesce
            && change.removed.is_empty()
            && let Some(last) = self.undo.last_mut()
            && last.kind == EditKind::Typing
            && let Some(prev) = last.changes.last_mut()
            && prev.pos + prev.inserted.chars().count() == change.pos
        {
            prev.inserted.push_str(&change.inserted);
            last.after = after;
            return;
        }

        self.push(vec![change], before, after, kind);
    }

    fn push(&mut self, changes: Vec<Change>, before: Selection, after: Selection, kind: EditKind) {
        self.redo.clear();
        self.undo.push(Transaction {
            changes,
            before,
            after,
            kind,
        });
        self.coalesce = kind == EditKind::Typing;
    }

    fn seal(&mut self) {
        self.coalesce = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Word,
    Punctuation,
}

impl CharClass {
    fn of(grapheme: &str) -> Self {
        let c = grapheme.chars().next().unwrap_or(' ');
        if c.is_whitespace() {
            CharClass::Whitespace
        } else if c.is_alphanumeric() || c == '_' {
            CharClass::Word
        } else {
            CharClass::Punctuation
        }
    }
}

/// A row/column in bytes, matching tree-sitter's point (column is bytes from the line start).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub row: usize,
    pub column: usize,
}

/// One rope mutation, in byte coordinates, recorded so a syntax tree can stay aligned.
/// Coordinates are in the text as it was just before this edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start: Point,
    pub old_end: Point,
    pub new_end: Point,
}

#[derive(Debug)]
pub struct Buffer {
    rope: Rope,
    selection: Selection,
    line_ending: LineEnding,
    indent_style: IndentStyle,
    /// Display width of a tab character, and the size of one space indent.
    tab_width: usize,
    /// Sticky visual column kept across consecutive vertical moves.
    target_col: Option<usize>,
    history: History,
    /// Bumped once per text mutation. Undo and redo count.
    revision: u64,
    pending_edits: Vec<ByteEdit>,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    pub fn new() -> Self {
        Self::from_text("")
    }

    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            selection: Selection::point(0),
            line_ending: LineEnding::detect(text),
            indent_style: IndentStyle::Spaces(TAB_WIDTH),
            tab_width: TAB_WIDTH,
            target_col: None,
            history: History::default(),
            revision: 0,
            pending_edits: Vec::new(),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Edits since the last call, and the revision they brought the buffer to.
    pub fn take_edits(&mut self) -> (u64, Vec<ByteEdit>) {
        (self.revision, std::mem::take(&mut self.pending_edits))
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn indent_style(&self) -> IndentStyle {
        self.indent_style
    }

    pub fn tab_width(&self) -> usize {
        self.tab_width
    }

    /// Next Tab, indent, and the on-screen width of a tab character use these values.
    /// Existing whitespace is left as it is. `forced` replaces the ending used for new breaks.
    pub fn apply_editor_style(
        &mut self,
        indent_with_spaces: bool,
        tab_width: usize,
        forced: Option<LineEnding>,
    ) {
        let tab_width = tab_width.max(1);
        self.tab_width = tab_width;
        self.indent_style = if indent_with_spaces {
            IndentStyle::Spaces(tab_width)
        } else {
            IndentStyle::Tabs
        };
        if let Some(ending) = forced {
            self.line_ending = ending;
        }
    }

    /// Rewrites line endings, trims trailing whitespace, and adds a final newline
    /// so the bytes written to disk match the buffer afterward.
    pub fn prepare_for_save(
        &mut self,
        forced: Option<LineEnding>,
        trim: bool,
        final_newline: bool,
    ) {
        if let Some(ending) = forced {
            self.line_ending = ending;
        }
        let mut edits = Vec::new();
        let ending = self.line_ending;
        let chars: Vec<char> = self.rope.chars().collect();
        if forced.is_some() {
            match ending {
                LineEnding::Lf => {
                    for (index, ch) in chars.iter().enumerate() {
                        if *ch == '\r' && chars.get(index + 1) == Some(&'\n') {
                            edits.push((index, 1, String::new()));
                        }
                    }
                }
                LineEnding::Crlf => {
                    for (index, ch) in chars.iter().enumerate() {
                        if *ch == '\n' && (index == 0 || chars[index - 1] != '\r') {
                            edits.push((index, 0, "\r".to_string()));
                        }
                    }
                }
            }
        }
        if trim {
            let mut line = 0;
            while line < self.rope.len_lines() {
                let start = self.line_start(line);
                let len = self.line_content_len(line);
                let content: String = self.rope.slice(start..start + len).chars().collect();
                let trimmed = content.trim_end_matches([' ', '\t']);
                let cut = content.chars().count() - trimmed.chars().count();
                if cut > 0 {
                    edits.push((start + trimmed.chars().count(), cut, String::new()));
                }
                line += 1;
            }
        }
        if final_newline
            && self.rope.len_chars() > 0
            && self.rope.char(self.rope.len_chars() - 1) != '\n'
        {
            edits.push((self.rope.len_chars(), 0, ending.as_str().to_string()));
        }
        edits.sort_by_key(|edit| edit.0);
        edits.dedup_by(|right, left| right.0 == left.0 && right.1 == left.1);
        self.apply_batch(edits);
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// Line text without its trailing line ending.
    pub fn line_content(&self, line: usize) -> String {
        let len = self.line_content_len(line);
        self.rope.line(line).slice(..len).to_string()
    }

    /// Zero-based `(line, visual column)` of the cursor.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let head = self.selection.head;
        let line = self.rope.char_to_line(head);
        let col = head - self.line_start(line);
        (
            line,
            visual_col(&self.line_content(line), col, self.tab_width),
        )
    }

    pub fn selected_text(&self) -> String {
        let sel = self.selection;
        self.rope.slice(sel.from()..sel.to()).to_string()
    }

    /// Text a copy acts on: the selection, or the whole cursor line (with its line ending) when nothing is selected.
    pub fn copy_text(&self) -> String {
        if !self.selection.is_empty() {
            return self.selected_text();
        }
        let line = self.rope.char_to_line(self.selection.head);
        let mut text = self.rope.line(line).to_string();
        if self.line_content_len(line) == text.chars().count() {
            text.push_str(self.line_ending.as_str());
        }
        text
    }

    /// Deletes the text [`copy_text`](Self::copy_text) names, as one undo step.
    /// A last line with no line break also drops the break before it, so the line disappears.
    pub fn cut_text(&mut self) -> String {
        let text = self.copy_text();
        if !self.selection.is_empty() {
            let sel = self.selection;
            self.apply_edit(sel.from(), sel.to(), "", EditKind::Other);
            return text;
        }
        let line = self.rope.char_to_line(self.selection.head);
        let mut start = self.line_start(line);
        let end = if line + 1 < self.rope.len_lines() {
            self.line_start(line + 1)
        } else {
            self.rope.len_chars()
        };
        let ends_with_break = end > start && self.rope.char(end - 1) == '\n';
        if !ends_with_break && line > 0 {
            start = self.line_end(line - 1);
        }
        if start < end {
            self.apply_edit(start, end, "", EditKind::Other);
        }
        text
    }

    /// Fingerprint of the exact bytes [`Rope::to_string`](ropey::Rope::to_string) would produce.
    /// Compared with the fingerprint taken at load or save to decide whether the buffer is modified.
    pub fn fingerprint(&self) -> u64 {
        let mut hash = FNV_OFFSET;
        for chunk in self.rope.chunks() {
            hash = fnv1a(hash, chunk.as_bytes());
        }
        hash
    }

    /// Puts the cursor on `head`. `extend` keeps the anchor, the same way Shift navigation does.
    pub fn move_to(&mut self, head: usize, extend: bool) {
        self.history.seal();
        self.target_col = None;
        let head = head.min(self.rope.len_chars());
        self.selection = if extend {
            Selection {
                anchor: self.selection.anchor,
                head,
            }
        } else {
            Selection::point(head)
        };
    }

    // ---- Movement ----------------------------------------------------------

    pub fn move_cursor(&mut self, movement: Movement, extend: bool) {
        self.history.seal();
        let sel = self.selection;

        if !extend && !sel.is_empty() {
            let collapsed = match movement {
                Movement::Left => Some(sel.from()),
                Movement::Right => Some(sel.to()),
                _ => None,
            };
            if let Some(pos) = collapsed {
                self.selection = Selection::point(pos);
                self.target_col = None;
                return;
            }
        }

        let head = sel.head;
        let new_head = match movement {
            Movement::Up => self.vertical(head, -1),
            Movement::Down => self.vertical(head, 1),
            Movement::PageUp(lines) => self.vertical(head, -(lines.max(1) as isize)),
            Movement::PageDown(lines) => self.vertical(head, lines.max(1) as isize),
            other => {
                self.target_col = None;
                match other {
                    Movement::Left => self.left_of(head),
                    Movement::Right => self.right_of(head),
                    Movement::WordLeft => self.word_left_of(head),
                    Movement::WordRight => self.word_right_of(head),
                    Movement::LineStart => self.smart_line_start(head),
                    Movement::LineEnd => self.line_end(self.rope.char_to_line(head)),
                    Movement::DocumentStart => 0,
                    Movement::DocumentEnd => self.rope.len_chars(),
                    _ => unreachable!("vertical movements handled above"),
                }
            }
        };

        self.selection = if extend {
            Selection {
                anchor: sel.anchor,
                head: new_head,
            }
        } else {
            Selection::point(new_head)
        };
    }

    /// Replaces non-overlapping literal matches. The inserted text is not searched again.
    pub fn replace_literal(text: &str, query: &str, replacement: &str) -> (String, usize) {
        if query.is_empty() {
            return (text.to_string(), 0);
        }
        let mut out = String::new();
        let mut from = 0;
        let mut count = 0;
        while let Some(rel) = text[from..].find(query) {
            out.push_str(&text[from..from + rel]);
            out.push_str(replacement);
            from += rel + query.len();
            count += 1;
        }
        out.push_str(&text[from..]);
        (out, count)
    }

    /// Non-overlapping literal matches of `query` in `text`. Case-sensitive.
    /// An empty query matches nothing.
    pub fn literal_match_count(text: &str, query: &str) -> usize {
        if query.is_empty() {
            return 0;
        }
        let mut count = 0;
        let mut from = 0;
        while let Some(rel) = text[from..].find(query) {
            count += 1;
            from += rel + query.len();
        }
        count
    }

    /// Non-overlapping literal matches of `query`, in order. Case-sensitive.
    /// An empty query matches nothing.
    pub fn find_spans(&self, query: &str) -> Vec<(usize, usize)> {
        if query.is_empty() {
            return Vec::new();
        }
        let text = self.rope.to_string();
        let query_chars = query.chars().count();
        let mut spans = Vec::new();
        let mut from_byte = 0;
        let mut from_char = 0;
        while let Some(rel) = text[from_byte..].find(query) {
            let start_byte = from_byte + rel;
            let start = from_char + text[from_byte..start_byte].chars().count();
            let end = start + query_chars;
            spans.push((start, end));
            from_byte = start_byte + query.len();
            from_char = end;
        }
        spans
    }

    /// Selects `[start, end)`. The cursor sits on the first character.
    pub fn select_span(&mut self, start: usize, end: usize) {
        self.history.seal();
        self.target_col = None;
        let len = self.rope.len_chars();
        let start = start.min(len);
        let end = end.min(len).max(start);
        self.selection = Selection {
            anchor: end,
            head: start,
        };
    }

    /// Applies non-overlapping char edits, sorted by start, as one undo step.
    /// `cursor` is a pre-edit char index that becomes a caret after the edits.
    pub fn edit_batch(&mut self, edits: Vec<(usize, usize, String)>, cursor: usize) {
        self.apply_batch_cursor(edits, Some(cursor), true);
    }

    /// Replaces the char range `[start, end)` as one undo step.
    pub fn replace_span(&mut self, start: usize, end: usize, replacement: &str) {
        let len = self.rope.len_chars();
        let start = start.min(len);
        let end = end.min(len).max(start);
        self.apply_edit(start, end, replacement, EditKind::Other);
    }

    /// Replaces every non-overlapping match as one undo step.
    /// Text inserted by a replacement is not searched again.
    /// The cursor is left at the start of the first replacement.
    /// Returns how many matches were replaced.
    pub fn replace_all(&mut self, query: &str, replacement: &str) -> usize {
        let spans = self.find_spans(query);
        let count = spans.len();
        if count == 0 {
            return 0;
        }
        let cursor_at = spans.first().map(|(start, _)| *start);
        let edits = spans
            .into_iter()
            .map(|(start, end)| (start, end - start, replacement.to_string()))
            .collect();
        self.apply_batch_cursor(edits, cursor_at, false);
        count
    }

    /// Zero-based `(line, character column)` of the cursor. The column skips the line ending.
    pub fn cursor_line_char(&self) -> (usize, usize) {
        let head = self.selection.head;
        let line = self.rope.char_to_line(head);
        (line, head - self.line_start(line))
    }

    /// Moves the cursor to zero-based `line` and character `char_col`, clamped to that line.
    pub fn place_cursor(&mut self, line: usize, char_col: usize) {
        self.history.seal();
        self.target_col = None;
        let line = line.min(self.rope.len_lines().saturating_sub(1));
        let start = self.line_start(line);
        let end = start + self.line_content_len(line);
        let col = char_col.min(end.saturating_sub(start));
        self.selection = Selection::point(start + col);
    }

    /// Moves the cursor to the start of zero-based `line`, clamped to the last line.
    pub fn goto_line(&mut self, line: usize) {
        self.history.seal();
        self.target_col = None;
        let line = line.min(self.rope.len_lines() - 1);
        self.selection = Selection::point(self.line_start(line));
    }

    pub fn select_all(&mut self) {
        self.history.seal();
        self.target_col = None;
        self.selection = Selection {
            anchor: 0,
            head: self.rope.len_chars(),
        };
    }

    // ---- Editing -----------------------------------------------------------

    pub fn insert_char(&mut self, c: char) {
        if c == '\n' {
            self.insert_newline();
            return;
        }
        let sel = self.selection;
        let mut utf8 = [0u8; 4];
        self.apply_edit(
            sel.from(),
            sel.to(),
            c.encode_utf8(&mut utf8),
            EditKind::Typing,
        );
    }

    /// Inserts a line break and carries over the current line's indentation.
    pub fn insert_newline(&mut self) {
        let sel = self.selection;
        let line = self.rope.char_to_line(sel.from());
        let col = sel.from() - self.line_start(line);
        let indent: String = self
            .line_content(line)
            .chars()
            .take(col)
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let text = format!("{}{}", self.line_ending.as_str(), indent);
        self.apply_edit(sel.from(), sel.to(), &text, EditKind::Other);
    }

    /// Inserts arbitrary text (e.g. a paste), normalizing line endings to the buffer's.
    pub fn insert_text(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let normalized = match self.line_ending {
            LineEnding::Lf => normalized,
            LineEnding::Crlf => normalized.replace('\n', "\r\n"),
        };
        let sel = self.selection;
        self.apply_edit(sel.from(), sel.to(), &normalized, EditKind::Other);
    }

    /// Inside leading spaces, deletes back to the previous indent stop instead of one space.
    pub fn delete_backward(&mut self) {
        let sel = self.selection;
        if !sel.is_empty() {
            self.apply_edit(sel.from(), sel.to(), "", EditKind::Other);
            return;
        }
        let from = match self.indent_style {
            IndentStyle::Spaces(width) => {
                let line = self.rope.char_to_line(sel.head);
                let start = self.line_start(line);
                let col = sel.head - start;
                let only_spaces_before = self.rope.slice(start..sel.head).chars().all(|c| c == ' ');
                if col > 0 && only_spaces_before {
                    let rem = if col.is_multiple_of(width) {
                        width
                    } else {
                        col % width
                    };
                    sel.head - rem
                } else {
                    self.left_of(sel.head)
                }
            }
            IndentStyle::Tabs => self.left_of(sel.head),
        };
        self.apply_edit(from, sel.head, "", EditKind::Other);
    }

    /// `Tab`: indents every selected line when the selection spans lines,
    /// otherwise inserts one indent (spaces up to the next stop, or a tab).
    pub fn indent(&mut self) {
        let sel = self.selection;
        let (first, last) = self.selected_lines();
        let spans_lines = self.rope.char_to_line(sel.from()) != self.rope.char_to_line(sel.to());

        if !spans_lines {
            let text = match self.indent_style {
                IndentStyle::Tabs => "\t".to_string(),
                IndentStyle::Spaces(width) => {
                    let line = self.rope.char_to_line(sel.from());
                    let col = sel.from() - self.line_start(line);
                    let vcol = visual_col(&self.line_content(line), col, self.tab_width);
                    " ".repeat(width - vcol % width)
                }
            };
            self.apply_edit(sel.from(), sel.to(), &text, EditKind::Other);
            return;
        }

        let unit = self.indent_style.unit();
        let edits = (first..=last)
            .filter(|&line| self.line_content_len(line) > 0)
            .map(|line| (self.line_start(line), 0, unit.clone()))
            .collect();
        self.apply_batch(edits);
    }

    /// `Shift+Tab`: removes one indent level from every selected line (or the cursor line).
    pub fn outdent(&mut self) {
        let (first, last) = self.selected_lines();
        let width = match self.indent_style {
            IndentStyle::Spaces(width) => width,
            IndentStyle::Tabs => self.tab_width,
        };
        let edits = (first..=last)
            .filter_map(|line| {
                let content = self.line_content(line);
                let removed = if content.starts_with('\t') {
                    1
                } else {
                    let spaces = content.chars().take_while(|c| *c == ' ').count();
                    if spaces.is_multiple_of(width) {
                        spaces.min(width)
                    } else {
                        spaces % width
                    }
                };
                (removed > 0).then(|| (self.line_start(line), removed, String::new()))
            })
            .collect();
        self.apply_batch(edits);
    }

    /// Moves the selected lines up or down by one line, as one undo step.
    /// A caret keeps its column. A selection keeps covering the moved lines.
    /// Returns `false` when the block is already at that edge.
    pub fn move_block(&mut self, down: bool) -> bool {
        let (first, last) = self.selected_lines();
        let edge = self.last_content_line();
        if down && last >= edge || !down && first == 0 {
            return false;
        }
        let point = self.selection.is_empty();
        let head = self.selection.head;
        let head_line = self.rope.char_to_line(head);
        let column = head - self.line_start(head_line);
        let block_start = self.line_start(first);
        let block_end = self.line_start(last + 1);
        let block = self.rope.slice(block_start..block_end).to_string();
        let block_chars = block.chars().count();
        let new_block = if down {
            let neighbor_end = if last + 2 >= self.rope.len_lines() {
                self.rope.len_chars()
            } else {
                self.line_start(last + 2)
            };
            let neighbor = self.rope.slice(block_end..neighbor_end).to_string();
            self.edit_batch(
                vec![(
                    block_start,
                    neighbor_end - block_start,
                    format!("{neighbor}{block}"),
                )],
                block_start,
            );
            block_start + neighbor.chars().count()
        } else {
            let neighbor_start = self.line_start(first - 1);
            let neighbor = self.rope.slice(neighbor_start..block_start).to_string();
            self.edit_batch(
                vec![(
                    neighbor_start,
                    block_end - neighbor_start,
                    format!("{block}{neighbor}"),
                )],
                neighbor_start,
            );
            neighbor_start
        };
        if point {
            let dest_line = if down {
                head_line + 1
            } else {
                head_line.saturating_sub(1)
            };
            let start = self.line_start(dest_line.min(self.last_content_line()));
            let len = self.line_content_len(self.rope.char_to_line(start));
            let at = start + column.min(len);
            self.selection = Selection::point(at);
        } else {
            let end = new_block + block_chars;
            self.selection = Selection {
                anchor: end,
                head: new_block,
            };
        }
        self.target_col = None;
        true
    }

    /// Deletes every line the selection touches, including a caret's line, as one undo step.
    /// A selection that ends at the start of a line does not include that line.
    /// The caret lands at the start of the next line, or at the end of the previous line
    /// when the deleted lines were the end of the file.
    pub fn delete_lines(&mut self) {
        let (first, last) = self.selected_lines();
        let mut start = self.line_start(first);
        let end = if last + 1 < self.rope.len_lines() {
            self.line_start(last + 1)
        } else {
            self.rope.len_chars()
        };
        if start == end {
            return;
        }
        let ends_with_break = self.rope.char(end - 1) == '\n';
        if !ends_with_break && first > 0 {
            start = self.line_end(first - 1);
        }
        if start < end {
            self.apply_edit(start, end, "", EditKind::Other);
        }
    }

    /// Copies every line the selection touches above or below, as one undo step.
    /// A caret keeps its column on the copy. A selection covers that same span on the copy.
    pub fn duplicate_block(&mut self, down: bool) {
        let (first, last) = self.selected_lines();
        let point = self.selection.is_empty();
        let head = self.selection.head;
        let anchor = self.selection.anchor;
        let head_line = self.rope.char_to_line(head);
        let anchor_line = self.rope.char_to_line(anchor);
        let head_col = head - self.line_start(head_line);
        let anchor_col = anchor - self.line_start(anchor_line);

        let block_start = self.line_start(first);
        let block_end = if last + 1 < self.rope.len_lines() {
            self.line_start(last + 1)
        } else {
            self.rope.len_chars()
        };
        let block = self.rope.slice(block_start..block_end).to_string();
        let ends_with_break = block_end > block_start && self.rope.char(block_end - 1) == '\n';
        let ending = self.line_ending.as_str();
        let (at, text) = if down {
            if ends_with_break {
                (block_end, block)
            } else {
                (block_end, format!("{ending}{block}"))
            }
        } else if ends_with_break {
            (block_start, block)
        } else {
            (block_start, format!("{block}{ending}"))
        };

        self.apply_edit(at, at, &text, EditKind::Other);
        let shift = if down { last - first + 1 } else { 0 };
        let selection = if point {
            Selection::point(self.column_on_line(head_line + shift, head_col))
        } else {
            Selection {
                anchor: self.column_on_line(anchor_line + shift, anchor_col),
                head: self.column_on_line(head_line + shift, head_col),
            }
        };
        self.selection = selection;
        self.target_col = None;
        if let Some(tx) = self.history.undo.last_mut() {
            tx.after = selection;
        }
    }

    fn column_on_line(&self, line: usize, column: usize) -> usize {
        let line = line.min(self.rope.len_lines().saturating_sub(1));
        let start = self.line_start(line);
        start + column.min(self.line_content_len(line))
    }

    /// Adds `marker` after the indent of each selected line, or removes one when every
    /// non-blank line already starts with it. One undo step.
    pub fn toggle_comment(&mut self, marker: &str) -> bool {
        if marker.is_empty() {
            return false;
        }
        let (first, last) = self.selected_lines();
        let lines: Vec<(usize, String)> = (first..=last)
            .map(|line| {
                let start = self.line_start(line);
                let len = self.line_content_len(line);
                (start, self.rope.slice(start..start + len).to_string())
            })
            .collect();
        let commented = lines
            .iter()
            .any(|(_, text)| text.trim_start().starts_with(marker))
            && lines.iter().all(|(_, text)| {
                let trimmed = text.trim_start();
                trimmed.is_empty() || trimmed.starts_with(marker)
            });
        let edits: Vec<(usize, usize, String)> = lines
            .iter()
            .filter_map(|(start, text)| {
                let indent = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
                let rest: String = text.chars().skip(indent).collect();
                if commented {
                    if !rest.starts_with(marker) {
                        return None;
                    }
                    let mut remove = marker.chars().count();
                    if rest.chars().nth(remove) == Some(' ') {
                        remove += 1;
                    }
                    Some((*start + indent, remove, String::new()))
                } else {
                    Some((*start + indent, 0, format!("{marker} ")))
                }
            })
            .collect();
        if edits.is_empty() {
            return false;
        }
        self.apply_batch(edits);
        true
    }

    /// Last line that holds text. A trailing newline does not count as another line.
    fn last_content_line(&self) -> usize {
        let lines = self.rope.len_lines();
        if lines > 1 && self.rope.char(self.rope.len_chars() - 1) == '\n' {
            lines - 2
        } else {
            lines.saturating_sub(1)
        }
    }

    pub fn delete_forward(&mut self) {
        let sel = self.selection;
        if sel.is_empty() {
            let to = self.right_of(sel.head);
            self.apply_edit(sel.head, to, "", EditKind::Other);
        } else {
            self.apply_edit(sel.from(), sel.to(), "", EditKind::Other);
        }
    }

    pub fn undo(&mut self) -> bool {
        self.history.seal();
        let Some(tx) = self.history.undo.pop() else {
            return false;
        };
        for change in tx.changes.iter().rev() {
            let end = change.pos + change.inserted.chars().count();
            let captured = self.capture_edit(change.pos, end);
            self.rope.remove(change.pos..end);
            self.rope.insert(change.pos, &change.removed);
            self.finish_edit(change.pos, &change.removed, captured);
        }
        self.selection = tx.before;
        self.target_col = None;
        self.history.redo.push(tx);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.history.seal();
        let Some(tx) = self.history.redo.pop() else {
            return false;
        };
        for change in &tx.changes {
            let end = change.pos + change.removed.chars().count();
            let captured = self.capture_edit(change.pos, end);
            self.rope.remove(change.pos..end);
            self.rope.insert(change.pos, &change.inserted);
            self.finish_edit(change.pos, &change.inserted, captured);
        }
        self.selection = tx.after;
        self.target_col = None;
        self.history.undo.push(tx);
        true
    }

    fn apply_edit(&mut self, from: usize, to: usize, inserted: &str, kind: EditKind) {
        if from == to && inserted.is_empty() {
            return;
        }
        let before = self.selection;
        let removed = self.rope.slice(from..to).to_string();
        let captured = self.capture_edit(from, to);
        self.rope.remove(from..to);
        self.rope.insert(from, inserted);
        self.finish_edit(from, inserted, captured);

        let after = Selection::point(from + inserted.chars().count());
        self.selection = after;
        self.target_col = None;
        self.history.record(
            Change {
                pos: from,
                removed,
                inserted: inserted.to_string(),
            },
            before,
            after,
            kind,
        );
    }

    /// Applies `(pos, removed_len, inserted)` edits as one undo step.
    /// Edits must be sorted by `pos`, non-overlapping, and in pre-edit coordinates.
    fn apply_batch(&mut self, edits: Vec<(usize, usize, String)>) {
        self.apply_batch_cursor(edits, None, false);
    }

    /// `cursor_at` is a pre-edit char index that becomes a caret after the edits.
    /// Without it, the previous selection is mapped through the edits.
    /// `after_empty` puts that caret after a zero-width insert at the same index.
    fn apply_batch_cursor(
        &mut self,
        edits: Vec<(usize, usize, String)>,
        cursor_at: Option<usize>,
        after_empty: bool,
    ) {
        if edits.is_empty() {
            return;
        }
        let before = self.selection;
        let shape: Vec<(usize, usize, usize)> = edits
            .iter()
            .map(|(pos, removed, inserted)| (*pos, *removed, inserted.chars().count()))
            .collect();

        let mut changes = Vec::with_capacity(edits.len());
        for (pos, removed_len, inserted) in edits.into_iter().rev() {
            let removed = self.rope.slice(pos..pos + removed_len).to_string();
            let captured = self.capture_edit(pos, pos + removed_len);
            self.rope.remove(pos..pos + removed_len);
            self.rope.insert(pos, &inserted);
            self.finish_edit(pos, &inserted, captured);
            changes.push(Change {
                pos,
                removed,
                inserted,
            });
        }

        let after = cursor_at.map_or_else(
            || Selection {
                anchor: map_through_edits(before.anchor, &shape),
                head: map_through_edits(before.head, &shape),
            },
            |pos| Selection::point(caret_after_batch(pos, &shape, after_empty)),
        );
        self.selection = after;
        self.target_col = None;
        self.history.push(changes, before, after, EditKind::Other);
    }

    fn capture_edit(&self, from: usize, to: usize) -> (usize, usize, Point, Point) {
        (
            self.rope.char_to_byte(from),
            self.rope.char_to_byte(to),
            point_at(&self.rope, from),
            point_at(&self.rope, to),
        )
    }

    fn finish_edit(
        &mut self,
        from: usize,
        inserted: &str,
        (start_byte, old_end_byte, start, old_end): (usize, usize, Point, Point),
    ) {
        let new_char = from + inserted.chars().count();
        self.revision = self.revision.wrapping_add(1);
        self.pending_edits.push(ByteEdit {
            start_byte,
            old_end_byte,
            new_end_byte: self.rope.char_to_byte(new_char),
            start,
            old_end,
            new_end: point_at(&self.rope, new_char),
        });
    }

    /// Lines touched by the selection; a selection ending at column 0 doesn't include that line.
    fn selected_lines(&self) -> (usize, usize) {
        let sel = self.selection;
        let first = self.rope.char_to_line(sel.from());
        let mut last = self.rope.char_to_line(sel.to());
        if last > first && sel.to() == self.line_start(last) {
            last -= 1;
        }
        (first, last)
    }

    // ---- Position helpers --------------------------------------------------

    fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    fn line_content_len(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        let mut end = slice.len_chars();
        if end > 0 && slice.char(end - 1) == '\n' {
            end -= 1;
        }
        if end > 0 && slice.char(end - 1) == '\r' {
            end -= 1;
        }
        end
    }

    fn line_end(&self, line: usize) -> usize {
        self.line_start(line) + self.line_content_len(line)
    }

    fn left_of(&self, pos: usize) -> usize {
        let line = self.rope.char_to_line(pos);
        let start = self.line_start(line);
        if pos > start {
            start + prev_grapheme_col(&self.line_content(line), pos - start)
        } else if line > 0 {
            self.line_end(line - 1)
        } else {
            0
        }
    }

    fn right_of(&self, pos: usize) -> usize {
        let line = self.rope.char_to_line(pos);
        let start = self.line_start(line);
        if pos < self.line_end(line) {
            start + next_grapheme_col(&self.line_content(line), pos - start)
        } else if line + 1 < self.rope.len_lines() {
            self.line_start(line + 1)
        } else {
            pos
        }
    }

    fn word_right_of(&self, pos: usize) -> usize {
        let line = self.rope.char_to_line(pos);
        let start = self.line_start(line);
        let spans = grapheme_spans(&self.line_content(line));
        let col = pos - start;

        let Some(mut i) = spans.iter().position(|s| s.0 >= col) else {
            return self.right_of(pos);
        };
        while i < spans.len() && spans[i].2 == CharClass::Whitespace {
            i += 1;
        }
        if i < spans.len() {
            let class = spans[i].2;
            while i < spans.len() && spans[i].2 == class {
                i += 1;
            }
        }
        start + spans[i - 1].1
    }

    fn word_left_of(&self, pos: usize) -> usize {
        let line = self.rope.char_to_line(pos);
        let start = self.line_start(line);
        let spans = grapheme_spans(&self.line_content(line));
        let col = pos - start;

        let mut i = spans.iter().take_while(|s| s.1 <= col).count();
        if i == 0 {
            return self.left_of(pos);
        }
        while i > 0 && spans[i - 1].2 == CharClass::Whitespace {
            i -= 1;
        }
        if i > 0 {
            let class = spans[i - 1].2;
            while i > 0 && spans[i - 1].2 == class {
                i -= 1;
            }
        }
        start + if i == 0 { 0 } else { spans[i].0 }
    }

    /// First `Home` press jumps to the first non-whitespace char; a second press goes to column 0.
    fn smart_line_start(&self, pos: usize) -> usize {
        let line = self.rope.char_to_line(pos);
        let start = self.line_start(line);
        let indent = self
            .line_content(line)
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        let first_non_ws = start + indent;
        if pos == first_non_ws {
            start
        } else {
            first_non_ws
        }
    }

    fn vertical(&mut self, pos: usize, delta: isize) -> usize {
        let line = self.rope.char_to_line(pos);
        let start = self.line_start(line);
        let text = self.line_content(line);
        let target = *self
            .target_col
            .get_or_insert_with(|| visual_col(&text, pos - start, self.tab_width));

        let last_line = self.rope.len_lines() - 1;
        let new_line = (line as isize + delta).clamp(0, last_line as isize) as usize;
        if new_line == line {
            return if delta < 0 {
                start
            } else {
                self.line_end(line)
            };
        }
        self.line_start(new_line)
            + col_at_visual(&self.line_content(new_line), target, self.tab_width)
    }
}

/// Maps a pre-edit position through sorted `(pos, removed, inserted)` edits.
/// Positions at an edit's start stay put; positions inside a removed range snap to its start.
/// A caret on a zero-width insert belongs after the new text. Every other index
/// uses [`map_through_edits`], which leaves a caret at an edit's start where it was.
fn caret_after_batch(pos: usize, edits: &[(usize, usize, usize)], after_empty: bool) -> usize {
    if !after_empty {
        return map_through_edits(pos, edits);
    }
    let mut delta = 0isize;
    for &(edit_pos, removed, inserted) in edits {
        if pos < edit_pos {
            break;
        }
        if pos == edit_pos && removed == 0 {
            return (edit_pos as isize + delta) as usize + inserted;
        }
        if pos < edit_pos + removed {
            break;
        }
        delta += inserted as isize - removed as isize;
    }
    map_through_edits(pos, edits)
}

fn map_through_edits(p: usize, edits: &[(usize, usize, usize)]) -> usize {
    let mut delta = 0isize;
    for &(pos, removed, inserted) in edits {
        if p <= pos {
            break;
        }
        if p < pos + removed {
            return (pos as isize + delta) as usize;
        }
        delta += inserted as isize - removed as isize;
    }
    (p as isize + delta) as usize
}

// ---- Grapheme & width helpers ----------------------------------------------

/// The bracket under the cursor, or the one immediately before it.
/// `<>` is included so a syntax tree can accept it; a plain scan ignores those.
pub fn bracket_near(rope: &Rope, head: usize) -> Option<(usize, char)> {
    if head < rope.len_chars() {
        let ch = rope.char(head);
        if is_bracket_char(ch) {
            return Some((head, ch));
        }
    }
    if head > 0 {
        let ch = rope.char(head - 1);
        if is_bracket_char(ch) {
            return Some((head - 1, ch));
        }
    }
    None
}

fn is_bracket_char(ch: char) -> bool {
    matches!(ch, '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>')
}

/// Pair of `bracket` at char index `at`, counting nested `()[]{}`.
/// `<>` is not paired. A closer that crosses a different pair returns nothing.
pub fn scan_bracket(rope: &Rope, at: usize, bracket: char) -> Option<usize> {
    if let Some(close) = scan_closer(bracket) {
        scan_forward(rope, at, close)
    } else if let Some(open) = scan_opener(bracket) {
        scan_backward(rope, at, open)
    } else {
        None
    }
}

fn scan_closer(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn scan_opener(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

fn scan_forward(rope: &Rope, at: usize, close: char) -> Option<usize> {
    let mut stack = vec![close];
    let mut index = at + 1;
    let mut chars = rope.chars_at(index);
    while let Some(ch) = chars.next() {
        if let Some(next_close) = scan_closer(ch) {
            stack.push(next_close);
        } else if scan_opener(ch).is_some() {
            if stack.last() != Some(&ch) {
                return None;
            }
            stack.pop();
            if stack.is_empty() {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

fn scan_backward(rope: &Rope, at: usize, open: char) -> Option<usize> {
    let mut stack = vec![open];
    let mut index = at;
    let mut chars = rope.chars_at(at);
    while let Some(ch) = chars.prev() {
        index -= 1;
        if let Some(next_open) = scan_opener(ch) {
            stack.push(next_open);
        } else if scan_closer(ch).is_some() {
            if stack.last() != Some(&ch) {
                return None;
            }
            stack.pop();
            if stack.is_empty() {
                return Some(index);
            }
        }
    }
    None
}

fn char_to_byte(text: &str, col: usize) -> usize {
    text.char_indices()
        .nth(col)
        .map_or(text.len(), |(byte, _)| byte)
}

fn byte_to_char(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

fn prev_grapheme_col(text: &str, col: usize) -> usize {
    let byte = char_to_byte(text, col);
    let prev = text
        .grapheme_indices(true)
        .map(|(b, _)| b)
        .take_while(|b| *b < byte)
        .last()
        .unwrap_or(0);
    byte_to_char(text, prev)
}

fn next_grapheme_col(text: &str, col: usize) -> usize {
    let byte = char_to_byte(text, col);
    text.grapheme_indices(true)
        .map(|(b, g)| b + g.len())
        .find(|end| *end > byte)
        .map_or_else(|| text.chars().count(), |end| byte_to_char(text, end))
}

/// `(start_col, end_col, class)` for each grapheme, in char columns.
fn grapheme_spans(text: &str) -> Vec<(usize, usize, CharClass)> {
    let mut col = 0;
    text.graphemes(true)
        .map(|g| {
            let len = g.chars().count();
            let span = (col, col + len, CharClass::of(g));
            col += len;
            span
        })
        .collect()
}

/// Display width of `grapheme` when drawn at visual column `current_col` (tabs expand to the next stop).
fn point_at(rope: &Rope, char_idx: usize) -> Point {
    let row = rope.char_to_line(char_idx);
    let line_start = rope.line_to_char(row);
    Point {
        row,
        column: rope.char_to_byte(char_idx) - rope.char_to_byte(line_start),
    }
}

pub fn grapheme_width(grapheme: &str, current_col: usize, tab_width: usize) -> usize {
    if grapheme == "\t" {
        let tab_width = tab_width.max(1);
        tab_width - current_col % tab_width
    } else {
        grapheme.width()
    }
}

/// Terminal display column of char column `col` within `text`.
pub fn visual_col(text: &str, col: usize, tab_width: usize) -> usize {
    let mut chars = 0;
    let mut width = 0;
    for g in text.graphemes(true) {
        if chars >= col {
            break;
        }
        chars += g.chars().count();
        width += grapheme_width(g, width, tab_width);
    }
    width
}

/// Char column whose display position is closest to (but not past) `target`.
fn col_at_visual(text: &str, target: usize, tab_width: usize) -> usize {
    let mut chars = 0;
    let mut width = 0;
    for g in text.graphemes(true) {
        let w = grapheme_width(g, width, tab_width);
        if width + w > target {
            break;
        }
        width += w;
        chars += g.chars().count();
    }
    chars
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_str(buf: &mut Buffer, text: &str) {
        for c in text.chars() {
            buf.insert_char(c);
        }
    }

    fn text(buf: &Buffer) -> String {
        buf.rope().to_string()
    }

    fn head(buf: &Buffer) -> usize {
        buf.selection().head
    }

    #[test]
    fn typing_coalesces_into_one_undo_step() {
        let mut buf = Buffer::new();
        type_str(&mut buf, "hello");
        assert_eq!(text(&buf), "hello");
        assert_eq!(head(&buf), 5);

        assert!(buf.undo());
        assert_eq!(text(&buf), "");
        assert!(buf.redo());
        assert_eq!(text(&buf), "hello");
        assert_eq!(head(&buf), 5);
    }

    #[test]
    fn movement_breaks_typing_coalescing() {
        let mut buf = Buffer::new();
        type_str(&mut buf, "ab");
        buf.move_cursor(Movement::Left, false);
        type_str(&mut buf, "c");
        assert_eq!(text(&buf), "acb");

        buf.undo();
        assert_eq!(text(&buf), "ab");
        buf.undo();
        assert_eq!(text(&buf), "");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut buf = Buffer::new();
        type_str(&mut buf, "a");
        buf.undo();
        type_str(&mut buf, "b");
        assert!(!buf.redo());
        assert_eq!(text(&buf), "b");
    }

    #[test]
    fn shift_extends_and_plain_arrow_collapses() {
        let mut buf = Buffer::from_text("hello world");
        for _ in 0..5 {
            buf.move_cursor(Movement::Right, true);
        }
        assert_eq!(buf.selection(), Selection { anchor: 0, head: 5 });
        assert_eq!(buf.selected_text(), "hello");

        buf.move_cursor(Movement::LineEnd, true);
        assert_eq!(
            buf.selection(),
            Selection {
                anchor: 0,
                head: 11
            }
        );

        buf.move_cursor(Movement::Left, false);
        assert_eq!(buf.selection(), Selection::point(0));
    }

    #[test]
    fn typing_replaces_selection_and_undo_restores_it() {
        let mut buf = Buffer::from_text("hello world");
        for _ in 0..5 {
            buf.move_cursor(Movement::Right, true);
        }
        type_str(&mut buf, "XY");
        assert_eq!(text(&buf), "XY world");

        buf.undo();
        assert_eq!(text(&buf), "hello world");
        assert_eq!(buf.selection(), Selection { anchor: 0, head: 5 });
    }

    #[test]
    fn moves_and_deletes_by_grapheme() {
        let mut buf = Buffer::from_text("e\u{301}x");
        buf.move_cursor(Movement::Right, false);
        assert_eq!(head(&buf), 2);
        buf.delete_backward();
        assert_eq!(text(&buf), "x");

        let mut buf = Buffer::from_text("👍🏽a");
        buf.move_cursor(Movement::Right, false);
        assert_eq!(head(&buf), 2);
    }

    #[test]
    fn vertical_movement_keeps_visual_column_across_wide_chars() {
        let mut buf = Buffer::from_text("abcdef\n日本\nabcdef");
        for _ in 0..4 {
            buf.move_cursor(Movement::Right, false);
        }
        buf.move_cursor(Movement::Down, false);
        assert_eq!(head(&buf), 9);
        assert_eq!(buf.cursor_line_col(), (1, 4));

        buf.move_cursor(Movement::Down, false);
        assert_eq!(head(&buf), 14);
    }

    #[test]
    fn vertical_movement_at_document_edges() {
        let mut buf = Buffer::from_text("abc\ndef");
        buf.move_cursor(Movement::Right, false);
        buf.move_cursor(Movement::Up, false);
        assert_eq!(head(&buf), 0);

        buf.move_cursor(Movement::Down, false);
        buf.move_cursor(Movement::Down, false);
        assert_eq!(head(&buf), 7);
    }

    #[test]
    fn page_movement_moves_multiple_lines() {
        let lines: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
        let mut buf = Buffer::from_text(&lines.join("\n"));
        buf.move_cursor(Movement::PageDown(3), false);
        assert_eq!(buf.cursor_line_col().0, 3);
        buf.move_cursor(Movement::PageUp(5), false);
        assert_eq!(buf.cursor_line_col().0, 0);
    }

    #[test]
    fn smart_home_toggles_between_indent_and_column_zero() {
        let mut buf = Buffer::from_text("    let x");
        buf.move_cursor(Movement::LineEnd, false);
        buf.move_cursor(Movement::LineStart, false);
        assert_eq!(head(&buf), 4);
        buf.move_cursor(Movement::LineStart, false);
        assert_eq!(head(&buf), 0);
        buf.move_cursor(Movement::LineStart, false);
        assert_eq!(head(&buf), 4);
    }

    #[test]
    fn word_movement_stops_at_class_boundaries() {
        let mut buf = Buffer::from_text("foo.bar  baz");
        let mut stops = Vec::new();
        for _ in 0..4 {
            buf.move_cursor(Movement::WordRight, false);
            stops.push(head(&buf));
        }
        assert_eq!(stops, [3, 4, 7, 12]);

        stops.clear();
        for _ in 0..4 {
            buf.move_cursor(Movement::WordLeft, false);
            stops.push(head(&buf));
        }
        assert_eq!(stops, [9, 4, 3, 0]);
    }

    #[test]
    fn crlf_line_ending_is_treated_as_one_unit() {
        let mut buf = Buffer::from_text("ab\r\ncd");
        assert_eq!(buf.line_ending(), LineEnding::Crlf);
        assert_eq!(buf.line_content(0), "ab");

        buf.move_cursor(Movement::DocumentEnd, false);
        buf.move_cursor(Movement::LineStart, false);
        assert_eq!(head(&buf), 4);
        buf.move_cursor(Movement::Left, false);
        assert_eq!(head(&buf), 2);

        buf.move_cursor(Movement::Right, false);
        buf.delete_backward();
        assert_eq!(text(&buf), "abcd");
        assert_eq!(head(&buf), 2);

        buf.insert_newline();
        assert_eq!(text(&buf), "ab\r\ncd");
    }

    #[test]
    fn newline_carries_indentation() {
        let mut buf = Buffer::from_text("    foo");
        buf.move_cursor(Movement::LineEnd, false);
        buf.insert_newline();
        assert_eq!(text(&buf), "    foo\n    ");
        assert_eq!(head(&buf), 12);
    }

    #[test]
    fn place_cursor_keeps_the_line_and_clamps_the_column() {
        let mut buf = Buffer::from_text("alpha\nbeta");
        buf.place_cursor(1, 2);
        assert_eq!(buf.cursor_line_char(), (1, 2));
        buf.place_cursor(9, 40);
        assert_eq!(buf.cursor_line_char(), (1, 4));
    }

    #[test]
    fn pasted_text_uses_buffer_line_ending() {
        let mut buf = Buffer::from_text("x\r\n");
        buf.move_cursor(Movement::DocumentEnd, false);
        buf.insert_text("a\nb");
        assert_eq!(text(&buf), "x\r\na\r\nb");
    }

    #[test]
    fn tab_inserts_spaces_to_next_stop() {
        let mut buf = Buffer::new();
        buf.indent();
        assert_eq!(text(&buf), "    ");

        let mut buf = Buffer::from_text("ab");
        buf.move_cursor(Movement::LineEnd, false);
        buf.indent();
        assert_eq!(text(&buf), "ab  ");
        assert_eq!(head(&buf), 4);
    }

    #[test]
    fn tab_inserts_spaces_even_when_the_file_uses_tabs() {
        let mut buf = Buffer::from_text("\tfoo\n\tbar\n");
        assert_eq!(buf.indent_style(), IndentStyle::Spaces(4));
        buf.move_cursor(Movement::DocumentEnd, false);
        buf.indent();
        assert_eq!(text(&buf), "\tfoo\n\tbar\n    ");
    }

    #[test]
    fn tab_inserts_a_tab_when_spaces_are_off() {
        let mut buf = Buffer::from_text("ab");
        buf.apply_editor_style(false, 4, None);
        buf.move_cursor(Movement::LineEnd, false);
        buf.indent();
        assert_eq!(text(&buf), "ab\t");
    }

    #[test]
    fn save_trims_trailing_space_and_adds_a_final_newline() {
        let mut buf = Buffer::from_text("a  \nb\t");
        buf.prepare_for_save(Some(LineEnding::Lf), true, true);
        assert_eq!(text(&buf), "a\nb\n");

        let mut crlf = Buffer::from_text("a\nb");
        crlf.prepare_for_save(Some(LineEnding::Crlf), false, true);
        assert_eq!(text(&crlf), "a\r\nb\r\n");
        assert_eq!(crlf.line_ending(), LineEnding::Crlf);
    }

    #[test]
    fn tab_indents_selected_lines_as_one_undo_step() {
        let mut buf = Buffer::from_text("a\n\nb\nc");
        buf.move_cursor(Movement::Down, true);
        buf.move_cursor(Movement::Down, true);
        buf.move_cursor(Movement::Down, true);
        buf.indent();
        assert_eq!(text(&buf), "    a\n\n    b\nc");
        assert_eq!(
            buf.selection(),
            Selection {
                anchor: 0,
                head: 13
            }
        );

        buf.undo();
        assert_eq!(text(&buf), "a\n\nb\nc");
        assert_eq!(buf.selection(), Selection { anchor: 0, head: 5 });
    }

    #[test]
    fn shift_tab_outdents_to_previous_stop() {
        let mut buf = Buffer::from_text("      x\n\ty");
        buf.move_cursor(Movement::LineEnd, false);
        buf.outdent();
        assert_eq!(text(&buf), "    x\n\ty");
        assert_eq!(head(&buf), 5);
        buf.outdent();
        assert_eq!(text(&buf), "x\n\ty");
        buf.outdent();
        assert_eq!(text(&buf), "x\n\ty");

        buf.select_all();
        buf.outdent();
        assert_eq!(text(&buf), "x\ny");
    }

    #[test]
    fn backspace_in_leading_spaces_removes_one_indent_level() {
        let mut buf = Buffer::from_text("          x");
        for _ in 0..10 {
            buf.move_cursor(Movement::Right, false);
        }
        buf.delete_backward();
        assert_eq!(text(&buf), "        x");
        buf.delete_backward();
        assert_eq!(text(&buf), "    x");

        let mut buf = Buffer::from_text("a   ");
        buf.move_cursor(Movement::LineEnd, false);
        buf.delete_backward();
        assert_eq!(text(&buf), "a  ");
    }

    #[test]
    fn copy_text_is_selection_or_whole_line() {
        let mut buf = Buffer::from_text("ab\r\ncd");
        assert_eq!(buf.copy_text(), "ab\r\n");
        buf.move_cursor(Movement::DocumentEnd, false);
        assert_eq!(buf.copy_text(), "cd\r\n");
        buf.move_cursor(Movement::Left, true);
        assert_eq!(buf.copy_text(), "d");
    }

    #[test]
    fn fingerprint_matches_the_bytes_on_disk() {
        for text in ["", "a", "ab\r\ncd", "สวัสดี\n", "a\n"] {
            let buf = Buffer::from_text(text);
            assert_eq!(
                buf.fingerprint(),
                fingerprint_bytes(buf.rope().to_string().as_bytes()),
                "{text:?}"
            );
        }
    }

    #[test]
    fn cut_removes_the_line_or_selection_in_one_step() {
        let mut buf = Buffer::from_text("a\nb\nc");
        buf.move_cursor(Movement::Down, false);
        assert_eq!(buf.cut_text(), "b\n");
        assert_eq!(text(&buf), "a\nc");
        buf.undo();
        assert_eq!(text(&buf), "a\nb\nc");

        let mut buf = Buffer::from_text("ab\ncd");
        buf.move_cursor(Movement::DocumentEnd, false);
        assert_eq!(buf.cut_text(), "cd\n");
        assert_eq!(text(&buf), "ab");

        let mut buf = Buffer::from_text("hello");
        buf.move_cursor(Movement::Right, true);
        assert_eq!(buf.cut_text(), "h");
        assert_eq!(text(&buf), "ello");
    }

    #[test]
    fn find_spans_are_literal_non_overlapping_and_case_sensitive() {
        let buf = Buffer::from_text("aaa aa Aa คำค้นคำ");
        assert!(buf.find_spans("").is_empty());
        assert_eq!(buf.find_spans("aa"), vec![(0, 2), (4, 6)]);
        assert!(buf.find_spans("AA").is_empty());
        assert_eq!(buf.find_spans("."), Vec::<(usize, usize)>::new());
        assert_eq!(buf.find_spans("คำ"), vec![(10, 12), (15, 17)]);
        let crlf = Buffer::from_text("ab\r\ncd ab");
        assert_eq!(crlf.find_spans("ab"), vec![(0, 2), (7, 9)]);
        assert_eq!(Buffer::literal_match_count("aaa", "aa"), 1);
        assert_eq!(Buffer::literal_match_count("Cat", "cat"), 0);
    }

    #[test]
    fn replace_span_and_replace_all_are_one_undo_step() {
        let mut buf = Buffer::from_text("a-a-a");
        buf.select_span(0, 1);
        assert_eq!(buf.selected_text(), "a");
        assert_eq!(buf.selection().head, 0);

        buf.replace_span(2, 3, "bb");
        assert_eq!(text(&buf), "a-bb-a");
        buf.undo();
        assert_eq!(text(&buf), "a-a-a");
        assert_eq!(buf.selected_text(), "a");

        assert_eq!(buf.replace_all("a", "aa"), 3);
        assert_eq!(text(&buf), "aa-aa-aa");
        assert_eq!(head(&buf), 0);
        buf.undo();
        assert_eq!(text(&buf), "a-a-a");
        assert_eq!(buf.selected_text(), "a");
        assert!(!buf.undo());
    }

    #[test]
    fn replace_all_does_not_search_inside_the_replacement() {
        let mut buf = Buffer::from_text("a");
        assert_eq!(buf.replace_all("a", "aa"), 1);
        assert_eq!(text(&buf), "aa");
        assert_eq!(buf.replace_all("z", "q"), 0);
        assert_eq!(text(&buf), "aa");
    }

    #[test]
    fn select_all_then_delete_clears_buffer() {
        let mut buf = Buffer::from_text("one\ntwo");
        buf.select_all();
        assert_eq!(buf.selected_text(), "one\ntwo");
        buf.delete_forward();
        assert_eq!(text(&buf), "");
    }

    #[test]
    fn byte_edit_for_an_ascii_insert() {
        let mut buf = Buffer::from_text("ac");
        buf.move_cursor(Movement::Right, false);
        buf.insert_text("b");
        let (revision, edits) = buf.take_edits();
        assert_eq!(revision, 1);
        assert_eq!(
            edits,
            vec![ByteEdit {
                start_byte: 1,
                old_end_byte: 1,
                new_end_byte: 2,
                start: Point { row: 0, column: 1 },
                old_end: Point { row: 0, column: 1 },
                new_end: Point { row: 0, column: 2 },
            }]
        );
        assert!(buf.take_edits().1.is_empty());
        assert_eq!(buf.revision(), 1);
    }

    #[test]
    fn byte_edit_counts_utf8_bytes_for_thai_and_emoji() {
        let mut buf = Buffer::from_text("a");
        buf.move_cursor(Movement::DocumentEnd, false);
        buf.insert_text("คำ👍🏽");
        let (_, edits) = buf.take_edits();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].start_byte, 1);
        assert_eq!(edits[0].old_end_byte, 1);
        assert_eq!(edits[0].new_end_byte, 1 + "คำ👍🏽".len());
        assert_eq!(edits[0].start, Point { row: 0, column: 1 });
        assert_eq!(
            edits[0].new_end,
            Point {
                row: 0,
                column: 1 + "คำ👍🏽".len(),
            }
        );
    }

    #[test]
    fn byte_edit_for_a_crlf_line_break_removal() {
        let mut buf = Buffer::from_text("ab\r\ncd");
        buf.move_cursor(Movement::DocumentEnd, false);
        buf.move_cursor(Movement::LineStart, false);
        buf.delete_backward();
        let (_, edits) = buf.take_edits();
        assert_eq!(text(&buf), "abcd");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].start_byte, 2);
        assert_eq!(edits[0].old_end_byte, 4);
        assert_eq!(edits[0].new_end_byte, 2);
        assert_eq!(edits[0].old_end, Point { row: 1, column: 0 });
        assert_eq!(edits[0].new_end, Point { row: 0, column: 2 });
    }

    #[test]
    fn byte_edits_for_a_batch_are_in_application_order() {
        let mut buf = Buffer::from_text("a-a");
        assert_eq!(buf.replace_all("a", "bb"), 2);
        let (revision, edits) = buf.take_edits();
        assert_eq!(text(&buf), "bb-bb");
        assert_eq!(revision, 2);
        assert_eq!(edits[0].start_byte, 2);
        assert_eq!(edits[0].old_end_byte, 3);
        assert_eq!(edits[0].new_end_byte, 4);
        assert_eq!(edits[1].start_byte, 0);
        assert_eq!(edits[1].old_end_byte, 1);
        assert_eq!(edits[1].new_end_byte, 2);
    }

    #[test]
    fn undo_and_redo_record_the_inverse_byte_edit() {
        let mut buf = Buffer::from_text("ab");
        buf.move_cursor(Movement::DocumentEnd, false);
        buf.insert_text("X");
        let _ = buf.take_edits();
        assert!(buf.undo());
        let (_, undone) = buf.take_edits();
        assert_eq!(text(&buf), "ab");
        assert_eq!(undone[0].start_byte, 2);
        assert_eq!(undone[0].old_end_byte, 3);
        assert_eq!(undone[0].new_end_byte, 2);
        assert!(buf.redo());
        let (_, redone) = buf.take_edits();
        assert_eq!(text(&buf), "abX");
        assert_eq!(redone[0].start_byte, 2);
        assert_eq!(redone[0].old_end_byte, 2);
        assert_eq!(redone[0].new_end_byte, 3);
    }

    #[test]
    fn moving_a_line_swaps_with_its_neighbor_and_stops_at_the_edge() {
        let mut buf = Buffer::from_text("a\nb\nc\n");
        assert!(buf.move_block(true));
        assert_eq!(text(&buf), "b\na\nc\n");
        assert!(buf.move_block(false));
        assert_eq!(text(&buf), "a\nb\nc\n");
        buf.move_cursor(Movement::DocumentStart, false);
        assert!(!buf.move_block(false));
        buf.move_cursor(Movement::DocumentEnd, false);
        assert!(!buf.move_block(true));
        assert!(buf.undo());
        assert_eq!(text(&buf), "b\na\nc\n");
        assert!(buf.undo());
        assert_eq!(text(&buf), "a\nb\nc\n");
    }

    #[test]
    fn delete_lines_removes_the_caret_line_or_the_selected_lines() {
        let mut buf = Buffer::from_text("a\nb\nc\n");
        buf.move_cursor(Movement::Down, false);
        buf.delete_lines();
        assert_eq!(text(&buf), "a\nc\n");
        assert_eq!(buf.cursor_line_col(), (1, 0));

        let mut buf = Buffer::from_text("a\nb\nc");
        buf.move_cursor(Movement::DocumentEnd, false);
        buf.delete_lines();
        assert_eq!(text(&buf), "a\nb");
        assert_eq!(buf.cursor_line_col(), (1, 1));

        let mut buf = Buffer::from_text("a\nb\nc\n");
        buf.move_cursor(Movement::Down, false);
        buf.move_cursor(Movement::Down, true);
        buf.delete_lines();
        assert_eq!(text(&buf), "a\nc\n");

        let mut buf = Buffer::from_text("only");
        buf.delete_lines();
        assert_eq!(text(&buf), "");
        assert!(buf.undo());
        assert_eq!(text(&buf), "only");
    }

    #[test]
    fn duplicate_block_copies_the_line_or_selection_and_follows_it() {
        let mut buf = Buffer::from_text("alpha\nbeta\ngamma\n");
        buf.move_cursor(Movement::Down, false);
        buf.move_cursor(Movement::Right, false);
        buf.move_cursor(Movement::Right, false);
        buf.duplicate_block(true);
        assert_eq!(text(&buf), "alpha\nbeta\nbeta\ngamma\n");
        assert_eq!(buf.cursor_line_col(), (2, 2));
        buf.duplicate_block(false);
        assert_eq!(text(&buf), "alpha\nbeta\nbeta\nbeta\ngamma\n");
        assert_eq!(buf.cursor_line_col(), (2, 2));
        assert!(buf.undo());
        assert_eq!(text(&buf), "alpha\nbeta\nbeta\ngamma\n");
        assert!(buf.redo());
        assert_eq!(buf.cursor_line_col(), (2, 2));

        let mut buf = Buffer::from_text("alpha\nbeta\ngamma\n");
        buf.move_cursor(Movement::Down, false);
        buf.move_cursor(Movement::Right, false);
        buf.move_cursor(Movement::Right, false);
        buf.move_cursor(Movement::Down, true);
        buf.duplicate_block(true);
        assert_eq!(text(&buf), "alpha\nbeta\ngamma\nbeta\ngamma\n");
        assert_eq!(buf.selected_text(), "ta\nga");
        assert_eq!(buf.cursor_line_col().0, 4);

        let mut buf = Buffer::from_text("tail");
        buf.move_cursor(Movement::Right, false);
        buf.duplicate_block(true);
        assert_eq!(text(&buf), "tail\ntail");
        assert_eq!(buf.cursor_line_col(), (1, 1));
        buf.duplicate_block(false);
        assert_eq!(text(&buf), "tail\ntail\ntail");
        assert_eq!(buf.cursor_line_col(), (1, 1));
    }

    #[test]
    fn scan_bracket_pairs_nested_and_stops_on_a_cross() {
        let rope = Rope::from_str("f(a)");
        assert_eq!(bracket_near(&rope, 1), Some((1, '(')));
        assert_eq!(scan_bracket(&rope, 1, '('), Some(3));
        assert_eq!(bracket_near(&rope, 2), Some((1, '(')));
        assert_eq!(scan_bracket(&rope, 3, ')'), Some(1));
        assert_eq!(bracket_near(&rope, 4), Some((3, ')')));
        assert_eq!(bracket_near(&rope, 0), None);

        let nested = Rope::from_str("{{}");
        assert_eq!(scan_bracket(&nested, 1, '{'), Some(2));
        assert_eq!(scan_bracket(&nested, 0, '{'), None);
        assert_eq!(scan_bracket(&nested, 2, '}'), Some(1));

        let crossed = Rope::from_str("([)]");
        assert_eq!(scan_bracket(&crossed, 0, '('), None);
        assert_eq!(scan_bracket(&Rope::from_str("a < b"), 2, '<'), None);

        let mut buf = Buffer::from_text("f(a)");
        buf.move_to(1, false);
        buf.move_to(3, true);
        assert_eq!(buf.selection(), Selection { anchor: 1, head: 3 });
        buf.move_to(1, false);
        assert!(buf.selection().is_empty());
        assert_eq!(buf.selection().head, 1);
    }

    #[test]
    fn toggle_comment_adds_and_removes_one_marker() {
        let mut buf = Buffer::from_text("  let x = 1;\n");
        assert!(buf.toggle_comment("//"));
        assert_eq!(text(&buf), "  // let x = 1;\n");
        assert!(buf.toggle_comment("//"));
        assert_eq!(text(&buf), "  let x = 1;\n");
    }
}
