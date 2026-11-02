//! Line and snippet edits shared by an open buffer and a file on disk.

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
    for (index, line) in text.split_inclusive('\n').enumerate() {
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
    if find.is_empty() {
        return Err("find is empty".to_string());
    }
    let hay = buffer.rope().to_string();
    let Some(at) = hay.find(find) else {
        return Err("Text was not found".to_string());
    };
    if hay[at + find.len()..].contains(find) {
        return Err("Text appears more than once".to_string());
    }
    let start = hay[..at].chars().count();
    let end = start + find.chars().count();
    buffer.replace_span(start, end, replace);
    Ok(())
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
}
