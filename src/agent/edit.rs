//! Line and snippet edits shared by an open buffer and a file on disk.

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
    },
}

/// Applies `edit` as one undo step. The string says which lines changed.
pub fn apply_to_buffer(buffer: &mut Buffer, edit: &FileEdit) -> Result<String, String> {
    if buffer.rope().len_bytes() > MAX_BYTES {
        return Err("File is too large".to_string());
    }
    match edit {
        FileEdit::Lines { start, end, text } => {
            replace_lines(buffer, *start, *end, text)?;
            Ok(format!("Replaced lines {start}-{end}"))
        }
        FileEdit::Snippet { find, replace } => {
            replace_snippet(buffer, find, replace)?;
            Ok("Replaced one snippet".to_string())
        }
    }
}

pub fn apply_str(original: &str, edit: &FileEdit) -> Result<String, String> {
    if original.len() > MAX_BYTES {
        return Err("File is too large".to_string());
    }
    let mut buffer = Buffer::from_text(original);
    apply_to_buffer(&mut buffer, edit)?;
    Ok(buffer.rope().to_string())
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

fn replace_lines(buffer: &mut Buffer, start: usize, end: usize, text: &str) -> Result<(), String> {
    if start == 0 || end < start {
        return Err("Line range is invalid".to_string());
    }
    let lines = buffer.len_lines().max(1);
    if start > lines {
        return Err("Start line is past the end".to_string());
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
    buffer.replace_span(from, to, &inserted);
    Ok(())
}

fn replace_snippet(buffer: &mut Buffer, find: &str, replace: &str) -> Result<(), String> {
    let (start, end) = find_unique_span(buffer.rope(), find)?;
    buffer.replace_span(start, end, replace);
    Ok(())
}

/// Char span of a unique `find` inside `rope`, without copying the whole buffer.
fn find_unique_span(rope: &Rope, find: &str) -> Result<(usize, usize), String> {
    if find.is_empty() {
        return Err("find is empty".to_string());
    }
    let overlap = find.len().saturating_sub(1);
    let mut first: Option<usize> = None;
    let mut prefix = String::new();
    let mut chars_before = 0usize;
    for chunk in rope.chunks() {
        let mut hay = std::mem::take(&mut prefix);
        hay.push_str(chunk);
        let mut from = 0usize;
        while let Some(rel) = hay[from..].find(find) {
            let at_byte = from + rel;
            let at_char = chars_before + hay[..at_byte].chars().count();
            match first {
                Some(prev) if prev == at_char => {}
                Some(_) => return Err("Text appears more than once".to_string()),
                None => first = Some(at_char),
            }
            from = at_byte + find.len();
        }
        if hay.len() <= overlap {
            prefix = hay;
        } else {
            let cut = hay.len() - overlap;
            chars_before += hay[..cut].chars().count();
            prefix = hay[cut..].to_string();
        }
    }
    let start = first.ok_or_else(|| "Text was not found".to_string())?;
    Ok((start, start + find.chars().count()))
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
            },
        )
        .unwrap();
        assert_eq!(buffer.rope().to_string(), "alpha BETA gamma\n");
        let err = apply_to_buffer(
            &mut buffer,
            &FileEdit::Snippet {
                find: "a".into(),
                replace: "x".into(),
            },
        )
        .unwrap_err();
        assert!(err.contains("more than once"), "{err}");
    }
}
