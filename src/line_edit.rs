//! Cursor stops for one-line text inputs (the command bar, the file picker, and
//! the text rows in Settings).
//!
//! Positions are byte offsets into the text and always sit on a grapheme
//! boundary, so a Thai base letter and its combining marks move and delete
//! as one. Word jumps use the editor's rule: skip whitespace, then one run of
//! word characters or one run of punctuation.

use unicode_segmentation::UnicodeSegmentation;

use crate::buffer::CharClass;

/// Start of the grapheme before `at`, or 0.
pub fn prev(text: &str, at: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(byte, _)| byte)
        .take_while(|byte| *byte < at)
        .last()
        .unwrap_or(0)
}

/// End of the grapheme that starts at or covers `at`, or the text length.
pub fn next(text: &str, at: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(byte, grapheme)| byte + grapheme.len())
        .find(|end| *end > at)
        .unwrap_or(text.len())
}

/// `at` moved back onto a grapheme boundary (the start of the grapheme it is inside).
pub fn snap(text: &str, at: usize) -> usize {
    if at >= text.len() {
        return text.len();
    }
    text.grapheme_indices(true)
        .map(|(byte, _)| byte)
        .take_while(|byte| *byte <= at)
        .last()
        .unwrap_or(0)
}

fn spans(text: &str) -> Vec<(usize, usize, CharClass)> {
    text.grapheme_indices(true)
        .map(|(byte, grapheme)| (byte, byte + grapheme.len(), CharClass::of(grapheme)))
        .collect()
}

/// `Ctrl+Right`: past the whitespace, then past one word or punctuation run.
pub fn word_right(text: &str, at: usize) -> usize {
    let spans = spans(text);
    let Some(mut i) = spans.iter().position(|span| span.0 >= at) else {
        return text.len();
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
    if i == 0 { 0 } else { spans[i - 1].1 }
}

/// `Ctrl+Left`: back over the whitespace, then back over one word or punctuation run.
pub fn word_left(text: &str, at: usize) -> usize {
    let spans = spans(text);
    let mut i = spans.iter().take_while(|span| span.1 <= at).count();
    while i > 0 && spans[i - 1].2 == CharClass::Whitespace {
        i -= 1;
    }
    if i > 0 {
        let class = spans[i - 1].2;
        while i > 0 && spans[i - 1].2 == class {
            i -= 1;
        }
    }
    if i == 0 { 0 } else { spans[i].0 }
}

/// Byte offset where a file name's extension starts (`index|.html`), for a rename
/// prefill. `None` for a name with no extension or a dot file such as `.gitignore`.
pub fn stem_end(name: &str) -> Option<usize> {
    name.rfind('.').filter(|dot| *dot > 0)
}

/// A one-line text input with a cursor: the shared editor behind Settings text rows.
/// Same keys and the same grapheme and word stops as the command bar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    text: String,
    /// Byte offset, always on a grapheme boundary.
    cursor: usize,
}

impl LineInput {
    /// The cursor starts at the end of `text`.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        snap(&self.text, self.cursor)
    }

    /// Replace the whole text; the cursor goes to the end.
    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
    }

    pub fn move_cursor(&mut self, movement: crate::command::LineMove) {
        use crate::command::LineMove;
        let at = self.cursor();
        self.cursor = match movement {
            LineMove::Left => prev(&self.text, at),
            LineMove::Right => next(&self.text, at),
            LineMove::WordLeft => word_left(&self.text, at),
            LineMove::WordRight => word_right(&self.text, at),
            LineMove::Home => 0,
            LineMove::End => self.text.len(),
        };
    }

    /// Insert at the cursor, keeping the whole text within `limit` characters.
    /// Returns true when the text changed.
    pub fn insert(&mut self, text: &str, limit: usize) -> bool {
        let room = limit.saturating_sub(self.text.chars().count());
        let piece: String = text.chars().take(room).collect();
        if piece.is_empty() {
            return false;
        }
        let at = self.cursor();
        self.text.insert_str(at, &piece);
        self.cursor = at + piece.len();
        true
    }

    fn delete_to(&mut self, to: usize) -> bool {
        let at = self.cursor();
        let (start, end) = if to < at { (to, at) } else { (at, to) };
        if start == end {
            return false;
        }
        self.text.replace_range(start..end, "");
        self.cursor = start;
        true
    }

    pub fn backspace(&mut self) -> bool {
        self.delete_to(prev(&self.text, self.cursor()))
    }

    pub fn delete_forward(&mut self) -> bool {
        self.delete_to(next(&self.text, self.cursor()))
    }

    pub fn delete_word_back(&mut self) -> bool {
        self.delete_to(word_left(&self.text, self.cursor()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_input_edits_at_the_cursor_by_grapheme() {
        use crate::command::LineMove;
        let mut input = LineInput::new("alpha beta");
        input.move_cursor(LineMove::WordLeft);
        assert_eq!(&input.text()[input.cursor()..], "beta");
        assert!(input.delete_word_back());
        assert_eq!(input.text(), "beta");
        assert_eq!(input.cursor(), 0);
        input.set("aกิน");
        input.move_cursor(LineMove::Left);
        assert!(input.backspace());
        assert_eq!(input.text(), "aน");
        input.move_cursor(LineMove::Home);
        assert!(input.delete_forward());
        assert_eq!(input.text(), "น");
        assert!(input.insert("abc", 3));
        assert_eq!(input.text(), "abน");
        assert!(!input.insert("z", 3), "the limit counts characters");
        input.move_cursor(LineMove::End);
        assert_eq!(input.cursor(), input.text().len());
    }

    #[test]
    fn steps_keep_thai_marks_with_their_letter() {
        // "กิน" is ก + sara i (combining) + น: two graphemes.
        let text = "aกิน";
        let after_a = 1;
        let after_ki = after_a + "กิ".len();
        assert_eq!(next(text, 0), after_a);
        assert_eq!(next(text, after_a), after_ki);
        assert_eq!(prev(text, after_ki), after_a);
        assert_eq!(prev(text, text.len()), after_ki);
        assert_eq!(prev(text, 0), 0);
        assert_eq!(next(text, text.len()), text.len());
        assert_eq!(
            snap(text, after_a + 3),
            after_a,
            "inside กิ snaps to its start"
        );
        assert_eq!(snap(text, text.len()), text.len());
        assert_eq!(snap(text, 999), text.len());
    }

    #[test]
    fn word_jumps_match_the_editor() {
        let text = "rename src/index.html";
        assert_eq!(word_left(text, text.len()), "rename src/index.".len());
        assert_eq!(
            word_left(text, "rename src/index.".len()),
            "rename src/index".len()
        );
        assert_eq!(
            word_left(text, "rename src/index".len()),
            "rename src/".len()
        );
        assert_eq!(word_left(text, "rename ".len()), 0);
        assert_eq!(word_right(text, 0), "rename".len());
        assert_eq!(word_right(text, "rename".len()), "rename src".len());
        assert_eq!(word_right(text, "rename src".len()), "rename src/".len());
        assert_eq!(word_right(text, text.len()), text.len());
        assert_eq!(word_left("", 0), 0);
        assert_eq!(word_right("", 0), 0);
        assert_eq!(word_left("ชื่อ ไฟล์", "ชื่อ ไฟล์".len()), "ชื่อ ".len());
    }

    #[test]
    fn stem_end_skips_dot_files() {
        assert_eq!(stem_end("index.html"), Some(5));
        assert_eq!(stem_end("archive.tar.gz"), Some(11));
        assert_eq!(stem_end("Makefile"), None);
        assert_eq!(stem_end(".gitignore"), None);
    }
}
