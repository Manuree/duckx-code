//! Line and snippet edits on an editor buffer, and line reads for the assistant.

use ropey::Rope;

use crate::buffer::Buffer;

const MAX_BYTES: usize = 2_000_000;
const MAX_READ_LINES: usize = 200;
const MAX_READ_CHARS: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEdit {
    Lines {
        start: usize,
        end: usize,
        text: String,
    },
    Snippet {
        find: String,
        replace: String,
        /// Replace every match instead of refusing when there is more than one.
        all: bool,
    },
}

/// What an edit changed. Lines are 1-based and count in the text after the edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOutcome {
    pub summary: String,
    pub first_line: usize,
    pub last_line: usize,
}

/// Applies `edit` as one undo step. The user's cursor and selection stay where they
/// were (moved along with the text), so someone typing elsewhere keeps their place.
pub fn apply_to_buffer(buffer: &mut Buffer, edit: &FileEdit) -> Result<EditOutcome, String> {
    if buffer.rope().len_bytes() > MAX_BYTES {
        return Err("File is too large".to_string());
    }
    match edit {
        FileEdit::Lines { start, end, text } => replace_lines(buffer, *start, *end, text),
        FileEdit::Snippet { find, replace, all } => replace_snippet(buffer, find, replace, *all),
    }
}

/// 1-based inclusive lines, each prefixed with its number. At most 200 lines.
pub fn read_lines(text: &str, start: usize, end: usize) -> Result<String, String> {
    read_line_iter(text.split_inclusive('\n').enumerate(), start, end)
}

/// Same as [`read_lines`], reading only the requested rope lines (no full-buffer copy).
pub fn read_lines_rope(rope: &Rope, start: usize, end: usize) -> Result<String, String> {
    let total = content_line_count(rope);
    let start = start.max(1);
    let end = if end == 0 {
        start + MAX_READ_LINES - 1
    } else {
        end
    };
    if end < start {
        return Err("Line range is invalid".to_string());
    }
    let end = end.min(start + MAX_READ_LINES - 1);
    let mut out = String::new();
    let mut seen = false;
    for number in start..=end {
        let index = number - 1;
        if index >= total {
            break;
        }
        seen = true;
        if out.chars().count() > MAX_READ_CHARS {
            out.push_str("…\n");
            break;
        }
        let line = rope.line(index).to_string();
        out.push_str(&format!("{number}|{line}"));
        if !line.ends_with('\n') {
            out.push('\n');
        }
    }
    if !seen {
        return Err("No lines in that range".to_string());
    }
    Ok(out)
}

fn read_line_iter<'a, I>(lines: I, start: usize, end: usize) -> Result<String, String>
where
    I: Iterator<Item = (usize, &'a str)>,
{
    let start = start.max(1);
    let end = if end == 0 {
        start + MAX_READ_LINES - 1
    } else {
        end
    };
    if end < start {
        return Err("Line range is invalid".to_string());
    }
    let end = end.min(start + MAX_READ_LINES - 1);
    let mut out = String::new();
    let mut seen = false;
    for (index, line) in lines {
        let number = index + 1;
        if number < start {
            continue;
        }
        if number > end {
            break;
        }
        seen = true;
        if out.chars().count() > MAX_READ_CHARS {
            out.push_str("…\n");
            break;
        }
        out.push_str(&format!("{number}|{line}"));
        if !line.ends_with('\n') {
            out.push('\n');
        }
    }
    if !seen {
        return Err("No lines in that range".to_string());
    }
    Ok(out)
}

/// Ropey counts a trailing empty line after a final newline; readers should not.
fn content_line_count(rope: &Rope) -> usize {
    let lines = rope.len_lines();
    if lines > 0 && rope.line(lines - 1).len_chars() == 0 {
        lines - 1
    } else {
        lines
    }
}

fn replace_lines(
    buffer: &mut Buffer,
    start: usize,
    end: usize,
    text: &str,
) -> Result<EditOutcome, String> {
    if start == 0 || end < start {
        return Err("Line range is invalid".to_string());
    }
    let lines = buffer.len_lines().max(1);
    if start > lines {
        return Err(format!(
            "Start line is past the end (the file has {} lines)",
            content_line_count(buffer.rope())
        ));
    }
    let end = end.min(lines);
    let from = buffer.rope().line_to_char(start - 1);
    let to = if end >= buffer.len_lines() {
        buffer.rope().len_chars()
    } else {
        buffer.rope().line_to_char(end)
    };
    let mut inserted = text.to_string();
    if !inserted.is_empty() && !inserted.ends_with('\n') {
        inserted.push('\n');
    }
    if uses_crlf(buffer.rope()) {
        inserted = to_crlf(&inserted);
    }
    buffer.replace_spans_in_place(&[(from, to)], &inserted);
    let added = inserted.lines().count().max(1);
    Ok(EditOutcome {
        summary: format!("Replaced lines {start}-{end}"),
        first_line: start,
        last_line: start + added - 1,
    })
}

fn replace_snippet(
    buffer: &mut Buffer,
    find: &str,
    replace: &str,
    all: bool,
) -> Result<EditOutcome, String> {
    if find.is_empty() {
        return Err("find is empty".to_string());
    }
    let (spans, replace) = locate(buffer.rope(), find, replace);
    if spans.is_empty() {
        return Err(
            "Text was not found. Read those lines again and copy them exactly, without the line numbers."
                .to_string(),
        );
    }
    if spans.len() > 1 && !all {
        return Err(format!(
            "Text appears {} times. Add lines around it so it is unique, or set replace_all to true.",
            spans.len()
        ));
    }
    let count = spans.len();
    let replace_chars = replace.chars().count();
    buffer.replace_spans_in_place(&spans, &replace);
    let rope = buffer.rope();
    let shifted = |index: usize| -> usize {
        let (start, _) = spans[index];
        let removed: usize = spans[..index].iter().map(|(a, b)| b - a).sum();
        start - removed + index * replace_chars
    };
    let first = shifted(0);
    let last = shifted(count - 1) + replace_chars;
    let len = rope.len_chars();
    let summary = if count == 1 {
        "Replaced one snippet".to_string()
    } else {
        format!("Replaced {count} matches")
    };
    Ok(EditOutcome {
        summary,
        first_line: rope.char_to_line(first.min(len)) + 1,
        last_line: rope.char_to_line(last.min(len)) + 1,
    })
}

/// Spans of `find`, and the replacement to use with them. When the exact text is
/// missing, retries what models get wrong most: `\n` against a CRLF file, curly
/// quotes typed as straight ones, and `12|` prefixes copied from `read_file`.
fn locate(rope: &Rope, find: &str, replace: &str) -> (Vec<(usize, usize)>, String) {
    let exact = find_all_spans(rope, find);
    if !exact.is_empty() {
        return (exact, replace.to_string());
    }
    let crlf = uses_crlf(rope);
    let mut candidates: Vec<(String, String)> = Vec::new();
    let stripped = strip_line_numbers(find);
    if let Some(stripped) = &stripped {
        candidates.push((stripped.clone(), replace.to_string()));
    }
    if crlf {
        candidates.push((to_crlf(find), to_crlf(replace)));
        if let Some(stripped) = &stripped {
            candidates.push((to_crlf(stripped), to_crlf(replace)));
        }
    }
    for (find, replace) in &candidates {
        let spans = find_all_spans(rope, find);
        if !spans.is_empty() {
            return (spans, replace.clone());
        }
    }
    // Quote-insensitive pass. Each curly quote maps to one straight char, so char
    // indices in the normalized text are char indices in the buffer.
    let text = normalize_quotes(&rope.to_string());
    candidates.insert(0, (find.to_string(), replace.to_string()));
    for (find, replace) in &candidates {
        let needle = normalize_quotes(find);
        let spans = find_all_in_str(&text, &needle);
        if !spans.is_empty() {
            return (spans, replace.clone());
        }
    }
    (Vec::new(), replace.to_string())
}

fn normalize_quotes(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            other => other,
        })
        .collect()
}

/// `12|text` on every non-empty line, as `read_file` prints it. `None` otherwise.
fn strip_line_numbers(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut any = false;
    for line in text.split_inclusive('\n') {
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && line[digits..].starts_with('|') {
            out.push_str(&line[digits + 1..]);
            any = true;
        } else if line.trim().is_empty() {
            out.push_str(line);
        } else {
            return None;
        }
    }
    any.then_some(out)
}

fn uses_crlf(rope: &Rope) -> bool {
    rope.lines()
        .take(50)
        .any(|line| line.len_chars() >= 2 && line.char(line.len_chars() - 2) == '\r')
}

fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Char spans of every non-overlapping `find`, without copying the whole buffer.
fn find_all_spans(rope: &Rope, find: &str) -> Vec<(usize, usize)> {
    if find.is_empty() {
        return Vec::new();
    }
    let find_chars = find.chars().count();
    let overlap = find.len().saturating_sub(1);
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut prefix = String::new();
    let mut chars_before = 0usize;
    for chunk in rope.chunks() {
        let mut hay = std::mem::take(&mut prefix);
        hay.push_str(chunk);
        let mut from = 0usize;
        while let Some(rel) = hay[from..].find(find) {
            let at_byte = from + rel;
            let at_char = chars_before + hay[..at_byte].chars().count();
            if spans.last().is_none_or(|(_, end)| at_char >= *end) {
                spans.push((at_char, at_char + find_chars));
            }
            from = at_byte + find.len();
        }
        if hay.len() <= overlap {
            prefix = hay;
        } else {
            // Overlap is a byte length; ceil so we never slice mid-character
            // (em dash, Thai, emoji) when carrying the window to the next chunk.
            let cut = crate::utf8::ceil_char_boundary(&hay, hay.len() - overlap);
            chars_before += hay[..cut].chars().count();
            prefix = hay[cut..].to_string();
        }
    }
    spans
}

fn find_all_in_str(text: &str, find: &str) -> Vec<(usize, usize)> {
    if find.is_empty() {
        return Vec::new();
    }
    let find_chars = find.chars().count();
    let mut spans = Vec::new();
    let mut from = 0usize;
    let mut chars_before = 0usize;
    let mut counted_to = 0usize;
    while let Some(rel) = text[from..].find(find) {
        let at_byte = from + rel;
        chars_before += text[counted_to..at_byte].chars().count();
        counted_to = at_byte;
        spans.push((chars_before, chars_before + find_chars));
        from = at_byte + find.len();
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_edit_is_one_undo_step() {
        let mut buffer = Buffer::from_text("one\ntwo\nthree\n");
        apply_to_buffer(
            &mut buffer,
            &FileEdit::Lines {
                start: 2,
                end: 2,
                text: "TWO".into(),
            },
        )
        .unwrap();
        assert_eq!(buffer.rope().to_string(), "one\nTWO\nthree\n");
        buffer.undo();
        assert_eq!(buffer.rope().to_string(), "one\ntwo\nthree\n");
        buffer.redo();
        assert_eq!(buffer.rope().to_string(), "one\nTWO\nthree\n");
    }

    #[test]
    fn rope_line_reads_match_string_reads() {
        let text = "one\ntwo\nthree\n";
        let rope = Rope::from_str(text);
        let from_str = read_lines(text, 2, 3).unwrap();
        let from_rope = read_lines_rope(&rope, 2, 3).unwrap();
        assert_eq!(from_str, from_rope);
        assert!(from_rope.contains("2|two"));
        assert!(from_rope.contains("3|three"));
    }

    #[test]
    fn snippet_edit_finds_across_chunk_boundaries() {
        let mut buffer = Buffer::from_text("alpha beta gamma\n");
        apply_to_buffer(
            &mut buffer,
            &FileEdit::Snippet {
                find: "beta".into(),
                replace: "BETA".into(),
                all: false,
            },
        )
        .unwrap();
        assert_eq!(buffer.rope().to_string(), "alpha BETA gamma\n");
        let err = apply_to_buffer(
            &mut buffer,
            &FileEdit::Snippet {
                find: "a".into(),
                replace: "x".into(),
                all: false,
            },
        )
        .unwrap_err();
        assert!(err.contains("2 times") || err.contains("times"), "{err}");
    }

    /// Regression: overlap cut used to land inside '—' and panic (Windows / LM Studio).
    #[test]
    fn snippet_edit_survives_em_dash_overlap_cut() {
        let find = "UNIQUE_FIND_TOKEN";
        let overlap = find.len() - 1;
        let mut text = String::new();
        text.push_str(find);
        text.push_str(&"x".repeat(80));
        text.push('—'); // 3-byte em dash; cut = len - overlap lands inside it
        text.push_str(&"y".repeat(overlap - 1));
        text.push('\n');
        let mut buffer = Buffer::from_text(&text);
        apply_to_buffer(
            &mut buffer,
            &FileEdit::Snippet {
                find: find.into(),
                replace: "REPLACED".into(),
                all: false,
            },
        )
        .unwrap();
        assert!(buffer.rope().to_string().starts_with("REPLACED"));
        assert!(buffer.rope().to_string().contains('—'));
    }

    #[test]
    fn snippet_edit_survives_thai_and_emoji_near_overlap() {
        for marker in ['ส', '👋', '—'] {
            let find = "NEEDLE_TOKEN_XX";
            let overlap = find.len() - 1;
            let mut text = String::new();
            text.push_str(find);
            text.push_str(&"a".repeat(40));
            text.push(marker);
            text.push_str(&"b".repeat(overlap - 1));
            text.push('\n');
            let mut buffer = Buffer::from_text(&text);
            apply_to_buffer(
                &mut buffer,
                &FileEdit::Snippet {
                    find: find.into(),
                    replace: "OK".into(),
                    all: false,
                },
            )
            .expect("multibyte overlap cut must not panic");
            assert!(buffer.rope().to_string().starts_with("OK"));
        }
    }

    #[test]
    fn snippet_edit_html_comment_with_em_dashes() {
        let text = "<!-- — Tailwind CSS via CDN —>\n<div>hi</div>\n";
        let mut buffer = Buffer::from_text(text);
        apply_to_buffer(
            &mut buffer,
            &FileEdit::Snippet {
                find: "Tailwind CSS via CDN".into(),
                replace: "UnoCSS".into(),
                all: false,
            },
        )
        .unwrap();
        assert!(buffer.rope().to_string().contains("UnoCSS"));
        assert!(buffer.rope().to_string().contains('—'));
    }

    fn snippet(find: &str, replace: &str, all: bool) -> FileEdit {
        FileEdit::Snippet {
            find: find.into(),
            replace: replace.into(),
            all,
        }
    }

    #[test]
    fn an_edit_elsewhere_keeps_the_users_cursor_on_its_text() {
        let mut buffer = Buffer::from_text("one\ntwo\nthree\n");
        buffer.place_cursor(2, 3);
        let before = buffer.rope().line(2).to_string();
        apply_to_buffer(
            &mut buffer,
            &FileEdit::Lines {
                start: 1,
                end: 1,
                text: "uno\nein".into(),
            },
        )
        .unwrap();
        let (line, col) = buffer.cursor_line_col();
        assert_eq!((line, col), (3, 3), "cursor moved with its line");
        assert_eq!(buffer.rope().line(3).to_string(), before);
    }

    #[test]
    fn replace_all_changes_every_match_in_one_undo_step() {
        let mut buffer = Buffer::from_text("let foo = foo + foo;\n");
        let outcome = apply_to_buffer(&mut buffer, &snippet("foo", "bar", true)).unwrap();
        assert_eq!(buffer.rope().to_string(), "let bar = bar + bar;\n");
        assert_eq!(outcome.summary, "Replaced 3 matches");
        buffer.undo();
        assert_eq!(buffer.rope().to_string(), "let foo = foo + foo;\n");
        let err = apply_to_buffer(&mut buffer, &snippet("foo", "bar", false)).unwrap_err();
        assert!(err.contains("3 times"), "{err}");
    }

    #[test]
    fn a_snippet_typed_with_lf_matches_a_crlf_file() {
        let mut buffer = Buffer::from_text("fn a() {\r\n    1\r\n}\r\n");
        apply_to_buffer(&mut buffer, &snippet("{\n    1\n}", "{\n    2\n}", false)).unwrap();
        assert_eq!(buffer.rope().to_string(), "fn a() {\r\n    2\r\n}\r\n");
    }

    #[test]
    fn straight_quotes_find_curly_ones_and_line_numbers_are_ignored() {
        let mut buffer = Buffer::from_text("say “hi” to ‘you’\n");
        apply_to_buffer(&mut buffer, &snippet("\"hi\" to 'you'", "bye", false)).unwrap();
        assert_eq!(buffer.rope().to_string(), "say bye\n");

        let mut buffer = Buffer::from_text("one\ntwo\nthree\n");
        apply_to_buffer(&mut buffer, &snippet("2|two\n3|three", "2\n3", false)).unwrap();
        assert_eq!(buffer.rope().to_string(), "one\n2\n3\n");
    }

    #[test]
    fn the_outcome_names_the_lines_after_the_edit() {
        let mut buffer = Buffer::from_text("a\nb\nc\nd\n");
        let outcome = apply_to_buffer(&mut buffer, &snippet("c", "x\ny", false)).unwrap();
        assert_eq!((outcome.first_line, outcome.last_line), (3, 4));
        let outcome = apply_to_buffer(
            &mut buffer,
            &FileEdit::Lines {
                start: 1,
                end: 1,
                text: "1\n2".into(),
            },
        )
        .unwrap();
        assert_eq!((outcome.first_line, outcome.last_line), (1, 2));
    }
}
